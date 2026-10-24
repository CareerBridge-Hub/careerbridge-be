use crate::{
    application::{EmbeddingKind, EmbeddingProvider, LlmProvider},
    domain::*,
    infrastructure::{
        auth::{Auth, Claims},
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
use uuid::Uuid;

pub struct State {
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
    c.route("/health", web::get().to(health))
        .route("/ready", web::get().to(ready))
        .service(
            web::scope("/api/v1")
                .route("/openapi.json", web::get().to(openapi))
                .route("/me", web::get().to(me))
                .route("/me", web::post().to(create_profile))
                .route("/me", web::patch().to(update_profile))
                .route("/catalog/{kind}", web::get().to(catalog))
                .route("/me/skills", web::put().to(save_skills))
                .route("/me/target", web::put().to(target))
                .route("/me/cv", web::post().to(upload))
                .route("/me/cv", web::delete().to(delete_cv))
                .route("/me/cv/{id}", web::get().to(cv_status))
                .route("/me/matches", web::get().to(matches))
                .route("/me/analysis/{id}", web::get().to(analysis)),
        );
}
async fn health() -> impl Responder {
    web::Json(json!({"status":"ok"}))
}
async fn ready(s: web::Data<State>) -> Result<web::Json<Value>, AppError> {
    sqlx::query("SELECT 1").execute(&s.pool).await?;
    Ok(web::Json(json!({"status":"ready"})))
}
async fn openapi() -> impl Responder {
    HttpResponse::Ok()
        .content_type("application/json")
        .body(include_str!("../../docs/openapi.json"))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileInput {
    nama: String,
    pendidikan: String,
    jurusan: String,
    provinsi: String,
    kota: String,
    #[serde(default)]
    consent: bool,
}
impl ProfileInput {
    fn validate(&self) -> Result<(), AppError> {
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
#[derive(Deserialize)]
struct Page {
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    limit: usize,
}
fn default_limit() -> usize {
    100
}
async fn catalog(
    s: web::Data<State>,
    a: Claims,
    kind: web::Path<String>,
    page: web::Query<Page>,
) -> Result<web::Json<Value>, AppError> {
    let tx = db::active_tx(&s.pool, a.sub).await?;
    tx.commit().await?;
    if page.limit == 0 || page.limit > 100 || page.offset > i64::MAX as usize {
        return Err(AppError::Invalid("Parameter halaman tidak valid."));
    }
    Ok(web::Json(
        db::catalog_page(&s.pool, &kind, page.offset as i64, page.limit as i64).await?,
    ))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SkillsInput {
    skills: Vec<UserSkill>,
    cv_id: Option<Uuid>,
}
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
    let known: HashSet<_> = db::skills(&s.pool)
        .await?
        .into_iter()
        .map(|s| s.id)
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
    db::bump(&mut tx, a.sub).await?;
    tx.commit().await?;
    Ok(web::Json(json!({"ok":true})))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TargetInput {
    occupation_id: Option<Uuid>,
}
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
    Ok(HttpResponse::Accepted()
        .json(json!({"id":id,"statusUrl":format!("/api/v1/me/cv/{id}"),"reused":false})))
}
async fn cv_status(
    s: web::Data<State>,
    a: Claims,
    id: web::Path<Uuid>,
) -> Result<web::Json<Value>, AppError> {
    let mut tx = db::active_tx(&s.pool, a.sub).await?;
    let row =
        sqlx::query("SELECT * FROM cv_uploads WHERE id=$1 AND user_id=$2 AND deleted_at IS NULL")
            .bind(*id)
            .bind(a.sub)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(AppError::NotFound)?;
    tx.commit().await?;
    let parsed = if let Some(encrypted) = row.get::<Option<String>, _>("parsed_json") {
        serde_json::from_str::<Value>(&s.cipher.decrypt(&encrypted)?)
            .map_err(|_| AppError::Internal)?
    } else {
        Value::Null
    };
    Ok(web::Json(
        json!({"id":*id,"status":row.get::<String,_>("status"),"step":row.get::<i16,_>("step"),"error":row.get::<Option<String>,_>("pesan_error"),"parsed":parsed,"normalized":row.get::<Option<Value>,_>("normalized_json")}),
    ))
}
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
async fn get_matches(s: &State, a: &Claims) -> Result<Vec<JobMatch>, AppError> {
    let mut tx = db::active_tx(&s.pool, a.sub).await?;
    let ids: HashSet<Uuid> =
        sqlx::query_scalar("SELECT skill_id FROM user_skills WHERE user_id=$1")
            .bind(a.sub)
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .collect();
    tx.commit().await?;
    let mut m: Vec<_> = db::occupations(&s.pool)
        .await?
        .iter()
        .map(|o| match_job(o, &ids))
        .collect();
    m.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.job.nama.cmp(&b.job.nama))
    });
    Ok(m)
}
async fn matches(s: web::Data<State>, a: Claims) -> Result<web::Json<Value>, AppError> {
    Ok(web::Json(json!({"items":get_matches(&s,&a).await?})))
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
async fn analysis(
    s: web::Data<State>,
    a: Claims,
    id: web::Path<Uuid>,
) -> Result<web::Json<Value>, AppError> {
    let mut tx = db::active_tx(&s.pool, a.sub).await?;
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM users WHERE id=$1")
        .bind(a.sub)
        .fetch_one(&mut *tx)
        .await?;
    let catalog_revision: i64 = sqlx::query_scalar("SELECT revision FROM catalog_revision")
        .fetch_one(&mut *tx)
        .await?;
    let key = format!(
        "{revision}:{catalog_revision}:{}:{}:analysis-v2",
        s.embedding.version(),
        s.llm.model()
    );
    let cached: Option<Value> = sqlx::query_scalar(
        "SELECT result FROM analyses WHERE user_id=$1 AND occupation_id=$2 AND revision_key=$3",
    )
    .bind(a.sub)
    .bind(*id)
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
        return Ok(web::Json(result));
    }
    let m = get_matches(&s, &a)
        .await?
        .into_iter()
        .find(|m| m.job.id == *id)
        .ok_or(AppError::NotFound)?;
    let skills = db::skills(&s.pool).await?;
    let trainings = db::trainings(&s.pool).await?;
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
        related_jobs = similar(&s, "occupations", &vectors[0], vec![m.job.id]).await?;
    }
    if !gap_text.is_empty() {
        let vectors = s
            .embedding
            .embed(vec![gap_text], EmbeddingKind::Query)
            .await?;
        related_trainings = similar(
            &s,
            "trainings",
            &vectors[0],
            ranked.iter().map(|r| r.training.id).collect(),
        )
        .await?;
    }
    let result = json!({"match":m,"trainings":ranked,"relatedOccupationIds":related_jobs,"relatedTrainingIds":related_trainings,"explanation":explanation(&m,&skills,&plan.priority_skill_ids),"prioritySkillIds":plan.priority_skill_ids,"fallback":fallback,"model":s.llm.model(),"revision":key});
    let mut tx = db::active_tx(&s.pool, a.sub).await?;
    sqlx::query("INSERT INTO analyses(user_id,occupation_id,revision_key,result) SELECT $1,$2,$3,$4 WHERE EXISTS(SELECT 1 FROM users WHERE id=$1 AND revision=$5) AND EXISTS(SELECT 1 FROM catalog_revision WHERE revision=$6) ON CONFLICT(user_id,occupation_id) DO UPDATE SET revision_key=excluded.revision_key,result=excluded.result,created_at=now()").bind(a.sub).bind(*id).bind(key).bind(&result).bind(revision).bind(catalog_revision).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(web::Json(result))
}
