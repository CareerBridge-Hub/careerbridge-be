//! The master admin UI uses these authenticated catalog mutations.
use super::http::State;
use crate::infrastructure::{admin, auth::Claims, error::AppError};
use actix_web::{HttpResponse, web};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

pub fn configure(c: &mut web::ServiceConfig) {
    c.service(
        web::scope("/admin")
            .app_data(
                web::JsonConfig::default()
                    .limit(2 * 1024 * 1024)
                    .error_handler(|_, _| {
                        AppError::Invalid("JSON admin tidak valid atau melebihi 2 MB.").into()
                    }),
            )
            .route("/catalog", web::get().to(catalog))
            .route("/vacancies/scrape", web::post().to(scrape))
            .route("/vacancies/{id}/visibility", web::patch().to(visibility))
            .route("/{kind}/import", web::post().to(import))
            .route("/{kind}", web::post().to(save))
            .route("/{kind}/{id}", web::delete().to(remove)),
    );
}
#[utoipa::path(get,path="/api/v1/admin/catalog",tag="Admin",security(("cookieAuth"=[])),responses((status=200,description="Admin catalog snapshot, bounded to 10,000 rows per entity; no user CV data",body=Value),(status=403,description="Active administrator required")))]
pub async fn catalog(s: web::Data<State>, a: Claims) -> Result<HttpResponse, AppError> {
    Ok(HttpResponse::Ok().json(admin::snapshot(&s.pool, a.sub).await?))
}
#[utoipa::path(post,path="/api/v1/admin/{kind}",tag="Admin",security(("cookieAuth"=[])),params(("kind"=String,Path,description="skills, jobs, institutions, trainings")),request_body=Value,responses((status=200,description="Created or updated with optional UUID id",body=Value),(status=400,description="Invalid or conflicting catalog data"),(status=403,description="Active administrator required")))]
pub async fn save(
    s: web::Data<State>,
    a: Claims,
    kind: web::Path<String>,
    body: web::Json<Value>,
) -> Result<HttpResponse, AppError> {
    let ids = admin::save(&s.pool, a.sub, &kind, vec![body.into_inner()]).await?;
    Ok(HttpResponse::Ok().json(json!({"id":ids[0]})))
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Import {
    pub rows: Vec<Value>,
}
#[utoipa::path(post,path="/api/v1/admin/{kind}/import",tag="Admin",security(("cookieAuth"=[])),params(("kind"=String,Path,description="skills, jobs, institutions, trainings")),request_body=Import,responses((status=200,description="Atomic import of 1–500 validated rows",body=Value),(status=400,description="Any invalid row rolls back the entire batch"),(status=403,description="Active administrator required")))]
pub async fn import(
    s: web::Data<State>,
    a: Claims,
    kind: web::Path<String>,
    body: web::Json<Import>,
) -> Result<HttpResponse, AppError> {
    let ids = admin::save(&s.pool, a.sub, &kind, body.into_inner().rows).await?;
    Ok(HttpResponse::Ok().json(json!({"ids":ids,"count":ids.len()})))
}
#[utoipa::path(delete,path="/api/v1/admin/{kind}/{id}",tag="Admin",security(("cookieAuth"=[])),params(("kind"=String,Path,description="skills, jobs, institutions, trainings"),("id"=Uuid,Path)),responses((status=200,description="Deleted with dependent catalog references",body=Value),(status=400,description="Required-skill invariant would be violated"),(status=403,description="Active administrator required"),(status=404,description="Not found")))]
pub async fn remove(
    s: web::Data<State>,
    a: Claims,
    path: web::Path<(String, Uuid)>,
) -> Result<HttpResponse, AppError> {
    let (kind, id) = path.into_inner();
    admin::remove(&s.pool, a.sub, &kind, id).await?;
    Ok(HttpResponse::Ok().json(json!({"ok":true})))
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Visibility {
    pub hidden: bool,
}
#[utoipa::path(patch,path="/api/v1/admin/vacancies/{id}/visibility",tag="Admin",security(("cookieAuth"=[])),params(("id"=Uuid,Path)),request_body=Visibility,responses((status=200,description="Visibility saved independently of cron status",body=Value),(status=403,description="Active administrator required"),(status=404,description="Not found")))]
pub async fn visibility(
    s: web::Data<State>,
    a: Claims,
    id: web::Path<Uuid>,
    body: web::Json<Visibility>,
) -> Result<HttpResponse, AppError> {
    admin::visibility(&s.pool, a.sub, *id, body.hidden).await?;
    Ok(HttpResponse::Ok().json(json!({"ok":true})))
}
#[utoipa::path(post,path="/api/v1/admin/vacancies/scrape",tag="Admin",security(("cookieAuth"=[])),responses((status=200,description="Queued non-metered sources (running or recently run/requested ones are skipped), keywords prioritized by user demand, and whether this process runs the scheduler",body=Value),(status=403,description="Active administrator required")))]
pub async fn scrape(s: web::Data<State>, a: Claims) -> Result<HttpResponse, AppError> {
    Ok(HttpResponse::Ok().json(admin::scrape_now(&s.pool, a.sub).await?))
}
