//! Never reads .env. Run only against the disposable migrated CI database.
use careerbridge_be::infrastructure::{
    db,
    extract::skillhub::{self, Detail, History, Institution, Snapshot, Training},
};
use serde_json::json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires disposable migrated TEST_DATABASE_URL"]
async fn skillhub_repeat_import_preserves_ids_embeddings_and_revision() {
    let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
    let owner = PgPool::connect(&url).await.unwrap();
    let name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&owner)
        .await
        .unwrap();
    assert!(name.contains("_test"));
    let pool = db::connect_runtime_pool(&url, db::RuntimeRole::CatalogImporter)
        .await
        .unwrap();
    let iid = Uuid::new_v4();
    let source_url = format!("https://skillhub.kemnaker.go.id/mitra/{iid}");
    let institution = Institution {
        id: iid,
        nama: "Native fixture LPK".into(),
        jenis: "LPK".into(),
        province_id: Some(11),
        kota: "Jakarta".into(),
        website: source_url.clone(),
        source_url: source_url.clone(),
    };
    let make = |n: &str| Training {
        id: skillhub::training_id("programs", n, iid),
        institution_id: iid,
        source_kind: "programs".into(),
        source_id: n.into(),
        nama: format!("Fixture {n}"),
        biaya: None,
        moda: None,
        url_daftar: source_url.clone(),
        tag_text: "SQL".into(),
    };
    let history_id = Uuid::new_v4();
    let mut snapshot = Snapshot {
        institutions: vec![institution],
        trainings: vec![make("1"), make("2"), make("3")],
        history: vec![History {
            source_id: history_id,
            institution_id: iid,
            payload: json!({"id":history_id,"name":"Past course"}),
        }],
        details: vec![Detail {
            institution_id: iid,
            payload: json!({"public":{"name":"Native fixture LPK"},"buildings":[]}),
        }],
    };
    let first = skillhub::persist(&pool, &snapshot, false).await.unwrap();
    assert_eq!(first.institutions, 1);
    assert_eq!(first.trainings, 3);
    assert_eq!(first.history, 1);
    let tid = snapshot.trainings[0].id;
    sqlx::query("UPDATE trainings SET embedding=$2,embedding_model='fixture',embedding_hash='fixture' WHERE id=$1").bind(tid).bind(pgvector::Vector::from(vec![0.1;384])).execute(&owner).await.unwrap();
    let repeated = skillhub::persist(&pool, &snapshot, false).await.unwrap();
    assert_eq!(
        repeated.institutions
            + repeated.trainings
            + repeated.history
            + repeated.details
            + repeated.tags_added
            + repeated.tags_removed
            + repeated.deactivated,
        0
    );
    assert_eq!(repeated.revision_bumps, 0);
    let row=sqlx::query("SELECT embedding IS NOT NULL kept,biaya,moda,(SELECT count(*) FROM trainings WHERE institution_id=$2) n FROM trainings WHERE id=$1").bind(tid).bind(iid).fetch_one(&owner).await.unwrap();
    assert!(row.get::<bool, _>("kept"));
    assert_eq!(row.get::<i64, _>("n"), 3);
    assert_eq!(row.get::<Option<i64>, _>("biaya"), None);
    assert_eq!(row.get::<Option<String>, _>("moda"), None);
    snapshot.trainings[0].nama = "Changed title".into();
    let dry = skillhub::persist(&pool, &snapshot, true).await.unwrap();
    assert_eq!(dry.trainings, 1);
    let unchanged: String = sqlx::query_scalar("SELECT nama FROM trainings WHERE id=$1")
        .bind(tid)
        .fetch_one(&owner)
        .await
        .unwrap();
    assert_eq!(unchanged, "Fixture 1");
    snapshot.trainings.clear();
    assert!(skillhub::persist(&pool, &snapshot, false).await.is_err());
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM trainings WHERE institution_id=$1 AND aktif")
            .bind(iid)
            .fetch_one(&owner)
            .await
            .unwrap();
    assert_eq!(count, 3);
    // Restricted importer cannot modify user accounts or non-Skillhub catalogue rows.
    assert!(
        sqlx::query("SELECT * FROM users")
            .fetch_all(&pool)
            .await
            .is_err()
    );
    assert!(sqlx::query("INSERT INTO institutions(id,nama,jenis,kota,website,source) VALUES($1,'Forbidden','LPK','Jakarta','https://example.com','other')").bind(Uuid::new_v4()).execute(&pool).await.is_err());
    sqlx::query("DELETE FROM skillhub_institution_details WHERE institution_id=$1")
        .bind(iid)
        .execute(&owner)
        .await
        .unwrap();
    sqlx::query("DELETE FROM skillhub_training_history WHERE institution_id=$1")
        .bind(iid)
        .execute(&owner)
        .await
        .unwrap();
    sqlx::query("DELETE FROM training_skills WHERE training_id IN(SELECT id FROM trainings WHERE institution_id=$1)").bind(iid).execute(&owner).await.unwrap();
    sqlx::query("DELETE FROM trainings WHERE institution_id=$1")
        .bind(iid)
        .execute(&owner)
        .await
        .unwrap();
    sqlx::query("DELETE FROM institutions WHERE id=$1")
        .bind(iid)
        .execute(&owner)
        .await
        .unwrap();
}
