use super::api_doc::*;
use crate::{
    application::{EmbeddingKind, EmbeddingProvider, LlmProvider},
    domain::*,
    infrastructure::{
        auth::{Auth, Claims},
        browsing::{self, Browse},
        crypto::Cipher,
        db,
        error::AppError,
        storage::Storage,
    },
};
use actix_multipart::Multipart;
use actix_web::{HttpResponse, Responder, web};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::{collections::HashSet, sync::Arc};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

pub struct State {
    pub events: super::events::JobEvents,
    pub pdf: Arc<dyn crate::application::CvTextExtractor>,
    pub pool: PgPool,
    pub worker_pool: PgPool,
    pub auth: Auth,
    pub cipher: Cipher,
    pub storage: Storage,
    pub embedding: Arc<dyn EmbeddingProvider>,
    pub llm: Arc<dyn LlmProvider>,
}
pub fn configure(c: &mut web::ServiceConfig) {
    c.app_data(
        web::PathConfig::default()
            .error_handler(|_, _| AppError::Invalid("ID tidak valid.").into()),
    )
    .app_data(
        web::QueryConfig::default()
            .error_handler(|_, _| AppError::Invalid("Parameter tidak valid.").into()),
    )
    .app_data(
        web::JsonConfig::default()
            .limit(64 * 1024)
            .error_handler(|_, _| AppError::Invalid("JSON tidak valid.").into()),
    );
    c.route("/swagger-ui", web::get().to(swagger_redirect))
        .route("/swagger-ui/", web::get().to(swagger_ui))
        .route("/swagger-ui/{asset}", web::get().to(swagger_asset))
        .route("/health", web::get().to(health))
        .route("/ready", web::get().to(ready))
        .service(
            web::scope("/api/v1")
                .configure(super::session::configure)
                .route("/openapi.json", web::get().to(openapi))
                .route("/me", web::get().to(me))
                .route("/me", web::post().to(create_profile))
                .route("/me", web::patch().to(update_profile))
                .route("/catalog/{kind}", web::get().to(catalog))
                .route("/me/skills", web::put().to(save_skills))
                .route("/me/target", web::put().to(target))
                .route("/me/cv", web::post().to(upload))
                .route("/me/cv", web::delete().to(delete_cv))
                .route("/me/cv/latest", web::get().to(super::events::latest_cv))
                .route(
                    "/me/cv/{id}/events",
                    web::get().to(super::events::cv_events),
                )
                .route(
                    "/me/cv/{id}/dismiss",
                    web::post().to(super::events::dismiss_cv),
                )
                .route(
                    "/me/analysis/{id}/jobs",
                    web::post().to(super::events::start_analysis),
                )
                .route(
                    "/me/analysis-jobs/{id}/events",
                    web::get().to(super::events::analysis_events),
                )
                .route("/me/cv/{id}", web::get().to(cv_status))
                .route("/me/matches", web::get().to(matches))
                .route("/me/summary", web::get().to(summary))
                .route("/me/trainings", web::get().to(training_page))
                .route("/me/analysis/{id}", web::get().to(analysis)),
        );
}
async fn swagger_redirect() -> HttpResponse {
    HttpResponse::TemporaryRedirect()
        .insert_header(("Location", "/swagger-ui/"))
        .finish()
}
async fn swagger_ui() -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .insert_header(("X-Content-Type-Options", "nosniff"))
        .insert_header(("Content-Security-Policy", "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'"))
        .body(include_str!("../../docs/swagger-ui/index.html"))
}
async fn swagger_asset(asset: web::Path<String>) -> HttpResponse {
    let (content_type, body): (&str, &'static str) = match asset.as_str() {
        "swagger-ui-bundle.js" => (
            "application/javascript",
            include_str!("../../docs/swagger-ui/swagger-ui-bundle.js"),
        ),
        "swagger-ui.css" => (
            "text/css",
            include_str!("../../docs/swagger-ui/swagger-ui.css"),
        ),
        "initializer.js" => (
            "application/javascript",
            include_str!("../../docs/swagger-ui/initializer.js"),
        ),
        _ => return HttpResponse::NotFound().finish(),
    };
    HttpResponse::Ok()
        .content_type(content_type)
        .insert_header(("X-Content-Type-Options", "nosniff"))
        .body(body)
}
#[utoipa::path(get, path = "/health", responses((status = 200, description = "Process liveness", body = Health)))]
async fn health() -> impl Responder {
    web::Json(json!({"status":"ok"}))
}
#[utoipa::path(get, path = "/ready", responses((status = 200, description = "Database readiness", body = Health), (status = 500, description = "Database unavailable", body = ErrorResponse)))]
async fn ready(s: web::Data<State>) -> Result<web::Json<Value>, AppError> {
    sqlx::query("SELECT 1").execute(&s.pool).await?;
    Ok(web::Json(json!({"status":"ready"})))
}
async fn openapi() -> impl Responder {
    HttpResponse::Ok()
        .content_type("application/json")
        .json(ApiDoc::openapi())
}
#[derive(serde::Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileInput {
    #[schema(min_length = 1, max_length = 100)]
    nama: String,
    #[schema(value_type = Education)]
    pendidikan: String,
    #[schema(max_length = 100)]
    jurusan: String,
    provinsi: String,
    #[schema(min_length = 1, max_length = 80)]
    kota: String,
    /// Must be true when provisioning a profile (POST); optional for PATCH.
    #[serde(default)]
    pub(super) consent: bool,
}
impl ProfileInput {
    pub(super) fn validate(&self) -> Result<(), AppError> {
        if self.nama.trim().is_empty()
            || self.nama.chars().count() > 100
            || !matches!(
                self.pendidikan.as_str(),
                "SMA" | "SMK" | "D3" | "D4" | "S1" | "S2"
            )
            || self.jurusan.chars().count() > 100
            || self.kota.trim().is_empty()
            || self.kota.chars().count() > 80
        {
            return Err(AppError::Invalid("Lengkapi data profil yang valid."));
        }
        Ok(())
    }
}
#[utoipa::path(post, summary = "Provision authenticated profile; consent must be true", path = "/api/v1/me", responses((status = 200, description = "Success", body = User), (status = 400, description = "Invalid input", body = ErrorResponse),
    (status = 401, description = "Invalid or expired session", body = ErrorResponse),
    (status = 404, description = "Missing or inaccessible resource", body = ErrorResponse),
    (status = 429, description = "Queue full; retry with backoff", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
    (status = 502, description = "Provider unavailable", body = ErrorResponse)), request_body = ProfileInput, security(("cookieAuth" = [])))]
