use careerbridge_be::{
    application::{EmbeddingKind, EmbeddingProvider},
    infrastructure::embedding::{LocalEmbedding, cosine},
};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
fn peak_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find(|l| l.starts_with("VmHWM:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}
#[tokio::main(worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let cache = std::env::var("MODEL_CACHE_DIR").unwrap_or_else(|_| ".cache/fastembed".into());
    let start = Instant::now();
    let model = LocalEmbedding::load(cache).await?;
    let startup = start.elapsed();
    let seed: serde_json::Value =
        serde_json::from_str(include_str!("../../docs/catalog.seed.json"))?;
    let texts: Vec<_> = seed["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            format!(
                "{} {} {}",
                s["nama"].as_str().unwrap(),
                s["kategori"].as_str().unwrap(),
                s["alias"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        })
        .collect();
    let b = Instant::now();
    let passages = model.embed(texts, EmbeddingKind::Passage).await?;
    let batch_ms = b.elapsed().as_millis();
    let pool = if let Ok(url) = std::env::var("SUPABASE_DB_URL") {
        Some(
            careerbridge_be::infrastructure::db::connect_runtime_pool(
                &url,
                careerbridge_be::infrastructure::db::RuntimeRole::Api,
            )
            .await?,
        )
    } else {
        None
    };
    if let Some(pool) = &pool {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM skills WHERE embedding_model=$1 AND embedding IS NOT NULL",
        )
        .bind(model.version())
        .fetch_one(pool)
        .await?;
        if count == 0 {
            return Err(
                "Run the backend catalog embedding worker before the PostgreSQL benchmark".into(),
            );
        }
    }
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    let address = listener.local_addr()?;
    let server = actix_web::HttpServer::new(|| {
        actix_web::App::new().route(
            "/health",
            actix_web::web::get()
                .to(|| async { actix_web::web::Json(serde_json::json!({"status":"ok"})) }),
        )
    })
    .workers(2)
    .listen(listener)?
    .run();
    let handle = server.handle();
    tokio::spawn(server);
    let running = Arc::new(AtomicBool::new(true));
    let keep_running = running.clone();
    let traffic = tokio::spawn(async move {
        let client = reqwest::Client::new();
        let mut requests = 0;
        let mut errors = 0;
        while keep_running.load(Ordering::Relaxed) {
            match client.get(format!("http://{address}/health")).send().await {
                Ok(r) if r.status().is_success() => requests += 1,
                _ => errors += 1,
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        (requests, errors)
    });
    let mut timings = vec![];
    for _ in 0..100 {
        let start = Instant::now();
        let v = model
            .embed(
                vec!["mengolah data menggunakan spreadsheet".into()],
                EmbeddingKind::Query,
            )
            .await?;
        if let Some(pool) = &pool {
            let _:Vec<uuid::Uuid>=sqlx::query_scalar("SELECT id FROM skills WHERE embedding IS NOT NULL AND embedding_model=$2 ORDER BY embedding <=> $1,id LIMIT 5").bind(pgvector::Vector::from(v[0].clone())).bind(model.version()).fetch_all(pool).await?;
        } else {
            let mut scores: Vec<_> = passages.iter().map(|p| cosine(&v[0], p)).collect();
            scores.sort_by(|a, b| b.total_cmp(a));
            scores.truncate(5);
        }
        timings.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    timings.sort_by(f64::total_cmp);
    let start = Instant::now();
    let results = futures_util::future::join_all(
        (0..8).map(|_| model.embed(vec!["query SQL database".into()], EmbeddingKind::Query)),
    )
    .await;
    for r in results {
        r?;
    }
    let concurrent_ms = start.elapsed().as_millis();
    running.store(false, Ordering::Relaxed);
    let (http_requests, http_errors) = traffic.await?;
    handle.stop(true).await;
    let address_space = rlimit::Resource::AS.get().ok().map(|limits| limits.0);
    let queue = json!({"requests":model.metrics.requests.load(Ordering::Relaxed),"totalQueueMicros":model.metrics.queue_micros.load(Ordering::Relaxed),"maxQueueMicros":model.metrics.max_queue_micros.load(Ordering::Relaxed),"totalInferenceMicros":model.metrics.inference_micros.load(Ordering::Relaxed)});
    let report = json!({"model":model.version(),"startupMs":startup.as_millis(),"peakRssKiB":peak_kib(),"singleQueryTopKp95Ms":timings[94],"batch66Ms":batch_ms,"concurrent8WallMs":concurrent_ms,"queue":queue,"httpTraffic":{"requests":http_requests,"errors":http_errors},"topKBackend":if pool.is_some(){"PostgreSQL pgvector exact cosine Top-5"}else{"in-memory exact cosine over 66 seeded skills"},"environment":{"addressSpaceLimitBytes":address_space,"cpuAffinity":std::fs::read_to_string("/proc/self/status").ok().and_then(|s|s.lines().find(|s|s.starts_with("Cpus_allowed_list:")).map(str::to_owned)),"cpuMax":std::fs::read_to_string("/sys/fs/cgroup/cpu.max").ok(),"memoryMax":std::fs::read_to_string("/sys/fs/cgroup/memory.max").ok()},"targetsMet":peak_kib()<=1572864&&startup.as_secs_f64()<=30.0&&timings[94]<=500.0,"acceptanceNote":"Run twice for first-download vs cached startup. HTTP traffic shares this process with inference. Set SUPABASE_DB_URL for PostgreSQL retrieval. Address-space limit is stricter than RSS; parent cgroup limits are also reported."});
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
