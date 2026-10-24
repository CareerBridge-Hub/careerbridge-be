use super::error::AppError;
use crate::application::{EmbeddingKind, EmbeddingProvider};
use async_trait::async_trait;
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::{mpsc, oneshot};
// Pin model assets by their resolved HuggingFace commit in the cache manifest at deployment.
const VERSION: &str = "intfloat/multilingual-e5-small:fastembed-7.5.0:prefix-v1:384";
type Reply = oneshot::Sender<Result<Vec<Vec<f32>>, AppError>>;
struct Request {
    texts: Vec<String>,
    reply: Reply,
    queued: std::time::Instant,
}
#[derive(Default)]
pub struct EmbeddingMetrics {
    pub queue_micros: AtomicU64,
    pub max_queue_micros: AtomicU64,
    pub inference_micros: AtomicU64,
    pub requests: AtomicU64,
}
pub struct LocalEmbedding {
    pub metrics: Arc<EmbeddingMetrics>,
    tx: mpsc::Sender<Request>,
    version: String,
}
impl LocalEmbedding {
    pub async fn load(cache: String) -> Result<Arc<Self>, AppError> {
        let started = std::time::Instant::now();
        let fingerprint_cache = cache.clone();
        let mut model = tokio::task::spawn_blocking(move || {
            TextEmbedding::try_new(
                TextInitOptions::new(EmbeddingModel::MultilingualE5Small)
                    .with_cache_dir(cache.into())
                    .with_intra_threads(2)
                    .with_show_download_progress(false),
            )
        })
        .await
        .map_err(|_| AppError::Internal)?
        .map_err(|_| AppError::Provider)?;
        tracing::info!(
            startup_ms = started.elapsed().as_millis(),
            "embedding model ready"
        );
        let (tx, mut rx) = mpsc::channel::<Request>(16);
        let metrics = Arc::new(EmbeddingMetrics::default());
        let worker_metrics = metrics.clone();
        std::thread::Builder::new()
            .name("embedding-cpu".into())
            .spawn(move || {
                while let Some(r) = rx.blocking_recv() {
                    if r.reply.is_closed() {
                        continue;
                    }
                    tracing::debug!(
                        queue_ms = r.queued.elapsed().as_millis(),
                        items = r.texts.len(),
                        "embedding request"
                    );
                    let wait = r.queued.elapsed().as_micros() as u64;
                    worker_metrics
                        .queue_micros
                        .fetch_add(wait, Ordering::Relaxed);
                    worker_metrics
                        .max_queue_micros
                        .fetch_max(wait, Ordering::Relaxed);
                    let started = std::time::Instant::now();
                    let result = model
                        .embed(r.texts, Some(8))
                        .map_err(|_| AppError::Provider)
                        .and_then(|v| {
                            if v.iter()
                                .all(|e| e.len() == 384 && e.iter().all(|f| f.is_finite()))
                            {
                                Ok(v)
                            } else {
                                Err(AppError::Internal)
                            }
                        });
                    worker_metrics
                        .inference_micros
                        .fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
                    worker_metrics.requests.fetch_add(1, Ordering::Relaxed);
                    let _ = r.reply.send(result);
                }
            })
            .map_err(|_| AppError::Internal)?;
        let revision = std::fs::read_to_string(
            std::path::Path::new(&fingerprint_cache)
                .join("models--intfloat--multilingual-e5-small/refs/main"),
        )
        .map_err(|_| AppError::Internal)?;
        let revision = revision.trim();
        if revision.len() != 40 || !revision.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(AppError::Internal);
        }
        if let Ok(expected) = std::env::var("EMBEDDING_MODEL_REVISION")
            && expected != revision
        {
            return Err(AppError::Invalid(
                "Model revision differs from the approved artifact.",
            ));
        }
        Ok(Arc::new(Self {
            tx,
            metrics,
            version: format!("{VERSION}:{revision}"),
        }))
    }
}
#[async_trait]
impl EmbeddingProvider for LocalEmbedding {
    async fn embed(
        &self,
        texts: Vec<String>,
        kind: EmbeddingKind,
    ) -> Result<Vec<Vec<f32>>, AppError> {
        if texts.len() > 100 || texts.iter().any(|t| t.len() > 16_000) {
            return Err(AppError::Invalid("Teks embedding terlalu panjang."));
        }
        if texts.is_empty() {
            return Ok(vec![]);
        }
        let prefix = match kind {
            EmbeddingKind::Query => "query: ",
            EmbeddingKind::Passage => "passage: ",
        };
        let (reply, rx) = oneshot::channel();
        self.tx
            .try_send(Request {
                texts: texts.into_iter().map(|s| format!("{prefix}{s}")).collect(),
                reply,
                queued: std::time::Instant::now(),
            })
            .map_err(|_| AppError::Busy)?;
        tokio::time::timeout(std::time::Duration::from_secs(60), rx)
            .await
            .map_err(|_| AppError::Busy)?
            .map_err(|_| AppError::Internal)?
    }
    fn version(&self) -> &str {
        &self.version
    }
}
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum();
    let nb: f32 = b.iter().map(|x| x * x).sum();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb).sqrt()
    }
}
