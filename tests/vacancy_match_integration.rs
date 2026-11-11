//! Run only against a disposable migrated test database. Never reads .env.
use careerbridge_be::infrastructure::{db, error::AppError, jobs};
use serde_json::json;
use sqlx::PgPool;
use std::collections::HashSet;
use uuid::Uuid;

struct Fixture {
    a: Uuid,
    b: Uuid,
    user: Uuid,
    v_both: Uuid,
    v_a: Uuid,
    v_none: Uuid,
    prefix: String,
}

async fn pools() -> (PgPool, PgPool, String) {
    let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL required");
    let owner = PgPool::connect(&url).await.unwrap();
    let name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&owner)
        .await
        .unwrap();
    assert!(name.contains("_test"), "disposable test database required");
    let importer = db::connect_runtime_pool(&url, db::RuntimeRole::Importer)
        .await
        .unwrap();
    (owner, importer, url)
}

async fn fixture(owner: &PgPool) -> Fixture {
    let (a, b, user) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let alias_a = format!("fixa{}", Uuid::new_v4().simple());
    let alias_b = format!("fixb{}", Uuid::new_v4().simple());
    for (id, nama, alias) in [
        (a, format!("Fixture A {}", Uuid::new_v4()), &alias_a),
        (b, format!("Fixture B {}", Uuid::new_v4()), &alias_b),
    ] {
        sqlx::query("INSERT INTO skills(id,nama,kategori) VALUES($1,$2,'Test')")
            .bind(id)
            .bind(nama)
            .execute(owner)
            .await
            .unwrap();
        sqlx::query("INSERT INTO skill_aliases(alias,skill_id) VALUES($1,$2)")
            .bind(alias)
            .bind(id)
            .execute(owner)
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO auth.users(id) VALUES($1)")
        .bind(user)
        .execute(owner)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users(id,role_id,nama,pendidikan,jurusan,province_id,kota,persetujuan_cv_at) VALUES($1,1,'Match fixture','S1','TI',1,'Test',now())")
        .bind(user).execute(owner).await.unwrap();
    sqlx::query("INSERT INTO user_skills(user_id,skill_id,sumber) VALUES($1,$2,'manual')")
        .bind(user)
        .bind(a)
        .execute(owner)
        .await
        .unwrap();
    let prefix = format!("Match fixture {}", Uuid::new_v4());
    let (v_both, v_a, v_none) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    // Default order is posted_at DESC: v_none (newest), v_both, v_a (oldest).
    for (id, days, description) in [
        (v_none, 1, "Tanpa skill katalog".to_owned()),
        (v_both, 2, format!("Wajib {alias_a} dan {alias_b}")),
        (v_a, 3, format!("Menguasai {alias_a}")),
    ] {
        sqlx::query("INSERT INTO job_vacancies(id,source,source_id,company,title,country,city,description,apply_url,source_url,posted_at,last_seen_run) VALUES($1,'grab',$2,'Fixture',$3,'ID','Jakarta',$4,'https://example.test/apply','https://example.test/source',now()-make_interval(days=>$5),$6)")
            .bind(id).bind(id.to_string()).bind(format!("{prefix} {id}")).bind(description).bind(days).bind(Uuid::new_v4())
            .execute(owner).await.unwrap();
    }
    Fixture {
        a,
        b,
        user,
        v_both,
        v_a,
        v_none,
        prefix,
    }
}

async fn cleanup(owner: &PgPool, f: &Fixture) {
    sqlx::query("DELETE FROM job_vacancies WHERE id=ANY($1)")
        .bind(vec![f.v_both, f.v_a, f.v_none])
        .execute(owner)
        .await
        .unwrap();
    sqlx::query("DELETE FROM auth.users WHERE id=$1")
        .bind(f.user)
        .execute(owner)
        .await
        .unwrap();
    sqlx::query("DELETE FROM skills WHERE id=ANY($1)")
        .bind(vec![f.a, f.b])
        .execute(owner)
        .await
        .unwrap();
}

async fn tags(owner: &PgPool, vacancy: Uuid) -> HashSet<Uuid> {
    sqlx::query_scalar("SELECT skill_id FROM vacancy_skills WHERE vacancy_id=$1")
        .bind(vacancy)
        .fetch_all(owner)
        .await
        .unwrap()
        .into_iter()
        .collect()
}

#[tokio::test]
#[ignore = "requires a disposable migrated TEST_DATABASE_URL"]
async fn retag_replaces_vacancy_skills_and_survives_skill_deletion() {
    let (owner, importer, _) = pools().await;
    let f = fixture(&owner).await;
    assert!(jobs::retag_all(&importer).await.unwrap() >= 3);
    assert_eq!(tags(&owner, f.v_both).await, HashSet::from([f.a, f.b]));
    assert_eq!(tags(&owner, f.v_a).await, HashSet::from([f.a]));
    assert!(tags(&owner, f.v_none).await.is_empty());
    // Deleting a tagged skill (b is not in user_skills) is not blocked by vacancy_skills.
    sqlx::query("DELETE FROM skills WHERE id=$1")
        .bind(f.b)
        .execute(&owner)
        .await
        .unwrap();
    assert_eq!(tags(&owner, f.v_both).await, HashSet::from([f.a]));
    // Alias removed → stale tag disappears on retag (replace, not append).
    sqlx::query("DELETE FROM skill_aliases WHERE skill_id=$1")
        .bind(f.a)
        .execute(&owner)
        .await
        .unwrap();
    jobs::retag_all(&importer).await.unwrap();
    assert!(tags(&owner, f.v_both).await.is_empty());
    assert!(tags(&owner, f.v_a).await.is_empty());
    cleanup(&owner, &f).await;
}

#[tokio::test]
#[ignore = "requires a disposable migrated TEST_DATABASE_URL"]
async fn vacancy_page_scores_and_sorts_by_user_skill_match() {
    let (owner, importer, url) = pools().await;
    let api = db::connect_runtime_pool(&url, db::RuntimeRole::Api)
        .await
        .unwrap();
    let f = fixture(&owner).await;
    jobs::retag_all(&importer).await.unwrap();
    let mut q = jobs::VacancyQuery {
        offset: 0,
        limit: 10,
        q: f.prefix.clone(),
        city: String::new(),
        source: String::new(),
        occupation: String::new(),
        sort: "match".into(),
    };
    let page = jobs::page(&api, Uuid::nil(), &[f.a], &q).await.unwrap();
    let items = page["items"].as_array().unwrap();
    let ids: Vec<String> = items
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_owned())
        .collect();
    // Default order (v_none, v_both, v_a) is reversed, so this only passes when sort=match applies.
    assert_eq!(
        ids,
        [
            f.v_a.to_string(),
            f.v_both.to_string(),
            f.v_none.to_string()
        ]
    );
    assert_eq!(items[0]["matchScore"], 100);
    assert_eq!(items[1]["matchScore"], 50);
    assert_eq!(items[1]["ownedSkillIds"], json!([f.a]));
    assert_eq!(items[1]["missingSkillIds"], json!([f.b]));
    assert!(items[2]["matchScore"].is_null());
    assert_eq!(items[2]["ownedSkillIds"], json!([]));
    let labels: HashSet<String> = page["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(labels, HashSet::from([f.a.to_string(), f.b.to_string()]));
    // No user skills: tagged vacancies score 0, untagged stay null.
    let none = jobs::page(&api, Uuid::nil(), &[], &q).await.unwrap();
    assert_eq!(none["items"][0]["matchScore"], 0);
    q.sort = "popular".into();
    assert!(matches!(
        jobs::page(&api, Uuid::nil(), &[f.a], &q).await,
        Err(AppError::Invalid(_))
    ));
    cleanup(&owner, &f).await;
}
