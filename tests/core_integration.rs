//! Requires a disposable, migrated PostgreSQL+pgvector test database. Never uses .env.
use actix_web::{App, HttpResponse, HttpServer, test, web};
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use careerbridge_be::{
    application::{CvTextExtractor, EmbeddingKind, EmbeddingProvider, LlmProvider},
    infrastructure::{auth::Auth, crypto::Cipher, db, error::AppError, storage::Storage, worker},
    presentation::http::{self, State},
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use uuid::Uuid;
use zeroize::Zeroizing;
struct FakeEmbedding;
#[async_trait]
impl EmbeddingProvider for FakeEmbedding {
    async fn embed(&self, texts: Vec<String>, _: EmbeddingKind) -> Result<Vec<Vec<f32>>, AppError> {
        Ok(texts
            .iter()
            .map(|_| {
                let mut v = vec![0.; 384];
                v[0] = 1.;
                v
            })
            .collect())
    }
    fn version(&self) -> &str {
        "test-model"
    }
}
struct FakePdf;
#[async_trait]
impl CvTextExtractor for FakePdf {
    async fn extract(&self, _: &[u8]) -> Result<Zeroizing<String>, AppError> {
        Ok(Zeroizing::new("Rina Putri\nrina@example.test\nPendidikan SMK\nPengalaman menggunakan excel untuk administrasi kantor".into()))
    }
}
struct FakeLlm {
    calls: AtomicUsize,
}
#[async_trait]
impl LlmProvider for FakeLlm {
    async fn json(&self, instruction: &str, data: Value) -> Result<Value, AppError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if instruction.starts_with("Choose") {
            return Ok(json!({"priorityIndices":[999]}));
        }
        assert!(!data.to_string().contains("rina@example"));
        assert!(!data.to_string().contains("Rina Putri"));
        Ok(json!({"pendidikan":["SMK"],"pengalaman":[],"skills":["excel","administrasi"]}))
    }
    fn model(&self) -> &str {
        "fake-llm"
    }
}
struct Objects {
    files: Mutex<HashMap<String, Vec<u8>>>,
    delete_failures: AtomicUsize,
}
async fn storage_put(
    path: web::Path<String>,
    body: web::Bytes,
    o: web::Data<Objects>,
) -> HttpResponse {
    o.files
        .lock()
        .unwrap()
        .insert(path.into_inner(), body.to_vec());
    HttpResponse::Ok().finish()
}
async fn storage_get(path: web::Path<String>, o: web::Data<Objects>) -> HttpResponse {
    if let Some(v) = o.files.lock().unwrap().get(path.as_str()) {
        HttpResponse::Ok().body(v.clone())
    } else {
        HttpResponse::NotFound().finish()
    }
}
async fn storage_delete(body: web::Json<Value>, o: web::Data<Objects>) -> HttpResponse {
    if o.delete_failures.load(Ordering::SeqCst) > 0 {
        o.delete_failures.fetch_sub(1, Ordering::SeqCst);
        return HttpResponse::ServiceUnavailable().finish();
    }
    let Some(prefixes) = body["prefixes"].as_array() else {
        return HttpResponse::BadRequest().finish();
    };
    let mut files = o.files.lock().unwrap();
    for path in prefixes.iter().filter_map(Value::as_str) {
        files.remove(path);
    }
    HttpResponse::Ok().json(json!([]))
}

