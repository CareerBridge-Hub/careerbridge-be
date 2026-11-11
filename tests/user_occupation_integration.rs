//! Run only against a disposable migrated test database. Never reads .env.
use careerbridge_be::{
    domain::OccupationSuggestion,
    infrastructure::{
        browsing::{self, Browse},
        db,
        error::AppError,
        jobs::{self, VacancyQuery},
    },
};
use sqlx::PgPool;
use uuid::Uuid;

struct Fixture {
    a: Uuid,
    b: Uuid,
    catalog: Uuid,
    catalog_name: String,
    owner_user: Uuid,
    other_user: Uuid,
    vacancy: Uuid,
}

async fn fixture(owner: &PgPool) -> Fixture {
    let (a, b, catalog) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    for id in [a, b] {
        sqlx::query("INSERT INTO skills(id,nama,kategori) VALUES($1,$2,'Test')")
            .bind(id)
            .bind(format!("Occupation fixture skill {id}"))
            .execute(owner)
            .await
            .unwrap();
    }
    let catalog_name = format!("Catalog fixture {catalog}");
    let mut tx = owner.begin().await.unwrap();
    sqlx::query("INSERT INTO occupations(id,nama,kode_kbji,bidang,deskripsi) VALUES($1,$2,'2523','Fixture','Fixture')")
        .bind(catalog).bind(&catalog_name).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO occupation_skills(occupation_id,skill_id,tipe) VALUES($1,$2,'wajib')")
        .bind(catalog)
        .bind(a)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let (owner_user, other_user) = (Uuid::new_v4(), Uuid::new_v4());
    for user in [owner_user, other_user] {
        sqlx::query("INSERT INTO auth.users(id) VALUES($1)")
            .bind(user)
            .execute(owner)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users(id,role_id,nama,pendidikan,jurusan,province_id,kota,persetujuan_cv_at) VALUES($1,1,'Occupation fixture','S1','TI',1,'Test',now())")
            .bind(user).execute(owner).await.unwrap();
    }
    let vacancy = Uuid::new_v4();
    sqlx::query("INSERT INTO job_vacancies(id,source,source_id,company,title,country,city,description,apply_url,source_url,last_seen_run,occupation_id) VALUES($1,'grab',$2,'Fixture',$3,'ID','Jakarta','Fixture','https://example.test/apply','https://example.test/source',$4,$5)")
        .bind(vacancy).bind(vacancy.to_string()).bind(format!("Fixture vacancy {vacancy}")).bind(Uuid::new_v4()).bind(catalog)
        .execute(owner).await.unwrap();
    Fixture {
        a,
        b,
        catalog,
        catalog_name,
        owner_user,
        other_user,
        vacancy,
    }
}

async fn cleanup(owner: &PgPool, f: &Fixture) {
    sqlx::query("DELETE FROM job_vacancies WHERE id=$1")
        .bind(f.vacancy)
        .execute(owner)
        .await
        .unwrap();
    // Deleting the users cascades to their own occupations.
    sqlx::query("DELETE FROM auth.users WHERE id=ANY($1)")
        .bind(vec![f.owner_user, f.other_user])
        .execute(owner)
        .await
        .unwrap();
    sqlx::query("DELETE FROM occupations WHERE id=$1")
        .bind(f.catalog)
        .execute(owner)
        .await
        .unwrap();
    sqlx::query("DELETE FROM skills WHERE id=ANY($1)")
        .bind(vec![f.a, f.b])
        .execute(owner)
        .await
        .unwrap();
}

fn custom(
    nama: &str,
    base: Option<Uuid>,
    wajib: Vec<Uuid>,
    opsional: Vec<Uuid>,
) -> OccupationSuggestion {
    OccupationSuggestion::Custom {
        nama: nama.into(),
        bidang: "Teknologi Informasi".into(),
        deskripsi: "Merancang dan mengelola infrastruktur cloud.".into(),
        base_occupation_id: base,
        wajib,
        opsional,
        evidence: vec!["Cloud Engineer".into()],
    }
}

async fn save(api: &PgPool, user: Uuid, suggestion: &OccupationSuggestion) -> Option<Uuid> {
    let mut tx = db::active_tx(api, user).await.unwrap();
    let id = db::save_owned_occupation(&mut tx, user, suggestion)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    id
}

fn vacancies(occupation: Uuid) -> VacancyQuery {
    VacancyQuery {
        offset: 0,
        limit: 100,
        q: "Fixture vacancy".into(),
        city: "".into(),
        source: "".into(),
        occupation: occupation.to_string(),
        sort: "".into(),
    }
}