async fn create_profile(
    s: web::Data<State>,
    a: Claims,
    input: web::Json<ProfileInput>,
) -> Result<web::Json<Value>, AppError> {
    input.validate()?;
    if !input.consent {
        return Err(AppError::Invalid(
            "Persetujuan pemrosesan CV wajib dicentang.",
        ));
    }
    let mut tx = db::user_tx(&s.pool, a.sub).await?;
    let province: Option<i16> = sqlx::query_scalar("SELECT id FROM provinces WHERE nama=$1")
        .bind(&input.provinsi)
        .fetch_optional(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO users(id,nama,pendidikan,jurusan,province_id,kota,persetujuan_cv_at) VALUES($1,$2,$3,$4,$5,$6,now()) ON CONFLICT(id) DO NOTHING").bind(a.sub).bind(input.nama.trim()).bind(&input.pendidikan).bind(input.jurusan.trim()).bind(province.ok_or(AppError::Invalid("Provinsi tidak valid."))?).bind(input.kota.trim()).execute(&mut *tx).await?;
    tx.commit().await?;
    me(s, a).await
}
#[utoipa::path(patch, summary = "Update all profile fields", path = "/api/v1/me", responses((status = 200, description = "Success", body = User), (status = 400, description = "Invalid input", body = ErrorResponse),
    (status = 401, description = "Invalid or expired session", body = ErrorResponse),
    (status = 404, description = "Missing or inaccessible resource", body = ErrorResponse),
    (status = 429, description = "Queue full; retry with backoff", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
    (status = 502, description = "Provider unavailable", body = ErrorResponse)), request_body = ProfileInput, security(("cookieAuth" = [])))]
async fn update_profile(
    s: web::Data<State>,
    a: Claims,
    input: web::Json<ProfileInput>,
) -> Result<web::Json<Value>, AppError> {
    input.validate()?;
    let mut tx = db::active_tx(&s.pool, a.sub).await?;
    let province: Option<i16> = sqlx::query_scalar("SELECT id FROM provinces WHERE nama=$1")
        .bind(&input.provinsi)
        .fetch_optional(&mut *tx)
        .await?;
    sqlx::query("UPDATE users SET nama=$2,pendidikan=$3,jurusan=$4,province_id=$5,kota=$6,updated_at=now() WHERE id=$1").bind(a.sub).bind(input.nama.trim()).bind(&input.pendidikan).bind(input.jurusan.trim()).bind(province.ok_or(AppError::Invalid("Provinsi tidak valid."))?).bind(input.kota.trim()).execute(&mut *tx).await?;
    tx.commit().await?;
    me(s, a).await
}
#[utoipa::path(get, summary = "Current profile; 404 means provisioning is required", path = "/api/v1/me", responses((status = 200, description = "Success", body = User), (status = 400, description = "Invalid input", body = ErrorResponse),
    (status = 401, description = "Invalid or expired session", body = ErrorResponse),
    (status = 404, description = "Missing or inaccessible resource", body = ErrorResponse),
    (status = 429, description = "Queue full; retry with backoff", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
    (status = 502, description = "Provider unavailable", body = ErrorResponse)), security(("cookieAuth" = [])))]