#[derive(Clone)]
struct AuthFixture {
    owner: PgPool,
    url: String,
    identities: Arc<Mutex<HashMap<String, Value>>>,
}
async fn auth_session(f: web::Data<AuthFixture>, body: web::Json<Value>) -> HttpResponse {
    let id = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        body["email"]
            .as_str()
            .unwrap_or("browser@example.test")
            .as_bytes(),
    );
    if sqlx::query("INSERT INTO auth.users(id) VALUES($1) ON CONFLICT DO NOTHING")
        .bind(id)
        .execute(&f.owner)
        .await
        .is_err()
    {
        return HttpResponse::InternalServerError().finish();
    }
    let access = token(id, &f.url);
    f.identities.lock().unwrap().insert(
        access.clone(),
        json!({"id":id,"user_metadata":body["data"]}),
    );
    HttpResponse::Ok().json(json!({"access_token":access,"token_type":"bearer","expires_in":600,"refresh_token":"test-only-refresh","user":{"id":id,"aud":"authenticated","role":"authenticated","email":body["email"],"email_confirmed_at":"2026-01-01T00:00:00Z","created_at":"2026-01-01T00:00:00Z","app_metadata":{"provider":"email","providers":["email"]},"user_metadata":body["data"]}}))
}
async fn setup() -> (
    web::Data<State>,
    PgPool,
    web::Data<Objects>,
    Arc<FakeLlm>,
    String,
    actix_web::dev::ServerHandle,
) {
    let url = std::env::var("TEST_DATABASE_URL")
        .expect("set TEST_DATABASE_URL to a disposable migrated database");
    let owner = PgPool::connect(&url).await.unwrap();
    let pool = db::connect_runtime_pool(&url, db::RuntimeRole::Api)
        .await
        .unwrap();
    let worker_pool = db::connect_runtime_pool(&url, db::RuntimeRole::Worker)
        .await
        .unwrap();
    let objects = web::Data::new(Objects {
        files: Mutex::new(HashMap::new()),
        delete_failures: AtomicUsize::new(0),
    });
    let cloned = objects.clone();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let auth_fixture = web::Data::new(AuthFixture {
        owner: owner.clone(),
        url: url.clone(),
        identities: Arc::new(Mutex::new(HashMap::new())),
    });
    let server = HttpServer::new(move || {
        App::new()
            .wrap(actix_cors::Cors::permissive())
            .app_data(auth_fixture.clone())
            .route("/auth/v1/token", web::post().to(auth_session))
            .route("/auth/v1/signup", web::post().to(auth_session))
            .route(
                "/auth/v1/user",
                web::get().to(
                    |req: actix_web::HttpRequest, f: web::Data<AuthFixture>| async move {
                        let token = req
                            .headers()
                            .get("authorization")
                            .and_then(|h| h.to_str().ok())
                            .and_then(|h| h.strip_prefix("Bearer "))
                            .unwrap_or("");
                        match f.identities.lock().unwrap().get(token) {
                            Some(user) => HttpResponse::Ok().json(user),
                            None => HttpResponse::Unauthorized().finish(),
                        }
                    },
                ),
            )
            .route(
                "/auth/v1/logout",
                web::post().to(|| async { HttpResponse::NoContent().finish() }),
            )
            .app_data(cloned.clone())
            .route(
                "/auth/v1/.well-known/jwks.json",
                web::get().to(|| async {
                    HttpResponse::Ok()
                        .content_type("application/json")
                        .body(include_str!("fixtures/jwks.json"))
                }),
            )
            .route(
                "/storage/v1/object/cv-private",
                web::delete().to(storage_delete),
            )
            .service(
                web::resource("/storage/v1/object/cv-private/{path:.*}")
                    .route(web::post().to(storage_put))
                    .route(web::get().to(storage_get)),
            )
    })
    .listen(listener)
    .unwrap();
    let url = format!("http://{}", server.addrs()[0]);
    let server = server.run();
    let handle = server.handle();
    actix_web::rt::spawn(server);
    let llm = Arc::new(FakeLlm {
        calls: AtomicUsize::new(0),
    });
    let client = reqwest::Client::new();
    let state = web::Data::new(State {
        events: Default::default(),
        pool,
        worker_pool,
        auth: Auth::new(&url, client.clone()),
        cipher: Cipher::new(&STANDARD.encode([9u8; 32])).unwrap(),
        storage: Storage {
            client,
            url: url.clone(),
            key: "test-only".into(),
            bucket: "cv-private".into(),
        },
        embedding: Arc::new(FakeEmbedding),
        llm: llm.clone(),
        pdf: Arc::new(FakePdf),
    });
    (state, owner, objects, llm, url, handle)
}
fn token(user: Uuid, url: &str) -> String {
    let mut h = Header::new(Algorithm::ES256);
    h.kid = Some("test-only".into());
    encode(&h,&json!({"sub":user,"email":"rina@example.test","iss":format!("{url}/auth/v1"),"aud":"authenticated","exp":chrono::Utc::now().timestamp()+600}),&EncodingKey::from_ec_der(include_bytes!("fixtures/test-jwt.der"))).unwrap()
}
#[actix_web::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn core_flow_is_isolated_recoverable_and_cache_aware() {
    let (s, owner, objects, llm, url, server) = setup().await;
    let app = test::init_service(App::new().app_data(s.clone()).configure(http::configure)).await;
    let u = Uuid::new_v4();
    let other = Uuid::new_v4();
    for id in [u, other] {
        sqlx::query("INSERT INTO auth.users(id) VALUES($1)")
            .bind(id)
            .execute(&owner)
            .await
            .unwrap();
    }
    let auth = format!("Bearer {}", token(u, &url));
    let other_auth = format!("Bearer {}", token(other, &url));
    for bearer in [&auth, &other_auth] {
        let response=test::call_service(&app,test::TestRequest::post().uri("/api/v1/me").insert_header(("Authorization",bearer.as_str())).set_json(json!({"nama":"Rina Putri","pendidikan":"SMK","jurusan":"Administrasi","provinsi":"Jawa Barat","kota":"Bandung","consent":true})).to_request()).await;
        assert_eq!(response.status(), 200);
    }
    let invalid = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/v1/me")
            .insert_header(("Authorization", "Bearer invalid"))
            .to_request(),
    )
    .await;
    assert_eq!(invalid.status(), 401);
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some("test-only".into());
    for claims in [
        json!({"sub":u,"exp":chrono::Utc::now().timestamp()-60,"iss":format!("{url}/auth/v1"),"aud":"authenticated"}),
        json!({"sub":u,"exp":chrono::Utc::now().timestamp()+600,"iss":"https://wrong.invalid/auth/v1","aud":"authenticated"}),
        json!({"sub":u,"exp":chrono::Utc::now().timestamp()+600,"iss":format!("{url}/auth/v1"),"aud":"wrong"}),
    ] {
        let bad = encode(
            &header,
            &claims,
            &EncodingKey::from_ec_der(include_bytes!("fixtures/test-jwt.der")),
        )
        .unwrap();
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/api/v1/me")
                .insert_header(("Authorization", format!("Bearer {bad}")))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 401);
    }
    let boundary = "careerbridge-boundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"cv.pdf\"\r\nContent-Type: application/pdf\r\n\r\n%PDF-test-only\r\n--{boundary}--\r\n"
    );
    let upload = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/v1/me/cv")
            .insert_header(("Authorization", auth.as_str()))
            .insert_header((
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            ))
            .set_payload(body.clone())
            .to_request(),
    )
    .await;
    assert_eq!(upload.status(), 202);
    let uploaded: Value = test::read_body_json(upload).await;
    let id = uploaded["id"].as_str().unwrap();
    let forbidden = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/api/v1/me/cv/{id}"))
            .insert_header(("Authorization", other_auth.as_str()))
            .to_request(),
    )
    .await;
    assert_eq!(forbidden.status(), 404);
    // Simulate an expired worker lease, then recover the durable job.
    sqlx::query(
        "UPDATE cv_uploads SET lease_until=now()-interval '1 second',attempts=1 WHERE id=$1",
    )
    .bind(Uuid::parse_str(id).unwrap())
    .execute(&owner)
    .await
    .unwrap();
    objects.delete_failures.store(1, Ordering::SeqCst);
    worker::tick(&s).await.unwrap();
    let complete: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri(&format!("/api/v1/me/cv/{id}"))
            .insert_header(("Authorization", auth.as_str()))
            .to_request(),
    )
    .await;
    assert_eq!(complete["status"], "selesai");
    assert_eq!(complete["parsed"]["skills"][0], "excel");
    assert!(!objects.files.lock().unwrap().is_empty());
    // Cleanup is now independent of queue processing. Its first failed delete
    // must preserve the object/path so the next maintenance pass can retry.
    worker::cleanup(&s).await.unwrap();
    assert!(!objects.files.lock().unwrap().is_empty());
    worker::cleanup(&s).await.unwrap();
    assert!(objects.files.lock().unwrap().is_empty());
    assert_eq!(llm.calls.load(Ordering::SeqCst), 1);
    let row = sqlx::query("SELECT parsed_json,file_path,attempts FROM cv_uploads WHERE id=$1")
        .bind(Uuid::parse_str(id).unwrap())
        .fetch_one(&owner)
        .await
        .unwrap();
    assert!(!row.get::<String, _>("parsed_json").contains("excel"));
    assert!(row.get::<Option<String>, _>("file_path").is_none());
    assert_eq!(row.get::<i16, _>("attempts"), 2);
    let reused: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::post()
            .uri("/api/v1/me/cv")
            .insert_header(("Authorization", auth.as_str()))
            .insert_header((
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            ))
            .set_payload(body)
            .to_request(),
    )
    .await;
    assert_eq!(reused["id"], id);
    assert_eq!(reused["reused"], true);
    let saved=test::call_service(&app,test::TestRequest::put().uri("/api/v1/me/skills").insert_header(("Authorization",auth.as_str())).set_json(json!({"skills":complete["normalized"]["matched"].as_array().unwrap().iter().map(|v|json!({"skillId":v["skillId"],"sumber":"cv_semantik"})).collect::<Vec<_>>(),"cvId":id})).to_request()).await;
    assert_eq!(saved.status(), 200);
    let ranked: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri("/api/v1/me/matches")
            .insert_header(("Authorization", auth.as_str()))
            .to_request(),
    )
    .await;
    let job = ranked["items"][0]["job"]["id"].as_str().unwrap();
    let analysis: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri(&format!("/api/v1/me/analysis/{job}"))
            .insert_header(("Authorization", auth.as_str()))
            .to_request(),
    )
    .await;
    assert_eq!(analysis["fallback"], true);
    assert_eq!(
        llm.calls.load(Ordering::SeqCst),
        3,
        "invalid explanation plans are retried once before fallback"
    );
    assert!(analysis["explanation"].as_str().unwrap().contains("Skormu"));
    // Only the server decides provenance: a forged semantic source becomes the CV's alias source.
    let profile: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri("/api/v1/me")
            .insert_header(("Authorization", auth.as_str()))
            .to_request(),
    )
    .await;
    assert_eq!(profile["cv"]["id"], id);
    assert!(
        profile["skills"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["sumber"] == "cv_alias")
    );
    let page: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri("/api/v1/catalog/skills?limit=2&offset=1")
            .insert_header(("Authorization", auth.as_str()))
            .to_request(),
    )
    .await;
    assert_eq!(page["items"].as_array().unwrap().len(), 2);
    assert_eq!(page["total"], 66);
    db::validate_runtime_role(&s.pool, "careerbridge_api")
        .await
        .unwrap();
    assert!(
        db::validate_runtime_role(&owner, "careerbridge_api")
            .await
            .is_err()
    );
    let count = llm.calls.load(Ordering::SeqCst);
    let _: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri(&format!("/api/v1/me/analysis/{job}"))
            .insert_header(("Authorization", auth.as_str()))
            .to_request(),
    )
    .await;
    assert_eq!(count, llm.calls.load(Ordering::SeqCst));
    let changed = test::call_service(
        &app,
        test::TestRequest::put()
            .uri("/api/v1/me/target")
            .insert_header(("Authorization", auth.as_str()))
            .set_json(json!({"occupationId":job}))
            .to_request(),
    )
    .await;
    assert_eq!(changed.status(), 200);
    let cached: i64 = sqlx::query_scalar("SELECT count(*) FROM analyses WHERE user_id=$1")
        .bind(u)
        .fetch_one(&owner)
        .await
        .unwrap();
    assert_eq!(cached, 0);
    let deleted = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri("/api/v1/me/cv")
            .insert_header(("Authorization", auth.as_str()))
            .to_request(),
    )
    .await;
    assert_eq!(deleted.status(), 200);
    worker::cleanup(&s).await.unwrap();
    let mut tx = db::user_tx(&s.pool, u).await.unwrap();
    let remain: i64 = sqlx::query_scalar("SELECT count(*) FROM user_skills")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert!(remain > 0);
    tx.commit().await.unwrap();
    let mut tx = db::user_tx(&s.pool, other).await.unwrap();
    let remain: i64 = sqlx::query_scalar("SELECT count(*) FROM user_skills")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(remain, 0);
    tx.commit().await.unwrap();
    worker::refresh_embeddings(&s).await.unwrap();
    let stable_revision: i64 = sqlx::query_scalar("SELECT revision FROM catalog_revision")
        .fetch_one(&owner)
        .await
        .unwrap();
    worker::refresh_embeddings(&s).await.unwrap();
    let next_revision: i64 = sqlx::query_scalar("SELECT revision FROM catalog_revision")
        .fetch_one(&owner)
        .await
        .unwrap();
    assert_eq!(
        stable_revision, next_revision,
        "unchanged catalog must not regenerate vectors"
    );
    let prior: Vec<(Uuid, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as("SELECT id,embedded_at FROM skills ORDER BY id")
            .fetch_all(&owner)
            .await
            .unwrap();
    let alias = format!("integration alias {}", Uuid::new_v4());
    sqlx::query("INSERT INTO skill_aliases(alias,skill_id) VALUES($1,$2)")
        .bind(&alias)
        .bind(prior[0].0)
        .execute(&owner)
        .await
        .unwrap();
    worker::refresh_embeddings(&s).await.unwrap();
    let next: Vec<(Uuid, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as("SELECT id,embedded_at FROM skills ORDER BY id")
            .fetch_all(&owner)
            .await
            .unwrap();
    assert_eq!(
        prior.iter().zip(&next).filter(|(a, b)| a != b).count(),
        1,
        "only the changed skill vector is regenerated"
    );
    sqlx::query("DELETE FROM skill_aliases WHERE alias=$1")
        .bind(alias)
        .execute(&owner)
        .await
        .unwrap();
    for id in [u, other] {
        sqlx::query("DELETE FROM auth.users WHERE id=$1")
            .bind(id)
            .execute(&owner)
            .await
            .unwrap();
    }
    server.stop(true).await;
}

/// Starts isolated fake external providers and a real API for browser tests.
/// Never compiled into the production application and never uses production credentials.
#[actix_web::test]
#[ignore = "manual browser fixture: requires CAREERBRIDGE_SERVE_TEST=1"]
async fn serve_browser_fixture() {
    if std::env::var("CAREERBRIDGE_SERVE_TEST").as_deref() != Ok("1") {
        return;
    }
    let (state, _owner, _objects, _llm, _url, _providers) = setup().await;
    let mut state = state.into_inner();
    if std::env::var("E2E_REAL_EMBEDDING").as_deref() == Ok("1") {
        let state_mut = Arc::get_mut(&mut state).unwrap();
        state_mut.embedding = careerbridge_be::infrastructure::embedding::LocalEmbedding::load(
            ".cache/fastembed".into(),
        )
        .await
        .unwrap();
    }
    let state_mut = Arc::get_mut(&mut state).unwrap();
    state_mut.pdf = Arc::new(careerbridge_be::infrastructure::pdf::SubprocessPdf {
        executable: std::path::PathBuf::from(env!("CARGO_BIN_EXE_careerbridge-be")),
    });
    let state = web::Data::from(state);
    worker::refresh_embeddings(&state).await.unwrap();
    careerbridge_be::presentation::events::start_background(state.clone().into_inner());
    let worker_state = state.clone();
    actix_web::rt::spawn(async move {
        loop {
            worker::tick(&worker_state).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    });
    let server_state = state.clone();
    let server = HttpServer::new(move || {
        App::new()
            .wrap(
                actix_cors::Cors::default()
                    .allowed_origin("http://127.0.0.1:5173")
                    .allowed_origin("http://localhost:5173")
                    .allowed_methods(vec!["GET", "POST", "PUT", "PATCH", "DELETE"])
                    .allowed_headers(vec!["content-type", "x-csrf-protection"])
                    .supports_credentials(),
            )
            .app_data(server_state.clone())
            .app_data(web::Data::new(
                careerbridge_be::presentation::session::CookieAuth::new(
                    false,
                    vec![
                        "http://127.0.0.1:5173".into(),
                        "http://localhost:5173".into(),
                    ],
                ),
            ))
            .configure(http::configure)
    })
    .bind(("127.0.0.1", 55433))
    .unwrap()
    .run();
    std::fs::write(
        "/tmp/careerbridge-e2e-config.json",
        serde_json::to_string(&json!({"apiUrl":"http://127.0.0.1:55433/api/v1"})).unwrap(),
    )
    .unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn subprocess_pdf_parser_reads_fixture_and_rejects_malformed_input() {
    let parser = careerbridge_be::infrastructure::pdf::SubprocessPdf {
        executable: std::path::PathBuf::from(env!("CARGO_BIN_EXE_careerbridge-be")),
    };
    let text = parser
        .extract(include_bytes!("fixtures/cvs/cv-01.pdf"))
        .await
        .unwrap();
    assert!(text.contains("Skill: excel"));
    assert!(parser.extract(b"%PDF-definitely-not-a-PDF").await.is_err());
}

/// A complete backend journey over TCP; external providers remain deterministic fixtures.
#[actix_web::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn backend_http_journey() {
    let (state, owner, objects, _llm, url, providers) = setup().await;
    let mut inner = state.into_inner();
    Arc::get_mut(&mut inner).unwrap().pdf =
        Arc::new(careerbridge_be::infrastructure::pdf::SubprocessPdf {
            executable: std::path::PathBuf::from(env!("CARGO_BIN_EXE_careerbridge-be")),
        });
    let state = web::Data::from(inner);
    worker::refresh_embeddings(&state).await.unwrap();
    let server_state = state.clone();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = HttpServer::new(move || {
        App::new()
            .app_data(server_state.clone())
            .configure(http::configure)
    })
    .workers(1)
    .listen(listener)
    .unwrap()
    .run();
    let handle = server.handle();
    actix_web::rt::spawn(server);
    let client = reqwest::Client::new();
    let user = Uuid::new_v4();
    sqlx::query("INSERT INTO auth.users(id) VALUES($1)")
        .bind(user)
        .execute(&owner)
        .await
        .unwrap();
    let jwt = token(user, &url);
    let response = client.get(format!("{base}/health")).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        client
            .get(format!("{base}/ready"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .get(format!("{base}/api/v1/me"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let response = client.post(format!("{base}/api/v1/me")).bearer_auth(&jwt)
        .json(&json!({"nama":"HTTP Journey","pendidikan":"SMK","jurusan":"Administrasi","provinsi":"Jawa Barat","kota":"Bandung","consent":true}))
        .send().await.unwrap();
    assert_eq!(response.status(), 200);
    // Domicile filter: offline programs only from the user's province, online from anywhere.
    let tag = format!("Domisili{}", Uuid::new_v4().simple());
    let (local, remote, online) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    for (id, institution, moda) in [
        (local, "0388e22a-8181-546b-9e51-3d0d2d8c0a65", "luring"), // BLK Bandung, Jawa Barat
        (remote, "83eeb1d9-f573-5ad2-a9c5-593369441b1f", "luring"), // LPK Prima Administrasi, DKI Jakarta
        (online, "a0434ed4-37b7-53f5-9d97-fcd540f924e0", "daring"), // Kelas Digital Indonesia, DKI Jakarta
    ] {
        sqlx::query("INSERT INTO trainings(id,institution_id,nama,biaya,moda,durasi,url_daftar,sample) VALUES($1,$2::uuid,$3,0,$4,'1 hari','https://example.test',false)")
            .bind(id).bind(institution).bind(format!("{tag} {moda}")).bind(moda)
            .execute(&owner).await.unwrap();
    }
    for (filter, expected) in [
        ("", vec![local, online]),
        ("&province=DKI%20Jakarta", vec![local, online]),
        ("&mode=luring", vec![local]),
        ("&mode=daring", vec![online]),
    ] {
        let page: Value = client
            .get(format!(
                "{base}/api/v1/me/trainings?limit=10&q={tag}{filter}"
            ))
            .bearer_auth(&jwt)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let mut ids: Vec<Uuid> = page["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap().parse().unwrap())
            .collect();
        let mut expected = expected;
        ids.sort();
        expected.sort();
        assert_eq!(ids, expected, "filter {filter:?}");
    }
    sqlx::query("DELETE FROM trainings WHERE id=ANY($1)")
        .bind(vec![local, remote, online])
        .execute(&owner)
        .await
        .unwrap();
    assert_eq!(
        client
            .get(format!("{base}/api/v1/me/vacancies"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .get(format!("{base}/api/v1/me/vacancies?limit=101"))
            .bearer_auth(&jwt)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    let vacancies: Value = client
        .get(format!(
            "{base}/api/v1/me/vacancies?limit=1&city=Yogyakarta"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(vacancies["limit"], 1);
    assert!(vacancies["items"].as_array().unwrap().len() <= 1);
    let boundary = "http-journey";
    let mut body = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"cv.pdf\"\r\nContent-Type: application/pdf\r\n\r\n").into_bytes();
    body.extend_from_slice(include_bytes!("fixtures/cvs/cv-01.pdf"));
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let response = client
        .post(format!("{base}/api/v1/me/cv"))
        .bearer_auth(&jwt)
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 202);
    let upload: Value = response.json().await.unwrap();
    worker::tick(&state).await.unwrap();
    let status: Value = client
        .get(format!(
            "{base}/api/v1/me/cv/{}",
            upload["id"].as_str().unwrap()
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["status"], "selesai");
    let skills: Vec<Value> = status["normalized"]["matched"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| json!({"skillId":v["skillId"],"sumber":v["sumber"]}))
        .collect();
    assert!(!skills.is_empty());
    let response = client
        .put(format!("{base}/api/v1/me/skills"))
        .bearer_auth(&jwt)
        .json(&json!({"skills":skills,"cvId":upload["id"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let matches: Value = client
        .get(format!("{base}/api/v1/me/matches"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let job = matches["items"][0]["job"]["id"].as_str().unwrap();
    let analysis: Value = client
        .get(format!("{base}/api/v1/me/analysis/{job}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(analysis["explanation"].as_str().unwrap().contains("Skormu"));
    let response = client
        .delete(format!("{base}/api/v1/me/cv"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    worker::cleanup(&state).await.unwrap();
    assert!(objects.files.lock().unwrap().is_empty());
    let profile: Value = client
        .get(format!("{base}/api/v1/me"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(profile["cv"].is_null());
    assert!(!profile["skills"].as_array().unwrap().is_empty());
    sqlx::query("DELETE FROM auth.users WHERE id=$1")
        .bind(user)
        .execute(&owner)
        .await
        .unwrap();
    handle.stop(true).await;
    providers.stop(true).await;
}

#[actix_web::test]
async fn swagger_ui_serves_local_assets_and_cookie_contract_without_credentials() {
    let app = test::init_service(App::new().configure(http::configure)).await;
    let redirect = test::call_service(
        &app,
        test::TestRequest::get().uri("/swagger-ui").to_request(),
    )
    .await;
    assert_eq!(redirect.status(), 307);
    assert_eq!(redirect.headers().get("Location").unwrap(), "/swagger-ui/");
    for (path, content_type) in [
        ("/swagger-ui/", "text/html"),
        ("/swagger-ui/swagger-ui-bundle.js", "application/javascript"),
        ("/swagger-ui/swagger-ui.css", "text/css"),
        ("/swagger-ui/initializer.js", "application/javascript"),
    ] {
        let response =
            test::call_service(&app, test::TestRequest::get().uri(path).to_request()).await;
        assert_eq!(response.status(), 200, "{path}");
        assert!(
            response
                .headers()
                .get("Content-Type")
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(content_type)
        );
        assert!(!test::read_body(response).await.is_empty());
    }
    let spec: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri("/api/v1/openapi.json")
            .to_request(),
    )
    .await;
    assert_eq!(
        spec["components"]["securitySchemes"]["cookieAuth"]["in"],
        "cookie"
    );
    assert_eq!(spec["servers"][0]["url"], "/");
    assert!(spec["paths"]["/api/v1/me/cv"]["post"].is_object());
    assert_eq!(
        spec["components"]["schemas"]["CatalogKind"]["enum"],
        json!([
            "skills",
            "occupations",
            "institutions",
            "trainings",
            "provinces"
        ])
    );
    for (path, methods) in [
        ("/api/v1/me", vec!["get", "post", "patch"]),
        ("/api/v1/catalog/{kind}", vec!["get"]),
        ("/api/v1/me/cv", vec!["post", "delete"]),
        ("/api/v1/me/cv/{id}", vec!["get"]),
        ("/api/v1/me/skills", vec!["put"]),
        ("/api/v1/me/target", vec!["put"]),
        ("/api/v1/me/matches", vec!["get"]),
        ("/api/v1/me/analysis/{id}", vec!["get"]),
    ] {
        for method in methods {
            assert_eq!(
                spec["paths"][path][method]["security"][0]["cookieAuth"],
                json!([])
            );
        }
    }
    assert_eq!(
        spec["components"]["schemas"]["PdfUpload"]["properties"]["file"]["format"],
        "binary"
    );
    assert_eq!(
        spec["components"]["schemas"]["Training"]["properties"]["biaya"]["type"],
        json!(["integer", "null"])
    );
    assert_eq!(
        spec["components"]["schemas"]["SkillSource"]["enum"],
        json!(["manual", "cv_alias", "cv_semantik"])
    );

    let missing = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/swagger-ui/unknown.js")
            .to_request(),
    )
    .await;
    assert_eq!(missing.status(), 404);
}

/// Read-only live/disposable connection check for the one-URL pool initialization.
#[actix_web::test]
#[ignore = "requires TEST_DATABASE_URL with migrations applied"]
async fn single_url_runtime_connections_enforce_roles_and_rls() {
    let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL required");
    for (role, name) in [
        (db::RuntimeRole::Api, "careerbridge_api"),
        (db::RuntimeRole::Worker, "careerbridge_worker"),
    ] {
        let pool = db::connect_runtime_pool(&url, role).await.unwrap();
        db::validate_runtime_role(&pool, name).await.unwrap();
        let mut first = pool.acquire().await.unwrap();
        let mut second = pool.acquire().await.unwrap();
        for connection in [&mut first, &mut second] {
            let identity: String = sqlx::query_scalar("SELECT current_user::text")
                .fetch_one(&mut **connection)
                .await
                .unwrap();
            assert_eq!(identity, name);
            let auth_read: bool = sqlx::query_scalar(
                "SELECT has_table_privilege(current_user,(SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='auth' AND c.relname='users'),'SELECT')",
            )
            .fetch_one(&mut **connection)
            .await
            .unwrap();
            assert!(
                !auth_read,
                "runtime role must not inherit owner auth privileges"
            );
        }
        drop(first);
        drop(second);
        if name == "careerbridge_api" {
            let visible: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(
                visible, 0,
                "API pool must enforce RLS when no user is selected"
            );
        }
        pool.close().await;
    }
}

#[actix_web::test]
async fn storage_adapter_roundtrip_matches_supabase_delete_contract() {
    let objects = web::Data::new(Objects {
        files: Mutex::new(HashMap::new()),
        delete_failures: AtomicUsize::new(0),
    });
    let server_objects = objects.clone();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = HttpServer::new(move || {
        App::new()
            .app_data(server_objects.clone())
            .route(
                "/storage/v1/object/cv-private",
                web::delete().to(storage_delete),
            )
            .service(
                web::resource("/storage/v1/object/cv-private/{path:.*}")
                    .route(web::post().to(storage_put))
                    .route(web::get().to(storage_get)),
            )
    })
    .workers(1)
    .listen(listener)
    .unwrap()
    .run();
    let handle = server.handle();
    actix_web::rt::spawn(server);
    let storage = Storage {
        client: reqwest::Client::new(),
        url,
        key: "sb_secret_test_fixture".into(),
        bucket: "cv-private".into(),
    };
    let path = "user-id/upload-id";
    let bytes = b"%PDF-synthetic-contract-fixture".to_vec();
    storage.put(path, bytes.clone()).await.unwrap();
    assert_eq!(storage.get(path).await.unwrap(), bytes);
    objects.delete_failures.store(1, Ordering::SeqCst);
    assert!(storage.delete(path).await.is_err());
    assert!(!objects.files.lock().unwrap().is_empty());
    storage.delete(path).await.unwrap();
    assert!(objects.files.lock().unwrap().is_empty());
    storage.delete(path).await.unwrap();
    handle.stop(true).await;
}

/// Non-destructive live acceptance using only the pre-provisioned synthetic accounts.
/// Runs only when explicitly selected with CAREERBRIDGE_LIVE_AUTH_TEST=1.
#[actix_web::test]
#[ignore = "explicit live Auth/session acceptance only"]
async fn live_cookie_sessions_reject_csrf_refresh_and_revoke() {
    if std::env::var("CAREERBRIDGE_LIVE_AUTH_TEST").as_deref() != Ok("1") {
        return;
    }
    dotenvy::dotenv().ok();
    let url = std::env::var("SUPABASE_URL").unwrap();
    let key = std::env::var("SUPABASE_SECRET_KEY").unwrap();
    let database = careerbridge_be::infrastructure::config::database_url().unwrap();
    let pool = db::connect_runtime_pool(&database, db::RuntimeRole::Api)
        .await
        .unwrap();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let state = web::Data::new(State {
        events: Default::default(),
        pool: pool.clone(),
        worker_pool: pool.clone(),
        auth: Auth::new(&url, client.clone()),
        cipher: Cipher::new(&std::env::var("CV_ENCRYPTION_KEY").unwrap()).unwrap(),
        storage: Storage {
            client,
            url,
            key,
            bucket: "cv-private".into(),
        },
        pdf: Arc::new(FakePdf),
        embedding: Arc::new(FakeEmbedding),
        llm: Arc::new(FakeLlm {
            calls: AtomicUsize::new(0),
        }),
    });
    let config = web::Data::new(careerbridge_be::presentation::session::CookieAuth::new(
        true,
        vec!["https://app.example.test".into()],
    ));
    let app = test::init_service(
        App::new()
            .app_data(state.clone())
            .app_data(config)
            .configure(http::configure),
    )
    .await;
    let accounts: Value =
        serde_json::from_slice(&std::fs::read(".cache/test-accounts.json").unwrap()).unwrap();
    let accounts = accounts["accounts"].as_array().unwrap();
    let mut sessions = Vec::new();
    for account in accounts.iter().filter(|a| a["role"] == "user").take(2) {
        let body = json!({"email":account["email"],"password":account["password"]});
        let rejected = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/v1/auth/login")
                .set_json(&body)
                .to_request(),
        )
        .await;
        assert_eq!(rejected.status(), 403);
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/v1/auth/login")
                .peer_addr("127.0.0.1:1234".parse().unwrap())
                .insert_header(("origin", "https://app.example.test"))
                .insert_header(("x-csrf-protection", "1"))
                .set_json(&body)
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        let cookie = response.response().cookies().next().unwrap().into_owned();
        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.secure(), Some(true));
        let body: Value = test::read_body_json(response).await;
        assert!(body.get("access_token").is_none());
        assert!(body.get("refresh_token").is_none());
        let identity: Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get()
                .uri("/api/v1/me")
                .cookie(cookie.clone())
                .to_request(),
        )
        .await;
        assert_eq!(identity["email"], account["email"]);
        let summary: Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get()
                .uri("/api/v1/me/summary")
                .cookie(cookie.clone())
                .to_request(),
        )
        .await;
        assert!(summary["items"].as_array().unwrap().len() <= 7);
        assert!(summary["freeTrainingCount"].is_i64());
        assert!(summary.get("trainings").is_none());
        let trainings: Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get()
                .uri("/api/v1/me/trainings?limit=2&cost=unknown")
                .cookie(cookie.clone())
                .to_request(),
        )
        .await;
        assert!(trainings["items"].as_array().unwrap().len() <= 2);
        assert!(
            trainings["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|t| t["biaya"].is_null())
        );
        let invalid = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/api/v1/me/trainings?limit=101")
                .cookie(cookie.clone())
                .to_request(),
        )
        .await;
        assert_eq!(invalid.status(), 400);
        let forged = test::call_service(
            &app,
            test::TestRequest::put()
                .uri("/api/v1/me/target")
                .cookie(cookie.clone())
                .insert_header(("origin", "https://evil.test"))
                .insert_header(("x-csrf-protection", "1"))
                .set_json(json!({"occupationId":null}))
                .to_request(),
        )
        .await;
        assert_eq!(forged.status(), 403);
        sessions.push(cookie);
    }
    assert_eq!(sessions.len(), 2);
    // Force only this synthetic session to refresh; no profile/catalog mutations.
    let secret = sessions[0].value();
    use sha2::{Digest, Sha256};
    let hash = format!("{:x}", Sha256::digest(secret.as_bytes()));
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.session_hash',$1,true)")
        .bind(&hash)
        .execute(&mut *tx)
        .await
        .unwrap();
    let encrypted: String =
        sqlx::query_scalar("SELECT encrypted_tokens FROM browser_sessions WHERE secret_hash=$1")
            .bind(&hash)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert!(!encrypted.contains("access_token"));
    let plain = state.cipher.decrypt(&encrypted).unwrap();
    let mut tokens: Value = serde_json::from_str(&plain).unwrap();
    let bearer_only = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/v1/me")
            .insert_header((
                "authorization",
                format!("Bearer {}", tokens["access_token"].as_str().unwrap()),
            ))
            .to_request(),
    )
    .await;
    assert_eq!(bearer_only.status(), 401);
    tokens["access_token"] = json!("expired-test-token");
    let encrypted = state.cipher.encrypt(&tokens.to_string()).unwrap();
    sqlx::query("UPDATE browser_sessions SET encrypted_tokens=$1 WHERE secret_hash=$2")
        .bind(encrypted)
        .bind(&hash)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let (first, second) = tokio::join!(
        test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/api/v1/auth/session")
                .cookie(sessions[0].clone())
                .to_request()
        ),
        test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/api/v1/auth/session")
                .cookie(sessions[0].clone())
                .to_request()
        )
    );
    assert_eq!(first.status(), 200);
    assert_eq!(second.status(), 200);
    // With no session context, RLS hides every session row.
    let visible: i64 = sqlx::query_scalar("SELECT count(*) FROM browser_sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(visible, 0);
    let worker = db::connect_runtime_pool(&database, db::RuntimeRole::Worker)
        .await
        .unwrap();
    let can_read_tokens: bool = sqlx::query_scalar(
        "SELECT has_column_privilege(current_user,'browser_sessions','encrypted_tokens','SELECT')",
    )
    .fetch_one(&worker)
    .await
    .unwrap();
    assert!(!can_read_tokens);
    let mut tx = pool.begin().await.unwrap();
    let expired_hash = format!("{:x}", Sha256::digest(sessions[1].value().as_bytes()));
    sqlx::query("SELECT set_config('app.session_hash',$1,true)")
        .bind(&expired_hash)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE browser_sessions SET expires_at=now()-interval '1 second' WHERE secret_hash=$1",
    )
    .bind(&expired_hash)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let expired = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/v1/me")
            .cookie(sessions[1].clone())
            .to_request(),
    )
    .await;
    assert_eq!(expired.status(), 401);
    sqlx::query("DELETE FROM browser_sessions WHERE expires_at<=now()")
        .execute(&worker)
        .await
        .unwrap();
    worker.close().await;
    for cookie in sessions {
        let logout = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/v1/auth/logout")
                .cookie(cookie.clone())
                .insert_header(("origin", "https://app.example.test"))
                .insert_header(("x-csrf-protection", "1"))
                .to_request(),
        )
        .await;
        assert_eq!(logout.status(), 200);
        assert_eq!(
            logout
                .response()
                .cookies()
                .next()
                .unwrap()
                .max_age()
                .unwrap()
                .whole_seconds(),
            0
        );
        let replay = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/api/v1/me")
                .cookie(cookie)
                .to_request(),
        )
        .await;
        assert_eq!(replay.status(), 401);
    }
}

/// Read-only query acceptance against the configured catalog; no destructive bootstrap.
#[actix_web::test]
#[ignore = "explicit live pagination acceptance only"]
async fn live_pagination_queries_are_bounded_and_filtered() {
    if std::env::var("CAREERBRIDGE_LIVE_PAGING_TEST").as_deref() != Ok("1") {
        return;
    }
    dotenvy::dotenv().ok();
    use careerbridge_be::infrastructure::browsing::{self, Browse};
    let url = careerbridge_be::infrastructure::config::database_url().unwrap();
    let pool = db::connect_runtime_pool(&url, db::RuntimeRole::Api)
        .await
        .unwrap();
    let first = browsing::catalog(
        &pool,
        "trainings",
        &Browse {
            limit: 3,
            ..Default::default()
        },
        None,
    )
    .await
    .unwrap();
    let second = browsing::catalog(
        &pool,
        "trainings",
        &Browse {
            limit: 3,
            offset: 3,
            ..Default::default()
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(first["items"].as_array().unwrap().len(), 3);
    assert_eq!(first["total"], second["total"]);
    assert!(first["total"].as_i64().unwrap() > 3);
    for a in first["items"].as_array().unwrap() {
        assert!(
            second["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|b| a["id"] != b["id"])
        );
    }
    for (cost, check) in [("gratis", 0), ("berbayar", 1), ("unknown", 2)] {
        let result = browsing::catalog(
            &pool,
            "trainings",
            &Browse {
                limit: 4,
                cost: cost.into(),
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
        for item in result["items"].as_array().unwrap() {
            match check {
                0 => assert_eq!(item["biaya"], 0),
                1 => assert!(item["biaya"].as_i64().unwrap() > 0),
                _ => assert!(item["biaya"].is_null()),
            }
        }
    }
    let empty = browsing::catalog(
        &pool,
        "trainings",
        &Browse {
            q: "no-such-training-7b5d9f".into(),
            ..Default::default()
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(empty["total"], 0);
    assert!(empty["items"].as_array().unwrap().is_empty());
    let beyond = browsing::catalog(
        &pool,
        "trainings",
        &Browse {
            offset: 99999,
            ..Default::default()
        },
        None,
    )
    .await
    .unwrap();
    assert!(beyond["items"].as_array().unwrap().is_empty());
    assert_eq!(beyond["total"], first["total"]);
    let fixture: Value =
        serde_json::from_slice(&std::fs::read(".cache/test-accounts.json").unwrap()).unwrap();
    let user: Uuid = fixture["accounts"][0]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let jobs = browsing::matches(
        &pool,
        user,
        &Browse {
            limit: 2,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(jobs["items"].as_array().unwrap().len(), 2);
    let id: Uuid = jobs["items"][0]["job"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let job = browsing::job(&pool, user, id).await.unwrap();
    let page = browsing::catalog(
        &pool,
        "trainings",
        &Browse {
            limit: 3,
            recommended: true,
            ..Default::default()
        },
        Some(&job),
    )
    .await
    .unwrap();
    assert!(page["items"].as_array().unwrap().len() <= 3);
    let all: Vec<careerbridge_be::domain::Training> = db::trainings(&pool).await.unwrap();
    let expected = careerbridge_be::domain::rank_trainings(&all, &job);
    assert_eq!(page["total"].as_u64().unwrap() as usize, expected.len());
    for (actual, expected) in page["items"]
        .as_array()
        .unwrap()
        .iter()
        .zip(expected.iter())
    {
        assert_eq!(actual["id"], json!(expected.training.id));
    }
    let key = first["items"][0]["nama"].as_str().unwrap();
    let found = browsing::catalog(
        &pool,
        "trainings",
        &Browse {
            q: key.into(),
            limit: 1,
            ..Default::default()
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(found["items"][0]["nama"], key);
    pool.close().await;
}

struct SseFixtureLlm(String);
#[async_trait]
impl LlmProvider for SseFixtureLlm {
    async fn json(&self, instruction: &str, _data: Value) -> Result<Value, AppError> {
        if instruction.starts_with("Choose") {
            Ok(json!({"priorityIndices":[0]}))
        } else {
            Ok(json!({"pendidikan":["SMK"],"pengalaman":[],"skills":[self.0]}))
        }
    }
    fn model(&self) -> &str {
        "sse-fixture-model"
    }
}
struct DebugCapture(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for DebugCapture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Own ephemeral identity only; local provider fixtures; workers target explicit job IDs.
#[actix_web::test]
#[ignore = "explicit live SSE persistence acceptance"]
async fn live_sse_jobs_survive_disconnect_and_restore_results() {
    if std::env::var("CAREERBRIDGE_LIVE_SSE_TEST").as_deref() != Ok("1") {
        return;
    }
    dotenvy::dotenv().ok();
    use careerbridge_be::presentation::events;
    use futures_util::FutureExt;
    let project = std::env::var("SUPABASE_URL").unwrap();
    let secret = std::env::var("SUPABASE_SECRET_KEY").unwrap();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let admin = |method: reqwest::Method, path: String| {
        let request = client
            .request(method, format!("{project}/auth/v1/admin/{path}"))
            .header("apikey", &secret);
        if secret.starts_with("sb_secret_") {
            request
        } else {
            request.bearer_auth(&secret)
        }
    };
    let email = format!("sse-{}@careerbridge.test", Uuid::new_v4().simple());
    let created = admin(reqwest::Method::POST, "users".into())
        .json(&json!({"email":email,"password":Uuid::new_v4().to_string(),"email_confirm":true,"app_metadata":{"careerbridge_sse_fixture":true}}))
        .send()
        .await
        .unwrap();
    assert!(
        created.status().is_success(),
        "synthetic Auth creation failed: {}",
        created.status()
    );
    let identity: Value = created.json().await.unwrap();
    let user: Uuid = identity["id"].as_str().unwrap().parse().unwrap();
    let outcome=std::panic::AssertUnwindSafe(async {
        let database=careerbridge_be::infrastructure::config::database_url().unwrap();
        let pool=db::connect_runtime_pool(&database,db::RuntimeRole::Api).await.unwrap();
        let worker_pool=db::connect_runtime_pool(&database,db::RuntimeRole::Worker).await.unwrap();
        let known_skill:String=sqlx::query_scalar("SELECT nama FROM skills ORDER BY id LIMIT 1").fetch_one(&pool).await.unwrap();
        let debug=Arc::new(Mutex::new(Vec::<u8>::new()));let writer=debug.clone();
        let subscriber=tracing_subscriber::fmt().json().with_writer(move||DebugCapture(writer.clone())).with_env_filter("careerbridge_be::infrastructure::worker=debug,careerbridge_be::presentation::events=debug").finish();
        let _logging=tracing::subscriber::set_default(subscriber);
        let objects=web::Data::new(Objects{files:Mutex::new(HashMap::new()),delete_failures:AtomicUsize::new(0)});
        let storage_objects=objects.clone();
        let listener=std::net::TcpListener::bind(("127.0.0.1",0)).unwrap();let provider_url=format!("http://{}",listener.local_addr().unwrap());
        let provider=HttpServer::new(move||App::new().app_data(storage_objects.clone())
            .route("/auth/v1/.well-known/jwks.json",web::get().to(||async{HttpResponse::Ok().content_type("application/json").body(include_str!("fixtures/jwks.json"))}))
            .route("/storage/v1/object/cv-private",web::delete().to(storage_delete))
            .service(web::resource("/storage/v1/object/cv-private/{path:.*}").route(web::post().to(storage_put)).route(web::get().to(storage_get))))
            .listen(listener).unwrap().run();let provider_handle=provider.handle();actix_web::rt::spawn(provider);
        let state=web::Data::new(State{events:Default::default(),pool:pool.clone(),worker_pool:worker_pool.clone(),auth:Auth::new(&provider_url,client.clone()),cipher:Cipher::new(&STANDARD.encode([9u8;32])).unwrap(),
            storage:Storage{client:client.clone(),url:provider_url.clone(),key:"test-only".into(),bucket:"cv-private".into()},pdf:Arc::new(FakePdf),embedding:Arc::new(FakeEmbedding),llm:Arc::new(SseFixtureLlm(known_skill))});
        let notifications=events::start_notifications(state.clone().into_inner());
        let server_state=state.clone();let listener=std::net::TcpListener::bind(("127.0.0.1",0)).unwrap();let base=format!("http://{}",listener.local_addr().unwrap());
        let server=HttpServer::new(move||App::new().app_data(server_state.clone()).configure(http::configure)).listen(listener).unwrap().run();let handle=server.handle();actix_web::rt::spawn(server);
        let bearer=token(user,&provider_url);
        let response=client.post(format!("{base}/api/v1/me")).bearer_auth(&bearer).json(&json!({"nama":"Rina Putri","pendidikan":"SMK","jurusan":"Administrasi","provinsi":"Jawa Barat","kota":"Bandung","consent":true})).send().await.unwrap();assert_eq!(response.status(),200);
        let boundary="sse-fixture";
        let bytes=format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"synthetic-sse.pdf\"\r\nContent-Type: application/pdf\r\n\r\n%PDF-1.4\nsynthetic fixture {}\n\r\n--{boundary}--\r\n",Uuid::new_v4());
        let response=client.post(format!("{base}/api/v1/me/cv")).bearer_auth(&bearer).header("content-type",format!("multipart/form-data; boundary={boundary}")).body(bytes).send().await.unwrap();assert_eq!(response.status(),202);
        let accepted:Value=response.json().await.unwrap();let cv:Uuid=accepted["id"].as_str().unwrap().parse().unwrap();
        let queued:Value=client.get(format!("{base}/api/v1/me/cv/latest")).bearer_auth(&bearer).send().await.unwrap().json().await.unwrap();assert_eq!(queued["id"],json!(cv));assert_eq!(queued["status"],"proses");
        let mut subscription=client.get(format!("{base}/api/v1/me/cv/{cv}/events")).bearer_auth(&bearer).send().await.unwrap();assert_eq!(subscription.status(),200);assert!(subscription.headers()["content-type"].to_str().unwrap().starts_with("text/event-stream"));
        let first=subscription.chunk().await.unwrap().unwrap();assert!(String::from_utf8_lossy(&first).contains("event: progress"));drop(subscription);
        // No connected browser: only the selected synthetic job may be processed.
        worker::tick_for(&state,Some(cv)).await.unwrap();
        let latest:Value=client.get(format!("{base}/api/v1/me/cv/latest")).bearer_auth(&bearer).send().await.unwrap().json().await.unwrap();assert_eq!(latest["status"],"selesai");assert!(latest["parsed"].is_object());assert_eq!(latest["reviewed"],false);assert!(latest["durationMs"].is_number());
        let replay=client.get(format!("{base}/api/v1/me/cv/{cv}/events")).bearer_auth(&bearer).header("last-event-id",queued["version"].as_str().unwrap()).send().await.unwrap().text().await.unwrap();assert!(replay.contains("event: completed"));
        let fixtures:Value=serde_json::from_slice(&std::fs::read(".cache/test-accounts.json").unwrap()).unwrap();let other:Uuid=fixtures["accounts"][0]["id"].as_str().unwrap().parse().unwrap();
        let denied=client.get(format!("{base}/api/v1/me/cv/{cv}/events")).bearer_auth(token(other,&provider_url)).send().await.unwrap();assert_eq!(denied.status(),404);
        let occupation:Uuid=sqlx::query_scalar("SELECT id FROM occupations ORDER BY nama LIMIT 1").fetch_one(&pool).await.unwrap();
        let analysis:Value=client.post(format!("{base}/api/v1/me/analysis/{occupation}/jobs")).bearer_auth(&bearer).send().await.unwrap().json().await.unwrap();let job:Uuid=analysis["id"].as_str().unwrap().parse().unwrap();
        let duplicate:Value=client.post(format!("{base}/api/v1/me/analysis/{occupation}/jobs")).bearer_auth(&bearer).send().await.unwrap().json().await.unwrap();assert_eq!(analysis["id"],duplicate["id"]);
        let mut analysis_subscription=client.get(format!("{base}/api/v1/me/analysis-jobs/{job}/events")).bearer_auth(&bearer).send().await.unwrap();assert!(String::from_utf8_lossy(&analysis_subscription.chunk().await.unwrap().unwrap()).contains("progress"));drop(analysis_subscription);
        events::analysis_tick_for(&state,Some(job)).await.unwrap();
        let completed=client.get(format!("{base}/api/v1/me/analysis-jobs/{job}/events")).bearer_auth(&bearer).send().await.unwrap().text().await.unwrap();assert!(completed.contains("event: completed"));assert!(completed.contains("\"match\""));
        let denied=client.get(format!("{base}/api/v1/me/analysis-jobs/{job}/events")).bearer_auth(token(other,&provider_url)).send().await.unwrap();assert_eq!(denied.status(),404);
        let dismissed=client.post(format!("{base}/api/v1/me/cv/{cv}/dismiss")).bearer_auth(&bearer).send().await.unwrap();assert_eq!(dismissed.status(),200);
        let latest:Value=client.get(format!("{base}/api/v1/me/cv/latest")).bearer_auth(&bearer).send().await.unwrap().json().await.unwrap();assert!(latest.is_null());
        let debug_text=String::from_utf8(debug.lock().unwrap().clone()).unwrap();
        assert!(debug_text.contains(&cv.to_string()));
        for stage in ["storage_download","pdf_extraction","llm_extraction","skill_normalization","completed"] {assert!(debug_text.contains(stage));}
        assert!(!debug_text.contains("Rina Putri"));assert!(!debug_text.contains("rina@example"));assert!(!debug_text.contains("synthetic-sse.pdf"));
        notifications.abort();handle.stop(false).await;provider_handle.stop(false).await;drop(state);pool.close().await;worker_pool.close().await;
    }).catch_unwind().await;
    let cleaned = admin(reqwest::Method::DELETE, format!("users/{user}"))
        .send()
        .await
        .unwrap();
    assert!(
        cleaned.status().is_success(),
        "synthetic identity cleanup failed: {}",
        cleaned.status()
    );
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic)
    }
}

#[actix_web::test]
#[ignore = "requires disposable TEST_DATABASE_URL"]
async fn cv_normalization_batches_semantic_work_and_deduplicates_unknowns() {
    struct CountingEmbedding(Arc<Mutex<Vec<usize>>>);
    #[async_trait]
    impl EmbeddingProvider for CountingEmbedding {
        async fn embed(
            &self,
            texts: Vec<String>,
            _: EmbeddingKind,
        ) -> Result<Vec<Vec<f32>>, AppError> {
            self.0.lock().unwrap().push(texts.len());
            Ok(texts
                .iter()
                .map(|_| {
                    let mut v = vec![0.; 384];
                    v[0] = 1.;
                    v
                })
                .collect())
        }
        fn version(&self) -> &str {
            "batch-fixture"
        }
    }
    let (s, owner, _, llm, url, server) = setup().await;
    let calls = Arc::new(Mutex::new(vec![]));
    let client = reqwest::Client::new();
    let state = State {
        events: Default::default(),
        pool: s.pool.clone(),
        worker_pool: s.worker_pool.clone(),
        auth: Auth::new(&url, client.clone()),
        cipher: Cipher::new(&STANDARD.encode([7u8; 32])).unwrap(),
        storage: Storage {
            client,
            url,
            key: "fixture".into(),
            bucket: "fixture".into(),
        },
        pdf: Arc::new(FakePdf),
        llm,
        embedding: Arc::new(CountingEmbedding(calls.clone())),
    };
    let id = Uuid::new_v4();
    let label = format!("Fixture {id}");
    let mut vector = vec![0.; 384];
    vector[0] = 1.;
    sqlx::query("INSERT INTO skills(id,nama,kategori,embedding,embedding_model) VALUES($1,$2,'fixture',$3,'batch-fixture')").bind(id).bind(&label).bind(pgvector::Vector::from(vector)).execute(&owner).await.unwrap();
    let terms: Vec<_> = (0..17)
        .map(|i| format!("fixture batch {id} term {i}"))
        .collect();
    let parsed = careerbridge_be::domain::ParsedCv {
        skills: terms.clone(),
        ..Default::default()
    };
    let result = worker::normalize_cv(&state, &parsed).await.unwrap();
    assert_eq!(*calls.lock().unwrap(), vec![8, 8, 1]);
    assert_eq!(result["matched"].as_array().unwrap().len(), 1);
    assert_eq!(result["matched"][0]["skillId"], id.to_string());
    assert_eq!(result["matched"][0]["sumber"], "cv_semantik");
    assert!(result["unknown"].as_array().unwrap().is_empty());
    sqlx::query("DELETE FROM skills WHERE id=$1")
        .bind(id)
        .execute(&owner)
        .await
        .unwrap();
    let raw = terms[0].clone();
    let result = worker::normalize_cv(
        &state,
        &careerbridge_be::domain::ParsedCv {
            skills: vec![raw.clone(), raw.clone()],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(result["unknown"].as_array().unwrap().len(), 1);
    let key = careerbridge_be::domain::normalize(&raw);
    let count: i32 = sqlx::query_scalar("SELECT jumlah FROM unmatched_skills WHERE teks=$1")
        .bind(&key)
        .fetch_one(&owner)
        .await
        .unwrap();
    assert_eq!(count, 1);
    sqlx::query("DELETE FROM unmatched_skills WHERE teks=$1")
        .bind(key)
        .execute(&owner)
        .await
        .unwrap();
    server.stop(true).await;
}

#[actix_web::test]
#[ignore = "requires disposable TEST_DATABASE_URL"]
async fn admin_http_routes_require_database_role() {
    let (state, owner, _objects, _llm, issuer, provider) = setup().await;
    let app = test::init_service(App::new().app_data(state).configure(http::configure)).await;
    let admin = Uuid::new_v4();
    let user = Uuid::new_v4();
    for (id, role) in [(admin, 2_i16), (user, 1_i16)] {
        sqlx::query("INSERT INTO auth.users(id) VALUES($1)")
            .bind(id)
            .execute(&owner)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users(id,role_id,nama,pendidikan,jurusan,province_id,kota,persetujuan_cv_at) VALUES($1,$2,'HTTP admin fixture','S1','TI',1,'Test',now())")
            .bind(id).bind(role).execute(&owner).await.unwrap();
    }
    let admin_token = token(admin, &issuer);
    let user_token = token(user, &issuer);
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/v1/admin/catalog")
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 401);
    for (method, path, body) in [
        (
            actix_web::http::Method::GET,
            "/api/v1/admin/catalog",
            json!(null),
        ),
        (
            actix_web::http::Method::POST,
            "/api/v1/admin/skills",
            json!({"nama":"Forbidden","kategori":"Test"}),
        ),
        (
            actix_web::http::Method::POST,
            "/api/v1/admin/skills/import",
            json!({"rows":[{"nama":"Forbidden","kategori":"Test"}]}),
        ),
        (
            actix_web::http::Method::DELETE,
            "/api/v1/admin/skills/00000000-0000-0000-0000-000000000000",
            json!(null),
        ),
        (
            actix_web::http::Method::PATCH,
            "/api/v1/admin/vacancies/00000000-0000-0000-0000-000000000000/visibility",
            json!({"hidden":true}),
        ),
    ] {
        let response = test::call_service(
            &app,
            test::TestRequest::default()
                .method(method)
                .uri(path)
                .insert_header(("Authorization", format!("Bearer {user_token}")))
                .set_json(body)
                .to_request(),
        )
        .await;
        assert_eq!(
            response.status(),
            403,
            "ordinary user must be denied on {path}"
        );
    }
    let id = Uuid::new_v4();
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/v1/admin/skills")
            .insert_header(("Authorization", format!("Bearer {admin_token}")))
            .set_json(
                json!({"id":id,"nama":format!("HTTP admin {id}"),"kategori":"Fixture","alias":[]}),
            )
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/v1/admin/catalog")
            .insert_header(("Authorization", format!("Bearer {admin_token}")))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    let body: Value = test::read_body_json(response).await;
    assert!(
        body["skills"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == id.to_string())
    );
    assert!(body.get("users").is_none());
    let job_id = Uuid::new_v4();
    let response=test::call_service(&app,test::TestRequest::post().uri("/api/v1/admin/jobs")
        .insert_header(("Authorization",format!("Bearer {admin_token}")))
        .set_json(json!({"id":job_id,"nama":format!("Mutable occupation {job_id}"),"kodeKbji":"2512","bidang":"Fixture","deskripsi":"Fixture","skills":[{"skillId":id,"tipe":"wajib"}]})).to_request()).await;
    assert_eq!(response.status(), 200);
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/v1/me/trainings?limit=1")
            .insert_header(("Authorization", format!("Bearer {user_token}")))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    let options: Value = test::read_body_json(response).await;
    assert!(
        options["occupationOptions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == job_id.to_string())
    );
    let response = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri(&format!("/api/v1/admin/jobs/{job_id}"))
            .insert_header(("Authorization", format!("Bearer {admin_token}")))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/v1/me/trainings?limit=1")
            .insert_header(("Authorization", format!("Bearer {user_token}")))
            .to_request(),
    )
    .await;
    let options: Value = test::read_body_json(response).await;
    assert!(
        !options["occupationOptions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == job_id.to_string())
    );
    let response = test::call_service(
        &app,
        test::TestRequest::delete()
            .uri(&format!("/api/v1/admin/skills/{id}"))
            .insert_header(("Authorization", format!("Bearer {admin_token}")))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    sqlx::query("UPDATE users SET role_id=1 WHERE id=$1")
        .bind(admin)
        .execute(&owner)
        .await
        .unwrap();
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/v1/admin/catalog")
            .insert_header(("Authorization", format!("Bearer {admin_token}")))
            .to_request(),
    )
    .await;
    assert_eq!(
        response.status(),
        403,
        "an existing JWT must not preserve a revoked admin role"
    );
    sqlx::query("DELETE FROM auth.users WHERE id=ANY($1)")
        .bind(vec![admin, user])
        .execute(&owner)
        .await
        .unwrap();
    provider.stop(true).await;
}
