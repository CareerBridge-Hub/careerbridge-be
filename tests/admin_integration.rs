//! Run only against a disposable migrated test database. Never reads .env.
use careerbridge_be::infrastructure::{admin, db, error::AppError, jobs};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires a disposable migrated TEST_DATABASE_URL"]
async fn admin_crud_rls_atomic_import_and_cron_visibility() {
    let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL required");
    let owner = PgPool::connect(&url).await.unwrap();
    let name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&owner)
        .await
        .unwrap();
    assert!(name.contains("_test"), "disposable test database required");
    let pool = db::connect_runtime_pool(&url, db::RuntimeRole::Api)
        .await
        .unwrap();
    let actor = Uuid::new_v4();
    let ordinary = Uuid::new_v4();
    for (id, role) in [(actor, 2_i16), (ordinary, 1_i16)] {
        sqlx::query("INSERT INTO auth.users(id) VALUES($1)")
            .bind(id)
            .execute(&owner)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users(id,role_id,nama,pendidikan,jurusan,province_id,kota,persetujuan_cv_at) VALUES($1,$2,'Admin fixture','S1','TI',1,'Test',now())")
            .bind(id).bind(role).execute(&owner).await.unwrap();
    }
    assert!(matches!(
        admin::snapshot(&pool, ordinary).await,
        Err(AppError::Forbidden)
    ));
    assert!(matches!(
        admin::save(
            &pool,
            ordinary,
            "skills",
            vec![json!({"nama":"Forbidden","kategori":"Test"})]
        )
        .await,
        Err(AppError::Forbidden)
    ));
    let mut tx = db::user_tx(&pool, ordinary).await.unwrap();
    let denied = sqlx::query(
        "INSERT INTO skills(id,nama,kategori) VALUES($1,'Forbidden direct SQL','Test')",
    )
    .bind(Uuid::new_v4())
    .execute(&mut *tx)
    .await
    .unwrap_err();
    assert_eq!(
        denied.as_database_error().unwrap().code().as_deref(),
        Some("42501")
    );
    tx.rollback().await.unwrap();
    let skill = Uuid::new_v4();
    let other = Uuid::new_v4();
    let prefix = format!("Admin fixture {}", Uuid::new_v4());
    admin::save(&pool,actor,"skills",vec![json!({"id":skill,"nama":format!("{prefix} skill"),"kategori":"Fixture","alias":[format!("{prefix} alias")]}),json!({"id":other,"nama":format!("{prefix} optional"),"kategori":"Fixture"})]).await.unwrap();
    assert!(
        admin::save(
            &pool,
            actor,
            "skills",
            vec![json!({"nama":format!("{prefix} ALIAS"),"kategori":"Fixture"})]
        )
        .await
        .is_err()
    );
    let rollback_id = Uuid::new_v4();
    assert!(
        admin::save(
            &pool,
            actor,
            "skills",
            vec![
                json!({"id":rollback_id,"nama":format!("{prefix} rollback"),"kategori":"Fixture"}),
                json!({"nama":"","kategori":"Fixture"})
            ]
        )
        .await
        .is_err()
    );
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM skills WHERE id=$1)")
        .bind(rollback_id)
        .fetch_one(&owner)
        .await
        .unwrap();
    assert!(!exists, "entire CSV batch must roll back");
    let job = Uuid::new_v4();
    admin::save(&pool,actor,"jobs",vec![json!({"id":job,"nama":format!("{prefix} job"),"kodeKbji":"2512","bidang":"Fixture","deskripsi":"Fixture","skills":[{"skillId":skill,"tipe":"wajib"},{"skillId":other,"tipe":"opsional"}]})]).await.unwrap();
    assert!(admin::save(&pool,actor,"jobs",vec![json!({"nama":"Invalid job","kodeKbji":"2512","bidang":"Fixture","deskripsi":"","skills":[]})]).await.is_err());
    assert!(
        admin::remove(&pool, actor, "skills", skill).await.is_err(),
        "must not delete the final required skill"
    );
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM skills WHERE id=$1)")
            .bind(skill)
            .fetch_one(&owner)
            .await
            .unwrap()
    );
    let institution = Uuid::new_v4();
    let training = Uuid::new_v4();
    let province: String = sqlx::query_scalar("SELECT nama FROM provinces WHERE id=1")
        .fetch_one(&owner)
        .await
        .unwrap();
    admin::save(&pool,actor,"institutions",vec![json!({"id":institution,"nama":format!("{prefix} institution"),"jenis":"LPK","provinsi":province,"kota":"Test","website":"https://example.test"})]).await.unwrap();
    admin::save(&pool,actor,"trainings",vec![json!({"id":training,"institutionId":institution,"nama":format!("{prefix} training"),"biaya":null,"moda":null,"durasi":"","urlDaftar":"https://example.test/course","skillIds":[skill]})]).await.unwrap();
    let snapshot = admin::snapshot(&pool, actor).await.unwrap();
    let program = snapshot["trainings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == training.to_string())
        .unwrap();
    assert!(program["biaya"].is_null());
    assert!(program["moda"].is_null());
    assert!(!snapshot.to_string().contains("persetujuan_cv_at"));
    // Persist a user's reference and an occupation target to exercise cascading cleanup.
    sqlx::query("INSERT INTO user_skills(user_id,skill_id,sumber) VALUES($1,$2,'manual')")
        .bind(ordinary)
        .bind(skill)
        .execute(&owner)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET target_occupation_id=$2 WHERE id=$1")
        .bind(ordinary)
        .bind(job)
        .execute(&owner)
        .await
        .unwrap();
    let vacancy = Uuid::new_v4();
    let title = format!("Software Engineer {vacancy}");
    sqlx::query("INSERT INTO job_vacancies(id,source,source_id,company,title,country,city,description,apply_url,source_url,last_seen_run) VALUES($1,'grab',$2,'Fixture',$3,'ID','Jakarta','Fixture','https://example.test/apply','https://example.test/source',$4)")
        .bind(vacancy).bind(vacancy.to_string()).bind(&title).bind(Uuid::new_v4()).execute(&owner).await.unwrap();
    let query = jobs::VacancyQuery {
        offset: 0,
        limit: 10,
        q: title,
        city: String::new(),
        source: String::new(),
        occupation: String::new(),
        sort: String::new(),
    };
    assert_eq!(
        jobs::page(&pool, Uuid::nil(), &[], &query).await.unwrap()["total"],
        1
    );
    assert!(matches!(
        admin::visibility(&pool, ordinary, vacancy, true).await,
        Err(AppError::Forbidden)
    ));
    admin::visibility(&pool, actor, vacancy, true)
        .await
        .unwrap();
    assert_eq!(
        jobs::page(&pool, Uuid::nil(), &[], &query).await.unwrap()["total"],
        0
    );
    // Cron changes source content/status without overriding moderation.
    let importer = db::connect_runtime_pool(&url, db::RuntimeRole::Importer)
        .await
        .unwrap();
    sqlx::query("UPDATE job_vacancies SET status='active',last_seen_at=now() WHERE id=$1")
        .bind(vacancy)
        .execute(&importer)
        .await
        .unwrap();
    assert_eq!(
        jobs::page(&pool, Uuid::nil(), &[], &query).await.unwrap()["total"],
        0
    );
    let hidden = admin::snapshot(&pool, actor).await.unwrap();
    assert!(
        hidden["postings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == vacancy.to_string() && r["hidden"] == true)
    );
    admin::visibility(&pool, actor, vacancy, false)
        .await
        .unwrap();
    assert_eq!(
        jobs::page(&pool, Uuid::nil(), &[], &query).await.unwrap()["total"],
        1
    );
    admin::remove(&pool, actor, "institutions", institution)
        .await
        .unwrap();
    assert!(
        !sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM trainings WHERE id=$1)")
            .bind(training)
            .fetch_one(&owner)
            .await
            .unwrap()
    );
    admin::remove(&pool, actor, "jobs", job).await.unwrap();
    assert!(
        sqlx::query_scalar::<_, Option<Uuid>>("SELECT target_occupation_id FROM users WHERE id=$1")
            .bind(ordinary)
            .fetch_one(&owner)
            .await
            .unwrap()
            .is_none()
    );
    admin::remove(&pool, actor, "skills", skill).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM user_skills WHERE user_id=$1 AND skill_id=$2"
        )
        .bind(ordinary)
        .bind(skill)
        .fetch_one(&owner)
        .await
        .unwrap(),
        0
    );
    admin::remove(&pool, actor, "skills", other).await.unwrap();
    assert!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM admin_audit WHERE actor_id=$1")
            .bind(actor)
            .fetch_one(&owner)
            .await
            .unwrap()
            >= 10
    );
    // Revocation is checked on every operation, including existing authenticated sessions.
    sqlx::query("UPDATE users SET role_id=1 WHERE id=$1")
        .bind(actor)
        .execute(&owner)
        .await
        .unwrap();
    assert!(matches!(
        admin::snapshot(&pool, actor).await,
        Err(AppError::Forbidden)
    ));
    sqlx::query("DELETE FROM job_vacancies WHERE id=$1")
        .bind(vacancy)
        .execute(&owner)
        .await
        .unwrap();
    sqlx::query("DELETE FROM auth.users WHERE id=ANY($1)")
        .bind(vec![actor, ordinary])
        .execute(&owner)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires a disposable migrated TEST_DATABASE_URL"]