async fn me(s: web::Data<State>, a: Claims) -> Result<web::Json<Value>, AppError> {
    let mut tx = db::user_tx(&s.pool, a.sub).await?;
    let row=sqlx::query("SELECT u.*,p.nama province,r.kode role FROM users u JOIN provinces p ON p.id=u.province_id JOIN roles r ON r.id=u.role_id WHERE u.id=$1 AND u.aktif").bind(a.sub).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    let skills: Vec<Value> = sqlx::query(
        "SELECT skill_id,sumber FROM user_skills WHERE user_id=$1 ORDER BY skill_id",
    )
    .bind(a.sub)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|r| json!({"skillId":r.get::<Uuid,_>("skill_id"),"sumber":r.get::<String,_>("sumber")}))
    .collect();
    let cv=sqlx::query("SELECT * FROM cv_uploads WHERE user_id=$1 AND id=(SELECT current_cv_id FROM users WHERE id=$1) AND status='selesai' AND deleted_at IS NULL").bind(a.sub).fetch_optional(&mut *tx).await?;
    let cv = if let Some(cv) = cv {
        let parsed: ParsedCv =
            serde_json::from_str(&s.cipher.decrypt(&cv.get::<String, _>("parsed_json"))?)
                .map_err(|_| AppError::Internal)?;
        json!({"id":cv.get::<Uuid,_>("id"),"fileName":cv.get::<String,_>("file_name"),"size":cv.get::<i32,_>("ukuran_byte"),"uploadedAt":cv.get::<chrono::DateTime<chrono::Utc>,_>("created_at"),"pendidikan":parsed.pendidikan,"pengalaman":parsed.pengalaman})
    } else {
        Value::Null
    };
    tx.commit().await?;
    Ok(web::Json(
        json!({"id":a.sub,"email":a.email,"nama":row.get::<String,_>("nama"),"pendidikan":row.get::<String,_>("pendidikan"),"jurusan":row.get::<String,_>("jurusan"),"provinsi":row.get::<String,_>("province"),"kota":row.get::<String,_>("kota"),"role":row.get::<String,_>("role"),"createdAt":row.get::<chrono::DateTime<chrono::Utc>,_>("created_at"),"skills":skills,"targetJobId":row.get::<Option<Uuid>,_>("target_occupation_id"),"cv":cv}),
    ))
}
#[utoipa::path(get, summary = "Paginated catalog; occupations are not vacancy postings", path = "/api/v1/catalog/{kind}", responses((status = 200, description = "Success", body = CatalogPage), (status = 400, description = "Invalid input", body = ErrorResponse),
    (status = 401, description = "Invalid or expired session", body = ErrorResponse),
    (status = 404, description = "Missing or inaccessible resource", body = ErrorResponse),
    (status = 429, description = "Queue full; retry with backoff", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
    (status = 502, description = "Provider unavailable", body = ErrorResponse)), params(("kind" = CatalogKind, Path), Browse), security(("cookieAuth" = [])))]
async fn catalog(
    s: web::Data<State>,
    a: Claims,
    kind: web::Path<String>,
    page: web::Query<Browse>,
) -> Result<web::Json<Value>, AppError> {
    page.validate()?;
    let tx = db::active_tx(&s.pool, a.sub).await?;
    tx.commit().await?;
    Ok(web::Json(
        browsing::catalog(&s.pool, &kind, &page, None).await?,
    ))
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillsInput {
    skills: Vec<UserSkill>,
    cv_id: Option<Uuid>,
}
#[utoipa::path(put, summary = "Replace confirmed skills; server validates CV provenance", path = "/api/v1/me/skills", responses((status = 200, description = "Success", body = OkResponse), (status = 400, description = "Invalid input", body = ErrorResponse),
    (status = 401, description = "Invalid or expired session", body = ErrorResponse),
    (status = 404, description = "Missing or inaccessible resource", body = ErrorResponse),
    (status = 429, description = "Queue full; retry with backoff", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
    (status = 502, description = "Provider unavailable", body = ErrorResponse)), request_body = SkillsInput, security(("cookieAuth" = [])))]
