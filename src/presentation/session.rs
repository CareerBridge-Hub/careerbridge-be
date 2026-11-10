//! Backend-for-frontend authentication. Browser cookies never contain provider tokens.
use super::http::State;
use crate::infrastructure::{auth::Claims, error::AppError};
use actix_web::{
    HttpRequest, HttpResponse,
    cookie::{Cookie, SameSite, time::Duration as CookieDuration},
    web,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use utoipa::ToSchema;
use uuid::Uuid;

pub struct CookieAuth {
    pub signup_otp_enabled: bool,
    pub secure: bool,
    pub origins: Vec<String>,
    attempts: Mutex<HashMap<std::net::IpAddr, (Instant, u32)>>,
}
impl CookieAuth {
    pub fn new(secure: bool, origins: Vec<String>) -> Self {
        Self {
            signup_otp_enabled: true,
            secure,
            origins,
            attempts: Mutex::new(HashMap::new()),
        }
    }
    pub fn with_signup_otp(mut self, enabled: bool) -> Self {
        self.signup_otp_enabled = enabled;
        self
    }
    pub fn name(&self) -> &'static str {
        if self.secure {
            "__Host-cb_session"
        } else {
            "cb_session_dev"
        }
    }
    pub fn csrf(&self, req: &HttpRequest) -> Result<(), AppError> {
        let origin = req.headers().get("origin").and_then(|v| v.to_str().ok());
        if !origin.is_some_and(|o| self.origins.iter().any(|allowed| allowed == o))
            || req
                .headers()
                .get("x-csrf-protection")
                .and_then(|v| v.to_str().ok())
                != Some("1")
        {
            return Err(AppError::Forbidden);
        }
        Ok(())
    }
    fn limit(&self, req: &HttpRequest) -> Result<(), AppError> {
        // Socket peer only: do not trust spoofable forwarded headers.
        let ip = req.peer_addr().ok_or(AppError::Busy)?.ip();
        let mut limits = self.attempts.lock().map_err(|_| AppError::Internal)?;
        limits.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(60));
        if limits.len() >= 10_000 && !limits.contains_key(&ip) {
            return Err(AppError::Busy);
        }
        let entry = limits.entry(ip).or_insert((Instant::now(), 0));
        entry.1 += 1;
        if entry.1 > 20 {
            return Err(AppError::Busy);
        }
        Ok(())
    }
    fn cookie(&self, value: String, age: i64) -> Cookie<'static> {
        Cookie::build(self.name(), value)
            .http_only(true)
            .secure(self.secure)
            .same_site(SameSite::Lax)
            .path("/")
            .max_age(CookieDuration::seconds(age))
            .finish()
    }
}
#[derive(Serialize, Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: String,
}
fn secret(req: &HttpRequest, auth: &CookieAuth) -> Result<String, AppError> {
    let value = req
        .cookie(auth.name())
        .ok_or(AppError::Unauthorized)?
        .value()
        .to_owned();
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AppError::Unauthorized);
    }
    Ok(value)
}
fn hash(secret: &str) -> String {
    format!("{:x}", Sha256::digest(secret.as_bytes()))
}
async fn session_tx<'a>(
    state: &'a State,
    secret: &str,
) -> Result<Transaction<'a, Postgres>, AppError> {
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT set_config('app.session_hash',$1,true)")
        .bind(hash(secret))
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}
async fn provider(
    state: &State,
    method: reqwest::Method,
    path: &str,
    body: Value,
    token: Option<&str>,
) -> Result<Value, AppError> {
    let mut request = state
        .storage
        .client
        .request(method, format!("{}/auth/v1/{path}", state.storage.url))
        .header("apikey", &state.storage.key)
        .json(&body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    } else if path == "admin/users" && !state.storage.key.starts_with("sb_secret_") {
        request = request.bearer_auth(&state.storage.key);
    }
    let response = request.send().await.map_err(|_| AppError::Provider)?;
    if !response.status().is_success() {
        return Err(if response.status().as_u16() == 429 {
            AppError::Busy
        } else if response.status().is_server_error() {
            AppError::Provider
        } else {
            AppError::Unauthorized
        });
    }
    // Auth payloads are small; bound upstream responses and never relay raw errors/tokens.
    if response.content_length().is_some_and(|n| n > 64 * 1024) {
        return Err(AppError::Provider);
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| AppError::Provider)?;
        if bytes.len() + chunk.len() > 64 * 1024 {
            return Err(AppError::Provider);
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(&bytes).map_err(|_| AppError::Provider)
}
fn tokens(value: &Value) -> Result<Tokens, AppError> {
    let tokens: Tokens = serde_json::from_value(value.clone()).map_err(|_| AppError::Provider)?;
    if tokens.access_token.len() > 8192 || tokens.refresh_token.len() > 8192 {
        return Err(AppError::Provider);
    }
    Ok(tokens)
}
async fn start(
    state: &State,
    auth: &CookieAuth,
    req: &HttpRequest,
    value: Value,
) -> Result<HttpResponse, AppError> {
    let tokens = tokens(&value)?;
    let claims = state.auth.verify(&tokens.access_token).await?;
    let new_secret = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let mut tx = session_tx(state, &new_secret).await?;
    let encrypted = state
        .cipher
        .encrypt(&serde_json::to_string(&tokens).map_err(|_| AppError::Internal)?)?;
    sqlx::query(
        "INSERT INTO browser_sessions(secret_hash,user_id,encrypted_tokens) VALUES($1,$2,$3)",
    )
    .bind(hash(&new_secret))
    .bind(claims.sub)
    .bind(encrypted)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    // Rotate the browser session on login, preventing fixation.
    if let Ok(old) = secret(req, auth) {
        let mut tx = session_tx(state, &old).await?;
        sqlx::query("DELETE FROM browser_sessions WHERE secret_hash=$1")
            .bind(hash(&old))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }
    Ok(HttpResponse::Ok()
        .insert_header(("Cache-Control", "no-store"))
        .cookie(auth.cookie(new_secret, 7 * 86400))
        .json(json!({"authenticated":true})))
}

/// Serialize refreshes across requests/replicas with a row lock. Never trust cookie claims.
pub async fn resolve(
    state: &State,
    req: &HttpRequest,
    auth: &CookieAuth,
) -> Result<(Claims, Value), AppError> {
    let secret = secret(req, auth)?;
    let mut tx = session_tx(state, &secret).await?;
    // Valid sessions only read: list requests must not queue behind a write lock.
    let row = sqlx::query("SELECT user_id,encrypted_tokens FROM browser_sessions WHERE secret_hash=$1 AND expires_at>now()")
        .bind(hash(&secret)).fetch_optional(&mut *tx).await?.ok_or(AppError::Unauthorized)?;
    let encrypted: String = row.get("encrypted_tokens");
    let plain = state.cipher.decrypt(&encrypted)?;
    let mut tokens: Tokens = serde_json::from_str(&plain).map_err(|_| AppError::Unauthorized)?;
    let claims = match state.auth.verify(&tokens.access_token).await {
        Ok(c) => c,
        Err(AppError::Unauthorized) => {
            // Lock and re-read only for refresh. A concurrent request may already
            // have rotated the tokens while this request waited for the lock.
            let locked=sqlx::query("SELECT user_id,encrypted_tokens FROM browser_sessions WHERE secret_hash=$1 AND expires_at>now() FOR UPDATE")
                .bind(hash(&secret)).fetch_optional(&mut *tx).await?.ok_or(AppError::Unauthorized)?;
            let plain = state
                .cipher
                .decrypt(&locked.get::<String, _>("encrypted_tokens"))?;
            tokens = serde_json::from_str(&plain).map_err(|_| AppError::Unauthorized)?;
            match state.auth.verify(&tokens.access_token).await {
                Ok(claims) => {
                    if claims.sub != locked.get::<Uuid, _>("user_id") {
                        return Err(AppError::Unauthorized);
                    }
                    tx.commit().await?;
                    return Ok((claims, json!({"access_token":tokens.access_token})));
                }
                Err(AppError::Unauthorized) => {}
                Err(error) => return Err(error),
            }
            let value = match provider(
                state,
                reqwest::Method::POST,
                "token?grant_type=refresh_token",
                json!({"refresh_token":tokens.refresh_token}),
                None,
            )
            .await
            {
                Ok(value) => value,
                Err(AppError::Unauthorized) => {
                    sqlx::query("DELETE FROM browser_sessions WHERE secret_hash=$1")
                        .bind(hash(&secret))
                        .execute(&mut *tx)
                        .await?;
                    tx.commit().await?;
                    return Err(AppError::Unauthorized);
                }
                Err(error) => return Err(error),
            };
            tokens = self::tokens(&value)?;
            let claims = state.auth.verify(&tokens.access_token).await?;
            if claims.sub != row.get::<Uuid, _>("user_id") {
                return Err(AppError::Unauthorized);
            }
            let encrypted = state
                .cipher
                .encrypt(&serde_json::to_string(&tokens).map_err(|_| AppError::Internal)?)?;
            sqlx::query("UPDATE browser_sessions SET encrypted_tokens=$1 WHERE secret_hash=$2")
                .bind(encrypted)
                .bind(hash(&secret))
                .execute(&mut *tx)
                .await?;
            claims
        }
        Err(e) => return Err(e),
    };
    if claims.sub != row.get::<Uuid, _>("user_id") {
        return Err(AppError::Unauthorized);
    }
    tx.commit().await?;
    // Metadata is obtained from the provider, never accepted as an authorization role.
    Ok((claims, json!({"access_token":tokens.access_token})))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    email: String,
    password: String,
}
impl Credentials {
    fn validate(&self) -> Result<(), AppError> {
        if self.email.len() > 254
            || !self.email.contains('@')
            || self.password.is_empty()
            || self.password.len() > 1024
        {
            return Err(AppError::Invalid("Email atau password tidak valid."));
        }
        Ok(())
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Signup {
    email: String,
    password: String,
    profile: super::http::ProfileInput,
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PasswordChange {
    current_password: String,
    password: String,
}

#[utoipa::path(post, path="/api/v1/auth/login", request_body=Credentials, responses((status=200, description="HttpOnly session cookie"),(status=401, description="Invalid credentials"),(status=403, description="CSRF rejected")), tag="Authentication")]
async fn login(
    req: HttpRequest,
    state: web::Data<State>,
    auth: web::Data<CookieAuth>,
    input: web::Json<Credentials>,
) -> Result<HttpResponse, AppError> {
    auth.csrf(&req)?;
    auth.limit(&req)?;
    input.validate()?;
    let value = provider(
        &state,
        reqwest::Method::POST,
        "token?grant_type=password",
        json!({"email":input.email.trim().to_lowercase(),"password":input.password}),
        None,
    )
    .await?;
    start(&state, &auth, &req, value).await
}
#[utoipa::path(post, path="/api/v1/auth/register", request_body=Signup, responses((status=200, description="Session created or email confirmation required")), tag="Authentication")]
async fn register(
    req: HttpRequest,
    state: web::Data<State>,
    auth: web::Data<CookieAuth>,
    input: web::Json<Signup>,
) -> Result<HttpResponse, AppError> {
    auth.csrf(&req)?;
    auth.limit(&req)?;
    Credentials {
        email: input.email.clone(),
        password: input.password.clone(),
    }
    .validate()?;
    if input.password.chars().count() < 12 || !input.profile.consent {
        return Err(AppError::Invalid(
            "Password minimal 12 karakter dan persetujuan wajib.",
        ));
    }
    input.profile.validate()?;
    let value = signup_session(&state, &input, auth.signup_otp_enabled).await?;
    if value.get("access_token").and_then(Value::as_str).is_some() {
        start(&state, &auth, &req, value).await
    } else {
        Ok(HttpResponse::Ok()
            .insert_header(("Cache-Control", "no-store"))
            .json(json!({"authenticated":false,"confirmationRequired":true})))
    }
}
async fn signup_session(
    state: &State,
    input: &Signup,
    otp_enabled: bool,
) -> Result<Value, AppError> {
    let email = input.email.trim().to_lowercase();
    if otp_enabled {
        return provider(
            state,
            reqwest::Method::POST,
            "signup",
            json!({"email":email,"password":input.password,"data":input.profile}),
            None,
        )
        .await;
    }
    // Create only: a duplicate must never confirm, adopt, or reset an existing account.
    // This intentionally skips email ownership verification for new test signups.
    provider(state,reqwest::Method::POST,"admin/users",json!({"email":email,"password":input.password,"email_confirm":true,"user_metadata":input.profile}),None).await?;
    provider(
        state,
        reqwest::Method::POST,
        "token?grant_type=password",
        json!({"email":email,"password":input.password}),
        None,
    )
    .await
}
#[utoipa::path(get, path="/api/v1/auth/session", responses((status=200, description="Current authenticated identity; no tokens"),(status=401, description="No session")), tag="Authentication")]
async fn session(
    req: HttpRequest,
    state: web::Data<State>,
    auth: web::Data<CookieAuth>,
) -> Result<HttpResponse, AppError> {
    let (claims, tokens) = resolve(&state, &req, &auth).await?;
    let user = provider(
        &state,
        reqwest::Method::GET,
        "user",
        Value::Null,
        tokens["access_token"].as_str(),
    )
    .await?;
    Ok(HttpResponse::Ok().insert_header(("Cache-Control","no-store")).json(json!({"user":{"id":claims.sub,"user_metadata":user.get("user_metadata").filter(|v|v.is_object()).cloned().unwrap_or(json!({}))}})))
}
#[utoipa::path(post, path="/api/v1/auth/logout", responses((status=200, description="Session revoked and cookie expired")), tag="Authentication")]
async fn logout(
    req: HttpRequest,
    state: web::Data<State>,
    auth: web::Data<CookieAuth>,
) -> Result<HttpResponse, AppError> {
    auth.csrf(&req)?;
    if let Ok(secret) = secret(&req, &auth) {
        let mut tx = session_tx(&state, &secret).await?;
        let encrypted: Option<String> = sqlx::query_scalar(
            "DELETE FROM browser_sessions WHERE secret_hash=$1 RETURNING encrypted_tokens",
        )
        .bind(hash(&secret))
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?; // Local revocation must succeed even if the provider is unavailable.
        if let Some(encrypted) = encrypted
            && let Ok(plain) = state.cipher.decrypt(&encrypted)
            && let Ok(tokens) = serde_json::from_str::<Tokens>(&plain)
        {
            let _ = provider(
                &state,
                reqwest::Method::POST,
                "logout?scope=local",
                json!({}),
                Some(&tokens.access_token),
            )
            .await;
        }
    }
    Ok(HttpResponse::Ok()
        .insert_header(("Cache-Control", "no-store"))
        .cookie(auth.cookie(String::new(), 0))
        .json(json!({"ok":true})))
}
#[utoipa::path(post, path="/api/v1/auth/password", request_body=PasswordChange, responses((status=200, description="Password changed")), tag="Authentication")]
async fn password(
    req: HttpRequest,
    state: web::Data<State>,
    auth: web::Data<CookieAuth>,
    input: web::Json<PasswordChange>,
) -> Result<HttpResponse, AppError> {
    auth.csrf(&req)?;
    auth.limit(&req)?;
    if input.password.chars().count() < 12
        || input.password.len() > 1024
        || input.current_password.len() > 1024
    {
        return Err(AppError::Invalid("Password minimal 12 karakter."));
    }
    let (claims, tokens) = resolve(&state, &req, &auth).await?;
    let checked = provider(
        &state,
        reqwest::Method::POST,
        "token?grant_type=password",
        json!({"email":claims.email,"password":input.current_password}),
        None,
    )
    .await?;
    let check = self::tokens(&checked)?;
    let verified = state.auth.verify(&check.access_token).await?;
    if verified.sub != claims.sub {
        return Err(AppError::Unauthorized);
    }
    let result = provider(
        &state,
        reqwest::Method::PUT,
        "user",
        json!({"password":input.password}),
        tokens["access_token"].as_str(),
    )
    .await;
    let _ = provider(
        &state,
        reqwest::Method::POST,
        "logout?scope=local",
        json!({}),
        Some(&check.access_token),
    )
    .await;
    result?;
    let mut tx = crate::infrastructure::db::user_tx(&state.pool, claims.sub).await?;
    sqlx::query("DELETE FROM browser_sessions WHERE user_id=$1")
        .bind(claims.sub)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(HttpResponse::Ok()
        .insert_header(("Cache-Control", "no-store"))
        .cookie(auth.cookie(String::new(), 0))
        .json(json!({"ok":true})))
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Verification {
    email: String,
    token: String,
}
#[utoipa::path(post, path="/api/v1/auth/verify", request_body=Verification, responses((status=200, description="Email verified and HttpOnly session created")), tag="Authentication")]
async fn verify(
    req: HttpRequest,
    state: web::Data<State>,
    auth: web::Data<CookieAuth>,
    input: web::Json<Verification>,
) -> Result<HttpResponse, AppError> {
    auth.csrf(&req)?;
    if !auth.signup_otp_enabled {
        return Err(AppError::Forbidden);
    }
    auth.limit(&req)?;
    if input.email.len() > 254
        || !input.email.contains('@')
        || !(6..=10).contains(&input.token.len())
        || !input.token.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(AppError::Invalid("Kode konfirmasi tidak valid."));
    }
    let value = provider(
        &state,
        reqwest::Method::POST,
        "verify",
        json!({"type":"signup","email":input.email.trim().to_lowercase(),"token":input.token}),
        None,
    )
    .await?;
    start(&state, &auth, &req, value).await
}
pub fn configure(c: &mut web::ServiceConfig) {
    c.service(
        web::scope("/auth")
            .route("/verify", web::post().to(verify))
            .route("/login", web::post().to(login))
            .route("/register", web::post().to(register))
            .route("/session", web::get().to(session))
            .route("/logout", web::post().to(logout))
            .route("/password", web::post().to(password)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::test::TestRequest;
    #[test]
    fn csrf_requires_exact_origin_and_custom_header_even_before_login() {
        let auth = CookieAuth::new(true, vec!["https://app.example.test".into()]);
        for origin in [
            None,
            Some("null"),
            Some("https://evil.test"),
            Some("https://app.example.test.evil.test"),
        ] {
            let mut req = TestRequest::post().insert_header(("x-csrf-protection", "1"));
            if let Some(origin) = origin {
                req = req.insert_header(("origin", origin));
            }
            assert!(matches!(
                auth.csrf(&req.to_http_request()),
                Err(AppError::Forbidden)
            ));
        }
        assert!(
            auth.csrf(
                &TestRequest::post()
                    .insert_header(("origin", "https://app.example.test"))
                    .to_http_request()
            )
            .is_err()
        );
        assert!(
            auth.csrf(
                &TestRequest::post()
                    .insert_header(("origin", "https://app.example.test"))
                    .insert_header(("x-csrf-protection", "1"))
                    .to_http_request()
            )
            .is_ok()
        );
    }
    #[test]
    fn production_cookie_is_host_only_secure_http_only_and_expiration_matches_deletion() {
        let auth = CookieAuth::new(true, vec![]);
        let cookie = auth.cookie("opaque-secret".into(), 604800);
        assert_eq!(cookie.name(), "__Host-cb_session");
        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.secure(), Some(true));
        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
        assert_eq!(cookie.path(), Some("/"));
        assert!(cookie.domain().is_none());
        let expired = auth.cookie(String::new(), 0);
        assert_eq!(expired.name(), cookie.name());
        assert_eq!(expired.path(), cookie.path());
        assert_eq!(expired.max_age().unwrap().whole_seconds(), 0);
        assert!(
            secret(
                &TestRequest::default()
                    .cookie(Cookie::new(auth.name(), "forged"))
                    .to_http_request(),
                &auth
            )
            .is_err()
        );
    }
    #[test]
    fn login_throttles_socket_peers_and_does_not_trust_forwarded_ips() {
        let auth = CookieAuth::new(true, vec![]);
        for i in 0..20 {
            let request = TestRequest::post()
                .peer_addr("127.0.0.1:1234".parse().unwrap())
                .insert_header(("x-forwarded-for", format!("192.0.2.{i}")))
                .to_http_request();
            assert!(auth.limit(&request).is_ok());
        }
        assert!(matches!(
            auth.limit(
                &TestRequest::post()
                    .peer_addr("127.0.0.1:9999".parse().unwrap())
                    .to_http_request()
            ),
            Err(AppError::Busy)
        ));
    }
    struct UnusedProvider;
    #[async_trait::async_trait]
    impl crate::application::CvTextExtractor for UnusedProvider {
        async fn extract(&self, _: &[u8]) -> Result<zeroize::Zeroizing<String>, AppError> {
            unreachable!()
        }
    }
    #[async_trait::async_trait]
    impl crate::application::EmbeddingProvider for UnusedProvider {
        async fn embed(
            &self,
            _: Vec<String>,
            _: crate::application::EmbeddingKind,
        ) -> Result<Vec<Vec<f32>>, AppError> {
            unreachable!()
        }
        fn version(&self) -> &str {
            "unused"
        }
    }
    #[async_trait::async_trait]
    impl crate::application::LlmProvider for UnusedProvider {
        async fn json(&self, _: &str, _: Value) -> Result<Value, AppError> {
            unreachable!()
        }
        fn model(&self) -> &str {
            "unused"
        }
    }
    #[actix_web::test]
    async fn signup_modes_use_correct_provider_calls_and_never_adopt_duplicates() {
        use actix_web::{App, HttpServer};
        use std::sync::Arc;
        for (enabled, duplicate) in [(true, false), (false, false), (false, true)] {
            let calls = web::Data::new(Mutex::new(Vec::<(String, Value, bool)>::new()));
            let captured = calls.clone();
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server=HttpServer::new(move||App::new().app_data(captured.clone()).default_service(web::to(move|req:HttpRequest,body:web::Json<Value>,calls:web::Data<Mutex<Vec<(String,Value,bool)>>>|async move {
                assert_eq!(req.headers().get("apikey").unwrap(),"sb_secret_test_only");
                calls.lock().unwrap().push((req.path().to_owned(),body.into_inner(),req.headers().contains_key("authorization")));
                if duplicate { HttpResponse::UnprocessableEntity().json(json!({"message":"existing user"})) }
                else { HttpResponse::Ok().json(json!({"access_token":"fixture-access","refresh_token":"fixture-refresh"})) }
            }))).listen(listener).unwrap().run();
            let handle = server.handle();
            actix_web::rt::spawn(server);
            let pool = sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgresql://unused:unused@127.0.0.1:1/unused")
                .unwrap();
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap();
            let state = State {
                events: Default::default(),
                pool: pool.clone(),
                worker_pool: pool,
                auth: crate::infrastructure::auth::Auth::new(&url, client.clone()),
                cipher: crate::infrastructure::crypto::Cipher::new(
                    "CQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk=",
                )
                .unwrap(),
                storage: crate::infrastructure::storage::Storage {
                    client,
                    url,
                    key: "sb_secret_test_only".into(),
                    bucket: "unused".into(),
                },
                pdf: Arc::new(UnusedProvider),
                embedding: Arc::new(UnusedProvider),
                llm: Arc::new(UnusedProvider),
            };
            let signup:Signup=serde_json::from_value(json!({"email":"NEW@example.test","password":"private-test-password","profile":{"nama":"Test","pendidikan":"S1","jurusan":"TI","provinsi":"Banten","kota":"Serang","consent":true}})).unwrap();
            let result = signup_session(&state, &signup, enabled).await;
            let calls = calls.lock().unwrap().clone();
            if enabled {
                assert!(result.is_ok());
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].0, "/auth/v1/signup");
                assert_eq!(calls[0].1["data"]["nama"], "Test");
                assert!(calls[0].1.get("email_confirm").is_none());
            } else {
                assert_eq!(calls[0].0, "/auth/v1/admin/users");
                assert_eq!(calls[0].1["email_confirm"], true);
                assert_eq!(calls[0].1["user_metadata"]["nama"], "Test");
                assert_eq!(calls[0].1["email"], "new@example.test");
                assert!(
                    !calls[0].2,
                    "modern secret must not be sent as a bearer JWT"
                );
                if duplicate {
                    assert!(matches!(result, Err(AppError::Unauthorized)));
                    assert_eq!(calls.len(), 1);
                } else {
                    assert!(result.is_ok());
                    assert_eq!(calls.len(), 2);
                    assert_eq!(calls[1].0, "/auth/v1/token");
                }
            }
            let auth = web::Data::new(
                CookieAuth::new(true, vec!["https://app.example.test".into()])
                    .with_signup_otp(false),
            );
            let response = verify(
                TestRequest::post()
                    .insert_header(("origin", "https://app.example.test"))
                    .insert_header(("x-csrf-protection", "1"))
                    .to_http_request(),
                web::Data::new(state),
                auth,
                web::Json(Verification {
                    email: "new@example.test".into(),
                    token: "123456".into(),
                }),
            )
            .await;
            assert!(matches!(response, Err(AppError::Forbidden)));
            handle.stop(true).await;
        }
    }
}