async fn scrape_now_queues_sources_and_searches_wanted_occupations_first() {
    let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL required");
    let owner = PgPool::connect(&url).await.unwrap();
    let name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&owner)
        .await
        .unwrap();
    assert!(name.contains("_test"), "disposable test database required");
    let pool = db::connect_runtime_pool(&url, db::RuntimeRole::Api)
        .await
        .unwrap();
    let importer = db::connect_runtime_pool(&url, db::RuntimeRole::Importer)
        .await
        .unwrap();
    // The alphabetically last keyword: plain round-robin would search it last.
    let (wanted_query, wanted): (Uuid, Uuid) = sqlx::query_as(
        "SELECT id,occupation_id FROM job_search_queries WHERE enabled ORDER BY keyword DESC LIMIT 1",
    )
    .fetch_one(&owner)
    .await
    .unwrap();
    let saved_runs: Vec<(Uuid, Option<chrono::DateTime<chrono::Utc>>)> =
        sqlx::query_as("SELECT id,last_run_at FROM job_search_queries")
            .fetch_all(&owner)
            .await
            .unwrap();
    let admin_id = Uuid::new_v4();
    let seeker = Uuid::new_v4();
    for (id, role) in [(admin_id, 2_i16), (seeker, 1_i16)] {
        sqlx::query("INSERT INTO auth.users(id) VALUES($1)")
            .bind(id)
            .execute(&owner)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users(id,role_id,nama,pendidikan,jurusan,province_id,kota,persetujuan_cv_at,target_occupation_id) VALUES($1,$2,'Scrape fixture','S1','TI',1,'Test',now(),$3)")
            .bind(id).bind(role).bind((role == 1).then_some(wanted)).execute(&owner).await.unwrap();
    }
    sqlx::query("UPDATE job_search_queries SET last_run_at=NULL")
        .execute(&owner)
        .await
        .unwrap();
    // The importer can refresh demand without reading users itself.
    sqlx::query("SELECT public.refresh_search_demand()")
        .execute(&importer)
        .await
        .unwrap();
    assert_eq!(
        jobs::next_queries(&importer, 1).await.unwrap()[0].id,
        wanted_query
    );
    // Searched in the last 2 hours: the rest of the catalog gets the slot (no starvation).
    sqlx::query("UPDATE job_search_queries SET last_run_at=now() WHERE id=$1")
        .bind(wanted_query)
        .execute(&owner)
        .await
        .unwrap();
    assert_ne!(
        jobs::next_queries(&importer, 1).await.unwrap()[0].id,
        wanted_query
    );

    let saved_sources: Vec<(String, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT key,managed_by,next_run_at FROM job_sources WHERE key IN ('grab','openwebninja')",
    )
    .fetch_all(&owner)
    .await
    .unwrap();
    sqlx::query("UPDATE job_sources SET managed_by='rust',next_run_at=now()+interval '1 day',lease_until=NULL,run_id=NULL,requested_at=NULL,last_success_at=NULL WHERE key IN ('grab','openwebninja')")
        .execute(&owner)
        .await
        .unwrap();
    assert!(matches!(
        admin::scrape_now(&pool, seeker).await,
        Err(AppError::Forbidden)
    ));
    let result = admin::scrape_now(&pool, admin_id).await.unwrap();
    let queued: Vec<&str> = result["sources"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(queued.contains(&"grab"));
    assert!(!queued.contains(&"openwebninja"), "metered quota untouched");
    assert!(
        result["priorities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["demand"].as_i64() >= Some(1))
    );
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT next_run_at<=now() FROM job_sources WHERE key='grab'"
        )
        .fetch_one(&owner)
        .await
        .unwrap()
    );
    // A second click within the cooldown queues nothing again.
    let again = admin::scrape_now(&pool, admin_id).await.unwrap();
    assert!(
        !again["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "grab")
    );

    for (key, managed_by, next_run_at) in saved_sources {
        sqlx::query(
            "UPDATE job_sources SET managed_by=$2,next_run_at=$3,requested_at=NULL WHERE key=$1",
        )
        .bind(key)
        .bind(managed_by)
        .bind(next_run_at)
        .execute(&owner)
        .await
        .unwrap();
    }
    let (ids, runs): (Vec<Uuid>, Vec<Option<chrono::DateTime<chrono::Utc>>>) =
        saved_runs.into_iter().unzip();
    sqlx::query("UPDATE job_search_queries q SET last_run_at=s.t FROM unnest($1::uuid[],$2::timestamptz[]) s(id,t) WHERE q.id=s.id")
        .bind(ids)
        .bind(runs)
        .execute(&owner)
        .await
        .unwrap();
    sqlx::query("DELETE FROM auth.users WHERE id=ANY($1)")
        .bind(vec![admin_id, seeker])
        .execute(&owner)
        .await
        .unwrap();
    sqlx::query("SELECT public.refresh_search_demand()")
        .execute(&owner)
        .await
        .unwrap();
}