async fn save_skills(
    s: web::Data<State>,
    a: Claims,
    input: web::Json<SkillsInput>,
) -> Result<web::Json<Value>, AppError> {
    if input.skills.len() > 100 {
        return Err(AppError::Invalid("Maksimal 100 skill."));
    }
    let mut tx = db::active_tx(&s.pool, a.sub).await?;
    // Serialize mutations for this user; provenance can only come from server-held CV results.
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(a.sub)
        .execute(&mut *tx)
        .await?;
    let prior = sqlx::query("SELECT skill_id,sumber FROM user_skills WHERE user_id=$1")
        .bind(a.sub)
        .fetch_all(&mut *tx)
        .await?;
    let normalized: Option<Value> = if let Some(id) = input.cv_id {
        Some(sqlx::query_scalar("SELECT normalized_json FROM cv_uploads WHERE id=$1 AND user_id=$2 AND status='selesai' AND deleted_at IS NULL").bind(id).bind(a.sub).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?)
    } else {
        None
    };
    let ids: Vec<Uuid> = input.skills.iter().map(|s| s.skill_id).collect();
    let known: HashSet<Uuid> = sqlx::query_scalar("SELECT id FROM skills WHERE id=ANY($1)")
        .bind(ids)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect();
    let mut seen = HashSet::new();
    for skill in &input.skills {
        if !known.contains(&skill.skill_id)
            || !seen.insert(skill.skill_id)
            || !matches!(
                skill.sumber.as_str(),
                "manual" | "cv_alias" | "cv_semantik" | "cv"
            )
        {
            return Err(AppError::Invalid("Daftar skill tidak valid."));
        }
    }
    sqlx::query("DELETE FROM user_skills WHERE user_id=$1")
        .bind(a.sub)
        .execute(&mut *tx)
        .await?;
    for skill in &input.skills {
        let mut source = "manual".to_owned();
        if skill.sumber != "manual" {
            if let Some(norm) = &normalized {
                if let Some(found) = norm["matched"].as_array().and_then(|list| {
                    list.iter()
                        .find(|v| v["skillId"] == skill.skill_id.to_string())
                }) {
                    source = found["sumber"].as_str().unwrap_or("manual").to_owned();
                }
            } else if let Some(old) = prior
                .iter()
                .find(|r| r.get::<Uuid, _>("skill_id") == skill.skill_id)
            {
                source = old.get("sumber");
            }
        }
        sqlx::query("INSERT INTO user_skills(user_id,skill_id,sumber) VALUES($1,$2,$3)")
            .bind(a.sub)
            .bind(skill.skill_id)
            .bind(source)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(cv_id) = input.cv_id {
        sqlx::query("UPDATE users SET current_cv_id=$2 WHERE id=$1")
            .bind(a.sub)
            .bind(cv_id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(id) = input.cv_id {
        sqlx::query("UPDATE cv_uploads SET reviewed_at=now() WHERE id=$1 AND user_id=$2")
            .bind(id)
            .bind(a.sub)
            .execute(&mut *tx)
            .await?;
    }
    db::bump(&mut tx, a.sub).await?;
    tx.commit().await?;
    Ok(web::Json(json!({"ok":true})))
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetInput {
    occupation_id: Option<Uuid>,
}
#[utoipa::path(put, summary = "Set or clear target occupation", path = "/api/v1/me/target", responses((status = 200, description = "Success", body = OkResponse), (status = 400, description = "Invalid input", body = ErrorResponse),
    (status = 401, description = "Invalid or expired session", body = ErrorResponse),
    (status = 404, description = "Missing or inaccessible resource", body = ErrorResponse),
    (status = 429, description = "Queue full; retry with backoff", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
    (status = 502, description = "Provider unavailable", body = ErrorResponse)), request_body = TargetInput, security(("cookieAuth" = [])))]
async fn target(
    s: web::Data<State>,
    a: Claims,
    input: web::Json<TargetInput>,
) -> Result<web::Json<Value>, AppError> {
    let mut tx = db::active_tx(&s.pool, a.sub).await?;
    if let Some(id) = input.occupation_id {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM occupations WHERE id=$1)")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        if !exists {
            return Err(AppError::NotFound);
        }
    }
    sqlx::query("UPDATE users SET target_occupation_id=$2 WHERE id=$1")
        .bind(a.sub)
        .bind(input.occupation_id)
        .execute(&mut *tx)
        .await?;
    db::bump(&mut tx, a.sub).await?;
    tx.commit().await?;
    Ok(web::Json(json!({"ok":true})))
}
#[utoipa::path(post, summary = "Queue PDF up to 2 MiB; reuse same-user completed results", path = "/api/v1/me/cv", responses((status = 202, description = "Success", body = UploadAccepted), (status = 400, description = "Invalid input", body = ErrorResponse),
    (status = 401, description = "Invalid or expired session", body = ErrorResponse),
    (status = 404, description = "Missing or inaccessible resource", body = ErrorResponse),
    (status = 429, description = "Queue full; retry with backoff", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
    (status = 502, description = "Provider unavailable", body = ErrorResponse)), request_body(content = PdfUpload, content_type = "multipart/form-data"), security(("cookieAuth" = [])))]
