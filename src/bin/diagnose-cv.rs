//! Read-only timing/queue summary. Never selects CV text, file names or user identities.
use careerbridge_be::infrastructure::{config, db};
use serde_json::Value;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let pool = db::connect_runtime_pool(&config::database_url()?, db::RuntimeRole::Worker).await?;
    let summary: Value = sqlx::query_scalar("SELECT jsonb_build_object('jobs',count(*),'pending',count(*) FILTER(WHERE status='proses'),'failed',count(*) FILTER(WHERE status='gagal'),'completed',count(*) FILTER(WHERE status='selesai'),'median_processing_ms',percentile_cont(0.5) WITHIN GROUP(ORDER BY processing_ms) FILTER(WHERE processing_ms IS NOT NULL),'p95_processing_ms',percentile_cont(0.95) WITHIN GROUP(ORDER BY processing_ms) FILTER(WHERE processing_ms IS NOT NULL),'oldest_pending_seconds',max(extract(epoch FROM now()-created_at)) FILTER(WHERE status='proses'),'expired_leases',count(*) FILTER(WHERE status='proses' AND lease_until<now())) FROM cv_uploads WHERE deleted_at IS NULL").fetch_one(&pool).await?;
    let recent: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(j),'[]') FROM (SELECT status,step,attempts,processing_ms,created_at,updated_at,lease_until IS NOT NULL AND lease_until>now() AS leased FROM cv_uploads WHERE deleted_at IS NULL ORDER BY created_at DESC LIMIT 10) j").fetch_one(&pool).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({"summary":summary,"recent":recent}))?
    );
    Ok(())
}
