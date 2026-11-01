use actix_cors::Cors;
use actix_web::{App, HttpServer, http::header, middleware::DefaultHeaders, web};
use careerbridge_be::{
    infrastructure::{
        auth::Auth, config::Config, crypto::Cipher, embedding::LocalEmbedding, llm::CompatibleLlm,
        storage::Storage, worker,
    },
    presentation::http::{self, State},
};
use std::{
    io::{Read, Write},
    sync::Arc,
    time::Duration,
};

fn extract() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(unix)]
    {
        rlimit::Resource::CORE.set(0, 0)?;
        rlimit::Resource::AS.set(512 * 1024 * 1024, 512 * 1024 * 1024)?;
        rlimit::Resource::CPU.set(10, 10)?;
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    std::io::stdin().take(2097153).read_to_end(&mut bytes)?;
    if bytes.len() > 2097152 || !bytes.starts_with(b"%PDF-") {
        return Err("invalid PDF".into());
    }
    let text = zeroize::Zeroizing::new(pdf_extract::extract_text_from_mem(&bytes)?);
    if text.len() > 100_000 {
        return Err("too much text".into());
    }
    std::io::stdout().write_all(text.as_bytes())?;
    Ok(())
}
#[actix_web::main]
async fn main() -> std::io::Result<()> {
    if std::env::args().any(|a| a == "--extract-cv") {
        return extract().map_err(|_| std::io::Error::other("PDF extraction failed"));
    }
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let c = Config::from_env()?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(std::io::Error::other)?;
    let pool = careerbridge_be::infrastructure::db::connect_runtime_pool(
        &c.database_url,
        careerbridge_be::infrastructure::db::RuntimeRole::Api,
    )
    .await
    .map_err(|_| {
        std::io::Error::other(
            "API database role initialization failed; run migrations with SUPABASE_DB_URL first",
        )
    })?;
    let worker_pool = careerbridge_be::infrastructure::db::connect_runtime_pool(
        &c.database_url,
        careerbridge_be::infrastructure::db::RuntimeRole::Worker,
    )
    .await
    .map_err(|_| {
        std::io::Error::other(
            "Worker database role initialization failed; run migrations with SUPABASE_DB_URL first",
        )
    })?;
    let embedding = LocalEmbedding::load(c.model_cache)
        .await
        .map_err(std::io::Error::other)?;
    let state = Arc::new(State {
        events: Default::default(),
        pdf: Arc::new(careerbridge_be::infrastructure::pdf::SubprocessPdf {
            executable: std::env::current_exe()?,
        }),
        pool,
        worker_pool,
        auth: Auth::new(&c.supabase_url, client.clone()),
        cipher: Cipher::new(&c.encryption_key).map_err(std::io::Error::other)?,
        storage: Storage {
            client: client.clone(),
            url: c.supabase_url,
            key: c.storage_key,
            bucket: c.bucket,
        },
        embedding,
        llm: Arc::new(CompatibleLlm {
            client,
            base_url: c.llm_url,
            key: c.llm_key,
            model: c.llm_model,
        }),
    });
    let cookie_auth = web::Data::new(
        careerbridge_be::presentation::session::CookieAuth::new(
            c.cookie_secure,
            c.auth_origins.clone(),
        )
        .with_signup_otp(c.signup_otp_enabled),
    );
    careerbridge_be::presentation::events::start_background(state.clone());
    tokio::spawn(worker::run(state.clone()));
    HttpServer::new(move || {
        let mut cors = Cors::default()
            .allowed_methods(vec!["GET", "POST", "PUT", "PATCH", "DELETE"])
            .allowed_headers(vec![
                header::CONTENT_TYPE,
                header::HeaderName::from_static("x-csrf-protection"),
                header::HeaderName::from_static("last-event-id"),
            ])
            .supports_credentials()
            .max_age(600);
        for origin in &c.cors_origins {
            cors = cors.allowed_origin(origin)
        }
        let mut headers = DefaultHeaders::new()
            .add(("X-Content-Type-Options", "nosniff"))
            .add(("Cache-Control", "no-store"))
            .add(("Referrer-Policy", "no-referrer"))
            .add(("X-Frame-Options", "DENY"));
        if c.cookie_secure {
            headers = headers.add(("Strict-Transport-Security", "max-age=31536000"));
        }
        App::new()
            .app_data(web::Data::from(state.clone()))
            .app_data(cookie_auth.clone())
            .app_data(
                web::JsonConfig::default()
                    .limit(64 * 1024)
                    .error_handler(|_, _| {
                        careerbridge_be::infrastructure::error::AppError::Invalid(
                            "JSON tidak valid.",
                        )
                        .into()
                    }),
            )
            .app_data(actix_multipart::form::MultipartFormConfig::default().total_limit(2100000))
            .wrap(cors)
            .wrap(headers)
            .configure(http::configure)
    })
    .workers(2)
    .bind((c.host.as_str(), c.port))?
    .run()
    .await
}