async fn upload(
    s: web::Data<State>,
    a: Claims,
    form: Result<Multipart, actix_web::Error>,
) -> Result<HttpResponse, AppError> {
    let mut tx = db::active_tx(&s.pool, a.sub).await?;
    let mut form = form.map_err(|_| AppError::Invalid("Unggahan tidak valid."))?;
    let mut bytes = vec![];
    let mut name = "cv.pdf".to_owned();
    let mut files = 0;
    while let Some(field) = tokio::time::timeout(std::time::Duration::from_secs(15), form.next())
        .await
        .map_err(|_| AppError::Invalid("Unggahan terlalu lambat."))?
    {
        let mut field = field.map_err(|_| AppError::Invalid("Unggahan tidak valid."))?;
        files += 1;
        if files > 1 || field.name() != Some("file") {
            return Err(AppError::Invalid("Unggah satu file PDF."));
        }
        if let Some(filename) = field.content_disposition().and_then(|d| d.get_filename()) {
            name = filename
                .chars()
                .filter(|c| !c.is_control() && *c != '/' && *c != '\\')
                .take(255)
                .collect();
        }
        while let Some(chunk) =
            tokio::time::timeout(std::time::Duration::from_secs(10), field.next())
                .await
                .map_err(|_| AppError::Invalid("Unggahan terlalu lambat."))?
        {
            let chunk = chunk.map_err(|_| AppError::Invalid("Unggahan tidak valid."))?;
            if bytes.len() + chunk.len() > 2097152 {
                return Err(AppError::Invalid("Ukuran file maksimal 2 MB."));
            }
            bytes.extend_from_slice(&chunk);
        }
    }
    if !bytes.starts_with(b"%PDF-") {
        return Err(AppError::Invalid("Isi file bukan PDF yang valid."));
    }
    let hash = format!("{:x}", Sha256::digest(&bytes));
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(a.sub)
        .execute(&mut *tx)
        .await?;
    let old:Option<Uuid>=sqlx::query_scalar("SELECT id FROM cv_uploads WHERE user_id=$1 AND file_hash=$2 AND deleted_at IS NULL AND status IN ('proses','selesai')").bind(a.sub).bind(&hash).fetch_optional(&mut *tx).await?;
    if let Some(id) = old {
        tx.commit().await?;
        tracing::debug!(cv_id=%id,stage="reused","CV extraction queued");
        return Ok(HttpResponse::Accepted()
            .json(json!({"id":id,"statusUrl":format!("/api/v1/me/cv/{id}"),"reused":true})));
    }
    let recent: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM cv_uploads WHERE user_id=$1 AND created_at>now()-interval '1 hour'",
    )
    .bind(a.sub)
    .fetch_one(&mut *tx)
    .await?;
    if recent >= 10 {
        return Err(AppError::Busy);
    }
    let id = Uuid::new_v4();
    let path = format!("{}/{id}", a.sub);
    let size = bytes.len() as i32;
    // Persist path before upload, so a failed commit cannot orphan a successfully stored object.
    sqlx::query("INSERT INTO cv_uploads(id,user_id,file_name,ukuran_byte,file_hash,file_path,status,lease_until,pesan_error) VALUES($1,$2,$3,$4,$5,$6,'gagal',now()+interval '3 minutes','Unggahan terputus. Unggah ulang CV.')").bind(id).bind(a.sub).bind(name).bind(size).bind(hash).bind(&path).execute(&mut *tx).await?;
    tx.commit().await?;
    s.storage.put(&path, bytes).await?;
    let mut tx = db::active_tx(&s.pool, a.sub).await?;
    sqlx::query(
        "UPDATE cv_uploads SET status='proses',lease_until=NULL,pesan_error=NULL WHERE id=$1 AND deleted_at IS NULL",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    tracing::debug!(cv_id=%id,stage="queued","CV extraction queued");
    Ok(HttpResponse::Accepted()
        .json(json!({"id":id,"statusUrl":format!("/api/v1/me/cv/{id}"),"reused":false})))
}
#[utoipa::path(get, summary = "Read own upload progress and decrypted result", path = "/api/v1/me/cv/{id}", responses((status = 200, description = "Success", body = UploadStatus), (status = 400, description = "Invalid input", body = ErrorResponse),
    (status = 401, description = "Invalid or expired session", body = ErrorResponse),
    (status = 404, description = "Missing or inaccessible resource", body = ErrorResponse),
    (status = 429, description = "Queue full; retry with backoff", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
    (status = 502, description = "Provider unavailable", body = ErrorResponse)), params(("id" = Uuid, Path)), security(("cookieAuth" = [])))]
