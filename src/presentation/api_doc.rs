//! OpenAPI is generated from Rust request/domain schemas and handler annotations.
use crate::domain::*;
use chrono::{DateTime, Utc};
use utoipa::{Modify, OpenApi, ToSchema};
use uuid::Uuid;

#[derive(ToSchema)]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
}
#[derive(ToSchema)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}
#[derive(ToSchema)]
pub struct OkResponse {
    pub ok: bool,
}
#[derive(ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkillsSaved {
    pub ok: bool,
    /// Occupation from the confirmed CV's AI suggestion: the user's own occupation or the matching catalog one.
    pub occupation_id: Option<Uuid>,
}
#[derive(ToSchema)]
pub struct Health {
    pub status: String,
}
#[derive(ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Cv {
    pub id: Uuid,
    pub file_name: String,
    pub size: i32,
    pub uploaded_at: DateTime<Utc>,
    pub pendidikan: Vec<String>,
    pub pengalaman: Vec<Experience>,
}
#[derive(ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub nama: String,
    pub pendidikan: String,
    pub jurusan: String,
    pub provinsi: String,
    pub kota: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
    pub skills: Vec<UserSkill>,
    pub target_job_id: Option<Uuid>,
    pub cv: Option<Cv>,
}
#[derive(ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UploadAccepted {
    pub id: Uuid,
    pub status_url: String,
    pub reused: bool,
}
#[derive(ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedSkill {
    pub skill_id: Uuid,
    pub sumber: String,
    pub raw: String,
}
#[derive(ToSchema)]
pub struct NormalizedCv {
    pub matched: Vec<NormalizedSkill>,
    pub unknown: Vec<String>,
}
#[derive(ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum CvStatus {
    Proses,
    Selesai,
    Gagal,
}
#[derive(ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum CatalogKind {
    Skills,
    Occupations,
    Institutions,
    Trainings,
    Provinces,
}
#[derive(ToSchema)]
pub struct UploadStatus {
    pub id: Uuid,
    pub status: CvStatus,
    #[schema(minimum = 0, maximum = 4)]
    pub step: i16,
    pub error: Option<String>,
    pub parsed: Option<ParsedCv>,
    pub normalized: Option<NormalizedCv>,
}
#[derive(ToSchema)]
pub struct PdfUpload {
    /// PDF signature required; maximum 2,097,152 bytes.
    #[schema(value_type = String, format = Binary)]
    pub file: String,
}
#[derive(ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeleteCv {
    pub ok: bool,
    pub cleanup_pending: bool,
}
#[derive(ToSchema)]
pub struct Matches {
    pub items: Vec<JobMatch>,
    pub skills: Vec<Skill>,
    pub fields: Vec<String>,
    pub total: i64,
    pub offset: i64,
    pub limit: i64,
}
#[derive(ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    pub r#match: JobMatch,
    pub trainings: Vec<RankedTraining>,
    pub training_evidence: std::collections::HashMap<String, TrainingEvidence>,
    pub training_retrieval: TrainingRetrieval,
    pub skills: Vec<Skill>,
    pub institutions: Vec<Institution>,
    pub related_occupations: Vec<Occupation>,
    pub related_trainings: Vec<Training>,
    pub related_occupation_ids: Vec<Uuid>,
    pub related_training_ids: Vec<Uuid>,
    pub explanation: String,
    pub priority_skill_ids: Vec<Uuid>,
    pub fallback: bool,
    pub model: String,
    pub revision: String,
}
#[derive(ToSchema)]
#[serde(untagged)]
pub enum CatalogItem {
    Skill(Skill),
    Occupation(Occupation),
    Institution(Institution),
    Training(Training),
    Province(String),
}
#[derive(ToSchema)]
pub struct CatalogPage {
    pub items: Vec<CatalogItem>,
    pub institutions: Option<Vec<Institution>>,
    pub skills: Option<Vec<Skill>>,
    pub total: i64,
    pub offset: i64,
    pub limit: i64,
}

pub struct Security;
impl Modify for Security {
    fn modify(&self, api: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};
        if let Some(components) = api.components.as_mut() {
            components.add_security_scheme(
                "cookieAuth",
                SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new("__Host-cb_session"))),
            );
        }
    }
}
#[derive(OpenApi)]
#[openapi(
    info(title = "CareerBridge Core API", version = "1.0.0", description = "Backend-managed HttpOnly sessions. Login first; cookies are sent automatically. Mutations require an allowed Origin and X-CSRF-Protection: 1. Unknown training fees/modes are null."),
    servers((url = "/")),
    paths(super::admin::catalog, super::admin::save, super::admin::import, super::admin::remove, super::admin::visibility, super::admin::scrape, super::events::latest_cv,super::events::dismiss_cv,super::events::cv_events,super::events::start_analysis,super::events::analysis_events,super::session::login, super::session::register, super::session::session, super::session::logout, super::session::password, super::session::verify, super::http::health, super::http::ready, super::http::me,
        super::http::create_profile, super::http::update_profile, super::http::catalog,
        super::http::save_skills, super::http::target, super::http::upload,
        super::http::cv_status, super::http::delete_cv, super::http::matches, super::http::summary, super::http::training_page, super::http::vacancies, super::http::analysis),
    components(schemas(ErrorResponse, CatalogKind)),
    modifiers(&Security)
)]
pub struct ApiDoc;

#[derive(ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSummary {
    pub items: Vec<JobMatch>,
    pub skills: Vec<Skill>,
    pub free_training_count: i64,
    pub fields: Vec<String>,
    pub total: i64,
    pub offset: i64,
    pub limit: i64,
}
#[derive(ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrainingPage {
    pub items: Vec<Training>,
    pub skills: Vec<Skill>,
    pub institutions: Vec<Institution>,
    pub total: i64,
    pub offset: i64,
    pub limit: i64,
    pub occupation_options: Vec<OccupationOption>,
    pub evidence: std::collections::HashMap<String, TrainingEvidence>,
    pub retrieval: TrainingRetrieval,
}
#[derive(ToSchema)]
pub struct OccupationOption {
    pub id: Uuid,
    pub nama: String,
}

#[derive(ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrainingEvidence {
    /// mapped: verified catalog relationship; text/semantic: topic relevance only.
    pub method: String,
    pub confirmed_skill_ids: Vec<Uuid>,
    pub related_skill_ids: Vec<Uuid>,
    /// Cosine similarity, not a probability or verified coverage score.
    pub similarity: Option<f64>,
    pub source: Option<String>,
    pub source_url: Option<String>,
    pub imported_at: Option<chrono::DateTime<chrono::Utc>>,
}
#[derive(ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrainingRetrieval {
    pub mode: String,
    pub model: String,
    pub minimum_similarity: f64,
    pub catalog_count: i64,
    pub indexed_count: i64,
    pub notice: Option<String>,
    pub sample_data_included: bool,
}
