//! Durable jobs, bounded authenticated SSE, and cross-process PostgreSQL wakeups.
use super::http::{self, State};
use crate::infrastructure::{auth::Claims, db, error::AppError};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast};
use tracing::Instrument;
use uuid::Uuid;

#[derive(Clone, Deserialize)]
struct Notice {
    kind: String,
    id: Uuid,
}
pub struct JobEvents {
    tx: broadcast::Sender<Notice>,
    capacity: Arc<Semaphore>,
    users: Mutex<HashMap<Uuid, Weak<Semaphore>>>,
}
impl Default for JobEvents {
    fn default() -> Self {
        Self {
            tx: broadcast::channel(256).0,
            capacity: Arc::new(Semaphore::new(64)),
            users: Mutex::new(HashMap::new()),
        }
    }
}
impl JobEvents {
    fn reserve(
        &self,
        user: Uuid,
    ) -> Result<(OwnedSemaphorePermit, OwnedSemaphorePermit), AppError> {
        let global = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| AppError::Busy)?;
        let mut users = self.users.lock().map_err(|_| AppError::Internal)?;
        users.retain(|_, v| v.strong_count() > 0);
        let capacity = users.get(&user).and_then(Weak::upgrade).unwrap_or_else(|| {
            let value = Arc::new(Semaphore::new(2));
            users.insert(user, Arc::downgrade(&value));
            value
        });
        let local = capacity.try_acquire_owned().map_err(|_| AppError::Busy)?;
        Ok((global, local))
    }
}
pub fn start_notifications(state: Arc<State>) -> tokio::task::JoinHandle<()> {
    let listener_state = state.clone();
    tokio::spawn(async move {
        loop {
            if let Err(_error) = listen(&listener_state).await {
                tracing::warn!(
                    kind = "job_notifications",
                    "job notification connection interrupted; retrying"
                );
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    })
}
pub fn start_background(state: Arc<State>) {
    start_notifications(state.clone());
    tokio::spawn(async move {
        loop {
            if let Err(error) = analysis_tick(&state).await {
                tracing::warn!(error=%error,"analysis worker tick failed");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}
async fn listen(state: &State) -> Result<(), sqlx::Error> {
    let mut listener = sqlx::postgres::PgListener::connect_with(&state.pool).await?;
    listener.listen("careerbridge_jobs").await?;
    loop {
        let message = listener.recv().await?;
        if let Ok(notice) = serde_json::from_str::<Notice>(message.payload()) {
            let _ = state.events.tx.send(notice);
        }
    }
}
pub(crate) async fn cv_snapshot(state: &State, user: Uuid, id: Uuid) -> Result<Value, AppError> {
    let mut tx = db::active_tx(&state.pool, user).await?;
    let row =
        sqlx::query("SELECT * FROM cv_uploads WHERE id=$1 AND user_id=$2 AND deleted_at IS NULL")
            .bind(id)
            .bind(user)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(AppError::NotFound)?;
    tx.commit().await?;
    let status: String = row.get("status");
    let step: i16 = row.get("step");
    let updated: chrono::DateTime<chrono::Utc> = row.get("updated_at");
    let parsed = if let Some(encrypted) = row.get::<Option<String>, _>("parsed_json") {
        serde_json::from_str::<Value>(&state.cipher.decrypt(&encrypted)?)
            .map_err(|_| AppError::Internal)?
    } else {
        Value::Null
    };
    Ok(
        json!({"id":id,"status":status,"step":step,"version":format!("{}:{}:{status}:{step}",updated.timestamp_micros(),row.get::<i16,_>("attempts")),"fileName":row.get::<String,_>("file_name"),"size":row.get::<i32,_>("ukuran_byte"),"createdAt":row.get::<chrono::DateTime<chrono::Utc>,_>("created_at"),"durationMs":row.get::<Option<i64>,_>("processing_ms"),"reviewed":row.get::<Option<chrono::DateTime<chrono::Utc>>,_>("reviewed_at").is_some(),"error":row.get::<Option<String>,_>("pesan_error"),"parsed":parsed,"normalized":row.get::<Option<Value>,_>("normalized_json")}),
    )
}
#[utoipa::path(get,path="/api/v1/me/cv/latest",responses((status=200,description="Latest unreviewed job, or null")),security(("cookieAuth"=[])))]
pub async fn latest_cv(
    state: web::Data<State>,
    claims: Claims,
) -> Result<web::Json<Value>, AppError> {
    let mut tx = db::active_tx(&state.pool, claims.sub).await?;
    let id:Option<Uuid>=sqlx::query_scalar("SELECT id FROM cv_uploads WHERE user_id=$1 AND deleted_at IS NULL AND reviewed_at IS NULL ORDER BY created_at DESC,id DESC LIMIT 1").bind(claims.sub).fetch_optional(&mut *tx).await?;
    tx.commit().await?;
    Ok(web::Json(if let Some(id) = id {
        cv_snapshot(&state, claims.sub, id).await?
    } else {
        Value::Null
    }))
}
#[utoipa::path(post,path="/api/v1/me/cv/{id}/dismiss",params(("id"=Uuid,Path)),responses((status=200,description="Dismiss terminal draft; does not confirm skills or cancel running work")),security(("cookieAuth"=[])))]
pub async fn dismiss_cv(
    state: web::Data<State>,
    claims: Claims,
    id: web::Path<Uuid>,
) -> Result<web::Json<Value>, AppError> {
    let mut tx = db::active_tx(&state.pool, claims.sub).await?;
    let count=sqlx::query("UPDATE cv_uploads SET reviewed_at=now() WHERE id=$1 AND user_id=$2 AND status!='proses' AND deleted_at IS NULL").bind(*id).bind(claims.sub).execute(&mut *tx).await?.rows_affected();
    if count == 0 {
        return Err(AppError::NotFound);
    }
    tx.commit().await?;
    Ok(web::Json(json!({"ok":true})))
}
async fn analysis_snapshot(state: &State, user: Uuid, id: Uuid) -> Result<Value, AppError> {
    let mut tx = db::active_tx(&state.pool, user).await?;
    let row = sqlx::query("SELECT * FROM analysis_jobs WHERE id=$1 AND user_id=$2")
        .bind(id)
        .bind(user)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;
    tx.commit().await?;
    let at: chrono::DateTime<chrono::Utc> = row.get("updated_at");
    Ok(
        json!({"id":id,"status":row.get::<String,_>("status"),"step":row.get::<i16,_>("step"),"version":at.timestamp_micros().to_string(),"result":row.get::<Option<Value>,_>("result"),"error":row.get::<Option<String>,_>("error")}),
    )
}
#[utoipa::path(post,path="/api/v1/me/analysis/{id}/jobs",params(("id"=Uuid,Path)),responses((status=202,description="Create or resume durable analysis job")),security(("cookieAuth"=[])))]
pub async fn start_analysis(
    state: web::Data<State>,
    claims: Claims,
    id: web::Path<Uuid>,
) -> Result<HttpResponse, AppError> {
    let mut tx = db::active_tx(&state.pool, claims.sub).await?;
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM users WHERE id=$1 FOR UPDATE")
        .bind(claims.sub)
        .fetch_one(&mut *tx)
        .await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM occupations WHERE id=$1)")
        .bind(*id)
        .fetch_one(&mut *tx)
        .await?;
    if !exists {
        return Err(AppError::NotFound);
    }
    let catalog: i64 = sqlx::query_scalar("SELECT revision FROM catalog_revision")
        .fetch_one(&mut *tx)
        .await?;
    let key = http::analysis_key(&state, revision, catalog);
    let prior:Option<Uuid>=sqlx::query_scalar("SELECT id FROM analysis_jobs WHERE user_id=$1 AND occupation_id=$2 AND revision_key=$3 AND status IN ('proses','selesai') ORDER BY created_at DESC LIMIT 1").bind(claims.sub).bind(*id).bind(&key).fetch_optional(&mut *tx).await?;
    let job = if let Some(prior) = prior {
        prior
    } else {
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM analysis_jobs WHERE user_id=$1 AND (status='proses' OR created_at>now()-interval '1 hour')").bind(claims.sub).fetch_one(&mut *tx).await?;
        if count >= 10 {
            return Err(AppError::Busy);
        }
        let pending: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM analysis_jobs WHERE user_id=$1 AND status='proses'",
        )
        .bind(claims.sub)
        .fetch_one(&mut *tx)
        .await?;
        if pending >= 3 {
            return Err(AppError::Busy);
        }
        let job = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO analysis_jobs(id,user_id,occupation_id,revision_key) VALUES($1,$2,$3,$4)",
        )
        .bind(job)
        .bind(claims.sub)
        .bind(*id)
        .bind(key)
        .execute(&mut *tx)
        .await?;
        job
    };
    tx.commit().await?;
    Ok(HttpResponse::Accepted()
        .json(json!({"id":job,"eventsUrl":format!("/api/v1/me/analysis-jobs/{job}/events")})))
}
async fn analysis_tick(state: &State) -> Result<(), AppError> {
    analysis_tick_for(state, None).await
}
/// Explicit selection is useful for isolated acceptance and administrative processing.
pub async fn analysis_tick_for(state: &State, only: Option<Uuid>) -> Result<(), AppError> {
    sqlx::query(
        "DELETE FROM analysis_jobs WHERE ($1::uuid IS NULL OR id=$1) AND status!='proses' AND updated_at<now()-interval '1 day'",
    ).bind(only)
    .execute(&state.worker_pool)
    .await?;
    sqlx::query("UPDATE analysis_jobs SET status='gagal',error='Pemrosesan terputus. Coba lagi.',lease_until=NULL,updated_at=now() WHERE ($1::uuid IS NULL OR id=$1) AND status='proses' AND attempts>=3 AND lease_until<now()").bind(only).execute(&state.worker_pool).await?;
    let row=sqlx::query("UPDATE analysis_jobs SET step=1,attempts=attempts+1,lease_until=now()+interval '3 minutes',updated_at=now() WHERE id=(SELECT id FROM analysis_jobs WHERE ($1::uuid IS NULL OR id=$1) AND status='proses' AND attempts<3 AND (lease_until IS NULL OR lease_until<now()) ORDER BY created_at FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING *").bind(only).fetch_optional(&state.worker_pool).await?;
    let Some(row) = row else { return Ok(()) };
    let id: Uuid = row.get("id");
    let user: Uuid = row.get("user_id");
    let occupation: Uuid = row.get("occupation_id");
    let attempt: i16 = row.get("attempts");
    let key: String = row.get("revision_key");
    let started = Instant::now();
    tracing::debug!(job_id=%id,kind="analysis",attempt,stage="started","AI job stage");
    let result = tokio::time::timeout(
        Duration::from_secs(150),
        http::compute_analysis(
            state,
            Claims {
                sub: user,
                email: None,
                exp: 0,
            },
            occupation,
        )
        .instrument(tracing::info_span!("analysis_job",job_id=%id,attempt)),
    )
    .await
    .unwrap_or(Err(AppError::Provider));
    let result = result.and_then(|value| {
        if value["revision"].as_str() == Some(key.as_str()) {
            Ok(value)
        } else {
            Err(AppError::Invalid("Data berubah. Muat ulang analisis."))
        }
    });
    match result {
        Ok(value) => {
            sqlx::query("UPDATE analysis_jobs SET status='selesai',step=2,result=$2,lease_until=NULL,updated_at=now() WHERE id=$1 AND attempts=$3 AND status='proses'").bind(id).bind(value).bind(attempt).execute(&state.worker_pool).await?;
            tracing::debug!(job_id=%id,kind="analysis",stage="completed",elapsed_ms=started.elapsed().as_millis() as u64,"AI job stage");
        }
        Err(error) => {
            sqlx::query("UPDATE analysis_jobs SET status='gagal',error=$2,lease_until=NULL,updated_at=now() WHERE id=$1 AND attempts=$3 AND status='proses'").bind(id).bind(error.to_string()).bind(attempt).execute(&state.worker_pool).await?;
            tracing::debug!(job_id=%id,kind="analysis",stage="failed",elapsed_ms=started.elapsed().as_millis() as u64,error_code=error.status_code().as_u16(),"AI job stage");
        }
    }
    Ok(())
}
struct Feed {
    state: web::Data<State>,
    claims: Claims,
    request: HttpRequest,
    kind: &'static str,
    id: Uuid,
    notices: broadcast::Receiver<Notice>,
    pending: Option<Value>,
    last: String,
    closed: bool,
    deadline: Instant,
    _permits: (OwnedSemaphorePermit, OwnedSemaphorePermit),
}
impl Feed {
    async fn snapshot(&self) -> Result<Value, AppError> {
        if self.kind == "cv" {
            cv_snapshot(&self.state, self.claims.sub, self.id).await
        } else {
            analysis_snapshot(&self.state, self.claims.sub, self.id).await
        }
    }
    async fn authorize(&self) -> Result<(), AppError> {
        if let Some(auth) = self
            .request
            .app_data::<web::Data<super::session::CookieAuth>>()
        {
            let (claims, _) = super::session::resolve(&self.state, &self.request, auth).await?;
            if claims.sub != self.claims.sub {
                return Err(AppError::Unauthorized);
            }
        } else {
            let token = self
                .request
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.strip_prefix("Bearer "))
                .ok_or(AppError::Unauthorized)?;
            self.state.auth.verify(token).await?;
        }
        Ok(())
    }
}
fn frame(event: &str, id: &str, value: &Value) -> web::Bytes {
    web::Bytes::from(format!("id: {id}\nevent: {event}\ndata: {value}\n\n"))
}
async fn feed(
    state: web::Data<State>,
    claims: Claims,
    request: HttpRequest,
    id: Uuid,
    kind: &'static str,
) -> Result<HttpResponse, AppError> {
    let permits = state.events.reserve(claims.sub)?;
    let notices = state.events.tx.subscribe(); // Subscribe before the snapshot to avoid missing a transition.
    let first = if kind == "cv" {
        cv_snapshot(&state, claims.sub, id).await?
    } else {
        analysis_snapshot(&state, claims.sub, id).await?
    };
    let last = request
        .headers()
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.len() <= 128)
        .unwrap_or("")
        .to_owned();
    let feed = Feed {
        state,
        claims,
        request,
        kind,
        id,
        notices,
        pending: Some(first),
        last,
        closed: false,
        deadline: Instant::now() + Duration::from_secs(300),
        _permits: permits,
    };
    let body = stream::unfold(feed, |mut f| async move {
        if f.closed {
            return None;
        }
        loop {
            if Instant::now() >= f.deadline {
                f.closed = true;
                return Some((
                    Ok::<_, actix_web::Error>(frame(
                        "reconnect",
                        "",
                        &json!({"reason":"stream_lifetime"}),
                    )),
                    f,
                ));
            }
            let mut heartbeat = false;
            let value = if let Some(value) = f.pending.take() {
                Ok(value)
            } else {
                let wake_at = tokio::time::Instant::now() + Duration::from_secs(15);
                loop {
                    match tokio::time::timeout_at(wake_at, f.notices.recv()).await {
                        Ok(Ok(notice)) if notice.kind == f.kind && notice.id == f.id => break,
                        Ok(Ok(_)) => continue,
                        _ => {
                            heartbeat = true;
                            break;
                        }
                    }
                }
                f.snapshot().await
            };
            let value = match value {
                Ok(value) => value,
                Err(error) => {
                    f.closed = true;
                    return Some((
                        Ok(frame(
                            "failed",
                            "",
                            &json!({"error":error.to_string(),"code":error.status_code().as_u16()}),
                        )),
                        f,
                    ));
                }
            };
            if let Err(error) = f.authorize().await {
                f.closed = true;
                return Some((
                    Ok(frame(
                        "failed",
                        "",
                        &json!({"error":error.to_string(),"code":error.status_code().as_u16()}),
                    )),
                    f,
                ));
            }
            let version = value["version"].as_str().unwrap_or("").to_owned();
            let status = value["status"].as_str().unwrap_or("");
            let terminal = status != "proses";
            if version != f.last || terminal {
                f.last = version.clone();
                f.closed = terminal;
                let event = if status == "selesai" {
                    "completed"
                } else if status == "gagal" {
                    "failed"
                } else {
                    "progress"
                };
                return Some((Ok(frame(event, &version, &value)), f));
            }
            if heartbeat {
                return Some((Ok(web::Bytes::from_static(b": keep-alive\n\n")), f));
            }
        }
    });
    Ok(HttpResponse::Ok()
        .insert_header(("Content-Type", "text/event-stream"))
        .insert_header(("Cache-Control", "private, no-store, no-transform"))
        .insert_header(("X-Accel-Buffering", "no"))
        .insert_header(("X-Content-Type-Options", "nosniff"))
        .streaming(body))
}
#[utoipa::path(get,path="/api/v1/me/cv/{id}/events",params(("id"=Uuid,Path)),responses((status=200,description="Authenticated resumable CV events",content_type="text/event-stream")),security(("cookieAuth"=[])))]
pub async fn cv_events(
    state: web::Data<State>,
    claims: Claims,
    req: HttpRequest,
    id: web::Path<Uuid>,
) -> Result<HttpResponse, AppError> {
    feed(state, claims, req, *id, "cv").await
}
#[utoipa::path(get,path="/api/v1/me/analysis-jobs/{id}/events",params(("id"=Uuid,Path)),responses((status=200,description="Authenticated durable analysis events",content_type="text/event-stream")),security(("cookieAuth"=[])))]
pub async fn analysis_events(
    state: web::Data<State>,
    claims: Claims,
    req: HttpRequest,
    id: web::Path<Uuid>,
) -> Result<HttpResponse, AppError> {
    feed(state, claims, req, *id, "analysis").await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stream_slots_are_bounded_per_user_and_released_on_disconnect() {
        let hub = JobEvents::default();
        let user = Uuid::new_v4();
        let first = hub.reserve(user).unwrap();
        let second = hub.reserve(user).unwrap();
        assert!(matches!(hub.reserve(user), Err(AppError::Busy)));
        drop(first);
        assert!(hub.reserve(user).is_ok());
        drop(second);
        assert_eq!(hub.capacity.available_permits(), 64);
    }
    #[test]
    fn event_frames_are_valid_and_do_not_interpolate_newlines_from_json() {
        let value = json!({"status":"proses","step":2,"text":"line1\nline2"});
        let bytes = frame("progress", "42", &value);
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.starts_with("id: 42\nevent: progress\ndata: "));
        assert!(text.ends_with("\n\n"));
        assert_eq!(text.lines().filter(|l| l.starts_with("data:")).count(), 1);
    }
}