async fn cv_status(
    s: web::Data<State>,
    a: Claims,
    id: web::Path<Uuid>,
) -> Result<web::Json<Value>, AppError> {
    Ok(web::Json(super::events::cv_snapshot(&s, a.sub, *id).await?))
}
#[utoipa::path(delete, summary = "Erase CV; queue Storage cleanup; retain confirmed skills", path = "/api/v1/me/cv", responses((status = 200, description = "Success", body = DeleteCv), (status = 400, description = "Invalid input", body = ErrorResponse),
    (status = 401, description = "Invalid or expired session", body = ErrorResponse),
    (status = 404, description = "Missing or inaccessible resource", body = ErrorResponse),
    (status = 429, description = "Queue full; retry with backoff", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
    (status = 502, description = "Provider unavailable", body = ErrorResponse)), security(("cookieAuth" = [])))]
async fn delete_cv(s: web::Data<State>, a: Claims) -> Result<web::Json<Value>, AppError> {
    let mut tx = db::active_tx(&s.pool, a.sub).await?;
    sqlx::query("UPDATE cv_uploads SET deleted_at=now(),parsed_json=NULL,normalized_json=NULL,status='gagal',pesan_error=NULL WHERE user_id=$1 AND deleted_at IS NULL").bind(a.sub).execute(&mut *tx).await?;
    sqlx::query("UPDATE users SET current_cv_id=NULL WHERE id=$1")
        .bind(a.sub)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(web::Json(json!({"ok":true,"cleanupPending":true})))
}
#[utoipa::path(get, summary = "Weighted skill coverage rankings", path = "/api/v1/me/matches", responses((status = 200, description = "Success", body = Matches), (status = 400, description = "Invalid input", body = ErrorResponse),
    (status = 401, description = "Invalid or expired session", body = ErrorResponse),
    (status = 404, description = "Missing or inaccessible resource", body = ErrorResponse),
    (status = 429, description = "Queue full; retry with backoff", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
    (status = 502, description = "Provider unavailable", body = ErrorResponse)), params(Browse), security(("cookieAuth" = [])))]
async fn matches(
    s: web::Data<State>,
    a: Claims,
    page: web::Query<Browse>,
) -> Result<web::Json<Value>, AppError> {
    Ok(web::Json(browsing::matches(&s.pool, a.sub, &page).await?))
}
#[utoipa::path(get,path="/api/v1/me/trainings",summary="One page of filtered training cards with required institutions and skills",params(Browse),responses((status=200,description="Bounded training page",body=TrainingPage)),security(("cookieAuth"=[])))]
async fn training_page(
    s: web::Data<State>,
    a: Claims,
    page: web::Query<Browse>,
) -> Result<web::Json<Value>, AppError> {
    page.validate()?;
    let tx = db::active_tx(&s.pool, a.sub).await?;
    tx.commit().await?;
    let m = if page.recommended {
        if let Some(id) = page.occupation_id {
            Some(browsing::job(&s.pool, a.sub, id).await?)
        } else {
            None
        }
    } else {
        None
    };
    if page.recommended && m.is_none() {
        return Err(AppError::Invalid("Pilih pekerjaan untuk rekomendasi."));
    }
    Ok(web::Json(
        browsing::catalog(&s.pool, "trainings", &page, m.as_ref()).await?,
    ))
}
#[utoipa::path(get,path="/api/v1/me/summary",summary="Bounded dashboard summary, target, skill labels and training count",responses((status=200,description="At most six top matches plus selected target",body=DashboardSummary)),security(("cookieAuth"=[])))]
async fn summary(s: web::Data<State>, a: Claims) -> Result<web::Json<Value>, AppError> {
    let mut value = browsing::matches(
        &s.pool,
        a.sub,
        &Browse {
            limit: 6,
            ..Default::default()
        },
    )
    .await?;
    let mut tx = db::active_tx(&s.pool, a.sub).await?;
    let target: Option<Uuid> =
        sqlx::query_scalar("SELECT target_occupation_id FROM users WHERE id=$1")
            .bind(a.sub)
            .fetch_one(&mut *tx)
            .await?;
    let own: Vec<Uuid> = sqlx::query_scalar("SELECT skill_id FROM user_skills WHERE user_id=$1")
        .bind(a.sub)
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;
    let mut items: Vec<JobMatch> =
        serde_json::from_value(value["items"].clone()).map_err(|_| AppError::Internal)?;
    if let Some(id) = target
        && !items.iter().any(|m| m.job.id == id)
    {
        items.push(browsing::job(&s.pool, a.sub, id).await?);
    }
    let focus = target
        .and_then(|id| items.iter().find(|m| m.job.id == id))
        .or(items.first());
    // Count without hydrating any training cards.
    let missing: Vec<Uuid> = focus
        .into_iter()
        .flat_map(|m| {
            m.missing_wajib
                .iter()
                .chain(&m.missing_opsional)
                .map(|j| j.skill_id)
        })
        .collect();
    let free:i64=sqlx::query_scalar("SELECT count(*) FROM trainings t WHERE t.aktif AND t.biaya=0 AND EXISTS(SELECT 1 FROM training_skills ts WHERE ts.training_id=t.id AND ts.skill_id=ANY($1))").bind(missing).fetch_one(&s.pool).await?;
    let ids: Vec<Uuid> = items
        .iter()
        .flat_map(|m| m.job.skills.iter().map(|j| j.skill_id))
        .chain(own)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    value["skills"] = json!(browsing::by_ids(&s.pool, "skills", &ids).await?);
    value["items"] = json!(items);
    value["freeTrainingCount"] = json!(free);
    Ok(web::Json(value))
}
async fn similar(
    s: &State,
    table: &str,
    vector: &[f32],
    exclude: Vec<Uuid>,
) -> Result<Vec<Uuid>, AppError> {
    // Table names originate exclusively from static calls below.
    let active = if table == "trainings" {
        "AND actif"
    } else {
        ""
    };
    let active = active.replace("actif", "aktif");
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT id FROM {table} WHERE embedding IS NOT NULL AND embedding_model=$2 AND NOT(id=ANY($3)) {active} ORDER BY embedding <=> $1,id LIMIT 5"))).bind(pgvector::Vector::from(vector.to_vec())).bind(s.embedding.version()).bind(exclude).fetch_all(&s.pool).await?)
}
#[utoipa::path(get, summary = "Cached gaps, training recommendations, and validated explanation", path = "/api/v1/me/analysis/{id}", responses((status = 200, description = "Success", body = Analysis), (status = 400, description = "Invalid input", body = ErrorResponse),
    (status = 401, description = "Invalid or expired session", body = ErrorResponse),
    (status = 404, description = "Missing or inaccessible resource", body = ErrorResponse),
    (status = 429, description = "Queue full; retry with backoff", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
    (status = 502, description = "Provider unavailable", body = ErrorResponse)), params(("id" = Uuid, Path)), security(("cookieAuth" = [])))]