#[tokio::test]
#[ignore = "requires a disposable migrated TEST_DATABASE_URL"]
async fn own_occupation_is_private_scored_and_mapped_to_catalog_vacancies() {
    let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL required");
    let owner = PgPool::connect(&url).await.unwrap();
    let name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&owner)
        .await
        .unwrap();
    assert!(name.contains("_test"), "disposable test database required");
    let api = db::connect_runtime_pool(&url, db::RuntimeRole::Api)
        .await
        .unwrap();
    let worker = db::connect_runtime_pool(&url, db::RuntimeRole::Worker)
        .await
        .unwrap();
    let importer = db::connect_runtime_pool(&url, db::RuntimeRole::Importer)
        .await
        .unwrap();
    let f = fixture(&owner).await;
    let nama = format!("Cloud Fixture {}", Uuid::new_v4().simple());

    let id = save(
        &api,
        f.owner_user,
        &custom(&nama, Some(f.catalog), vec![f.a, f.b], vec![]),
    )
    .await
    .expect("owned occupation saved");
    // A newer CV replaces the occupation in place, so a target pointing at it survives.
    let renamed = format!("{nama} v2");
    let again = save(
        &api,
        f.owner_user,
        &custom(&renamed, Some(f.catalog), vec![f.b], vec![f.a]),
    )
    .await;
    assert_eq!(again, Some(id));
    let (stored, kbji, base): (String, Option<String>, Option<Uuid>) =
        sqlx::query_as("SELECT nama,kode_kbji,base_occupation_id FROM occupations WHERE id=$1")
            .bind(id)
            .fetch_one(&owner)
            .await
            .unwrap();
    assert_eq!(
        (stored.as_str(), kbji, base),
        (renamed.as_str(), None, Some(f.catalog))
    );
    let tipe: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT skill_id,tipe FROM occupation_skills WHERE occupation_id=$1 ORDER BY tipe DESC",
    )
    .bind(id)
    .fetch_all(&owner)
    .await
    .unwrap();
    assert_eq!(tipe, vec![(f.b, "wajib".into()), (f.a, "opsional".into())]);
    // Owned occupations never enter the metered job-search queue.
    let queued: i64 =
        sqlx::query_scalar("SELECT count(*) FROM job_search_queries WHERE occupation_id=$1")
            .bind(id)
            .fetch_one(&owner)
            .await
            .unwrap();
    assert_eq!(queued, 0);

    // The owner sees, scores and analyses it like a catalog occupation.
    sqlx::query("INSERT INTO user_skills(user_id,skill_id,sumber) VALUES($1,$2,'manual')")
        .bind(f.owner_user)
        .bind(f.b)
        .execute(&owner)
        .await
        .unwrap();
    let m = browsing::job(&api, f.owner_user, id).await.unwrap();
    assert!(m.job.custom);
    assert_eq!(m.score, 67, "owned required (2) of required+optional (3)");
    let page = browsing::matches(
        &api,
        f.owner_user,
        &Browse {
            q: renamed.clone(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page["total"], 1);

    // Nobody else can read, target or edit it: other users, the importer, the worker, or callers without user context.
    assert!(matches!(
        browsing::job(&api, f.other_user, id).await,
        Err(AppError::NotFound)
    ));
    let mut tx = db::active_tx(&api, f.other_user).await.unwrap();
    let edited = sqlx::query("UPDATE occupations SET nama='hijacked' WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .unwrap()
        .rows_affected();
    let skills_seen: i64 =
        sqlx::query_scalar("SELECT count(*) FROM occupation_skills WHERE occupation_id=$1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!((edited, skills_seen), (0, 0));
    for pool in [&api, &worker, &importer] {
        let visible: i64 = sqlx::query_scalar("SELECT count(*) FROM occupations WHERE id=$1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(visible, 0);
    }

    // Vacancies come from the closest catalog occupation, for the owner only.
    let own = jobs::page(&api, f.owner_user, &[], &vacancies(id))
        .await
        .unwrap();
    assert_eq!(own["total"], 1);
    assert!(
        own["occupationOptions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["id"] == id.to_string() && o["custom"] == true)
    );
    let foreign = jobs::page(&api, f.other_user, &[], &vacancies(id))
        .await
        .unwrap();
    assert_eq!(foreign["total"], 0);

    // Without a related catalog occupation, vacancies tagged with a required skill are used.
    save(
        &api,
        f.owner_user,
        &custom(&renamed, None, vec![f.a], vec![]),
    )
    .await
    .unwrap();
    assert_eq!(
        jobs::page(&api, f.owner_user, &[], &vacancies(id))
            .await
            .unwrap()["total"],
        0
    );
    sqlx::query("INSERT INTO vacancy_skills(vacancy_id,skill_id) VALUES($1,$2)")
        .bind(f.vacancy)
        .bind(f.a)
        .execute(&owner)
        .await
        .unwrap();
    assert_eq!(
        jobs::page(&api, f.owner_user, &[], &vacancies(id))
            .await
            .unwrap()["total"],
        1
    );

    // A suggestion whose name an admin has since added to the catalog, or whose skills
    // were all deleted, is not saved; the existing own occupation is left untouched.
    assert_eq!(
        save(
            &api,
            f.owner_user,
            &custom(&f.catalog_name.to_uppercase(), None, vec![f.a], vec![])
        )
        .await,
        None
    );
    assert_eq!(
        save(
            &api,
            f.owner_user,
            &custom(&nama, None, vec![Uuid::new_v4()], vec![])
        )
        .await,
        None
    );
    let still: String = sqlx::query_scalar("SELECT nama FROM occupations WHERE id=$1")
        .bind(id)
        .fetch_one(&owner)
        .await
        .unwrap();
    assert_eq!(still, renamed);

    cleanup(&owner, &f).await;
    let gone: i64 = sqlx::query_scalar("SELECT count(*) FROM occupations WHERE id=$1")
        .bind(id)
        .fetch_one(&owner)
        .await
        .unwrap();
    assert_eq!(gone, 0, "account deletion removes the own occupation");
}