async fn analysis(
    s: web::Data<State>,
    a: Claims,
    id: web::Path<Uuid>,
) -> Result<web::Json<Value>, AppError> {
    Ok(web::Json(compute_analysis(&s, a, *id).await?))
}
pub(crate) fn analysis_key(s: &State, revision: i64, catalog_revision: i64) -> String {
    format!(
        "{revision}:{catalog_revision}:{}:{}:analysis-v3-paged",
        s.embedding.version(),
        s.llm.model()
    )
}
pub(crate) async fn compute_analysis(s: &State, a: Claims, id: Uuid) -> Result<Value, AppError> {
    let mut tx = db::active_tx(&s.pool, a.sub).await?;
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM users WHERE id=$1")
        .bind(a.sub)
        .fetch_one(&mut *tx)
        .await?;
    let catalog_revision: i64 = sqlx::query_scalar("SELECT revision FROM catalog_revision")
        .fetch_one(&mut *tx)
        .await?;
    let key = analysis_key(s, revision, catalog_revision);
    let cached: Option<Value> = sqlx::query_scalar(
        "SELECT result FROM analyses WHERE user_id=$1 AND occupation_id=$2 AND revision_key=$3",
    )
    .bind(a.sub)
    .bind(id)
    .bind(&key)
    .fetch_optional(&mut *tx)
    .await?;
    let profile_skill_ids: HashSet<Uuid> =
        sqlx::query_scalar("SELECT skill_id FROM user_skills WHERE user_id=$1")
            .bind(a.sub)
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .collect();
    tx.commit().await?;
    if let Some(result) = cached {
        return Ok(result);
    }
    let m = browsing::job(&s.pool, a.sub, id).await?;
    let skill_ids: Vec<Uuid> = m
        .job
        .skills
        .iter()
        .map(|j| j.skill_id)
        .chain(profile_skill_ids.iter().copied())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let skills: Vec<Skill> = browsing::by_ids(&s.pool, "skills", &skill_ids)
        .await?
        .into_iter()
        .map(|v| serde_json::from_value(v).map_err(|_| AppError::Internal))
        .collect::<Result<_, _>>()?;
    let page = browsing::catalog(
        &s.pool,
        "trainings",
        &Browse {
            limit: 3,
            recommended: true,
            ..Default::default()
        },
        Some(&m),
    )
    .await?;
    let trainings: Vec<Training> =
        serde_json::from_value(page["items"].clone()).map_err(|_| AppError::Internal)?;
    let ranked = rank_trainings(&trainings, &m);
    let missing: Vec<_> = m
        .missing_wajib
        .iter()
        .chain(&m.missing_opsional)
        .filter_map(|j| skills.iter().find(|s| s.id == j.skill_id))
        .collect();
    let mut plan = ExplanationPlan {
        priority_skill_ids: missing.iter().take(5).map(|s| s.id).collect(),
    };
    let mut fallback = true;
    // The LLM receives names and local indices, never user identifiers or database UUIDs.
    let context = json!({
      "occupation":m.job.nama,"score":m.score,
      "ownedSkills":skills.iter().filter(|s|profile_skill_ids.contains(&s.id)).map(|s|s.nama.as_str()).collect::<Vec<_>>(),
      "missingSkills":missing.iter().enumerate().map(|(index,s)|json!({"index":index,"name":s.nama,"required":m.missing_wajib.iter().any(|j|j.skill_id==s.id)})).collect::<Vec<_>>(),
      "trainings":ranked.iter().take(10).map(|r|json!({"name":r.training.nama,"missingSkillsCovered":r.covers_wajib.iter().chain(&r.covers_opsional).filter_map(|id|skills.iter().find(|s|s.id==*id).map(|s|s.nama.as_str())).collect::<Vec<_>>()})).collect::<Vec<_>>()
    });
    if !missing.is_empty() {
        for _ in 0..2 {
            if let Ok(v)=s.llm.json("Choose at most 5 priority missing skills, prioritizing required skills and the supplied training coverage. Return JSON {\"priorityIndices\":[0,1]}; use only supplied indices, no prose.",context.clone()).await && let Some(indices)=v["priorityIndices"].as_array(){let selected:Option<Vec<Uuid>>=indices.iter().map(|v|v.as_u64().and_then(|i|missing.get(i as usize).map(|s|s.id))).collect();if let Some(selected)=selected{let candidate=ExplanationPlan{priority_skill_ids:selected};if !candidate.priority_skill_ids.is_empty()&&validate_explanation(&candidate,&m){plan=candidate;fallback=false;break}}}
        }
    }
    let profile_text = skills
        .iter()
        .filter(|s| profile_skill_ids.contains(&s.id))
        .map(|s| s.nama.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let gap_text = missing
        .iter()
        .map(|s| s.nama.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let mut related_jobs = vec![];
    let mut related_trainings = vec![];
    if !profile_text.is_empty() {
        let vectors = s
            .embedding
            .embed(vec![profile_text], EmbeddingKind::Query)
            .await?;
        related_jobs = similar(s, "occupations", &vectors[0], vec![m.job.id]).await?;
    }
    if !gap_text.is_empty() {
        let vectors = s
            .embedding
            .embed(vec![gap_text], EmbeddingKind::Query)
            .await?;
        related_trainings = similar(
            s,
            "trainings",
            &vectors[0],
            ranked.iter().map(|r| r.training.id).collect(),
        )
        .await?;
    }
    let related_occupations = browsing::by_ids(&s.pool, "occupations", &related_jobs).await?;
    let related_training_items = browsing::by_ids(&s.pool, "trainings", &related_trainings).await?;
    let institution_ids: Vec<Uuid> = ranked
        .iter()
        .map(|r| r.training.institution_id)
        .chain(
            related_training_items
                .iter()
                .filter_map(|v| v["institutionId"].as_str()?.parse().ok()),
        )
        .collect();
    let institutions = browsing::by_ids(&s.pool, "institutions", &institution_ids).await?;
    let extra_ids: Vec<Uuid> = ranked
        .iter()
        .flat_map(|r| r.training.skill_ids.iter().copied())
        .chain(
            related_training_items
                .iter()
                .flat_map(|t| t["skillIds"].as_array().into_iter().flatten())
                .filter_map(|v| v.as_str()?.parse().ok()),
        )
        .chain(skills.iter().map(|s| s.id))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let card_skills = browsing::by_ids(&s.pool, "skills", &extra_ids).await?;
    let result = json!({"match":m,"skills":card_skills,"institutions":institutions,"relatedOccupations":related_occupations,"relatedTrainings":related_training_items,"trainings":ranked,"relatedOccupationIds":related_jobs,"relatedTrainingIds":related_trainings,"explanation":explanation(&m,&skills,&plan.priority_skill_ids),"prioritySkillIds":plan.priority_skill_ids,"fallback":fallback,"model":s.llm.model(),"revision":key});
    let mut tx = db::active_tx(&s.pool, a.sub).await?;
    sqlx::query("INSERT INTO analyses(user_id,occupation_id,revision_key,result) SELECT $1,$2,$3,$4 WHERE EXISTS(SELECT 1 FROM users WHERE id=$1 AND revision=$5) AND EXISTS(SELECT 1 FROM catalog_revision WHERE revision=$6) ON CONFLICT(user_id,occupation_id) DO UPDATE SET revision_key=excluded.revision_key,result=excluded.result,created_at=now()").bind(a.sub).bind(id).bind(key).bind(&result).bind(revision).bind(catalog_revision).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(result)
}
