use std::{env, io};
#[derive(Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub worker_database_url: String,
    pub supabase_url: String,
    pub storage_key: String,
    pub bucket: String,
    pub encryption_key: String,
    pub llm_url: String,
    pub llm_key: String,
    pub llm_model: String,
    pub cors_origins: Vec<String>,
    pub model_cache: String,
}
fn required(key: &str) -> io::Result<String> {
    env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| io::Error::other(format!("{key} is required")))
}
impl Config {
    pub fn from_env() -> io::Result<Self> {
        let _ = dotenvy::dotenv();
        let url = required("SUPABASE_URL")?.trim_end_matches('/').to_owned();
        if !url.starts_with("https://")
            && !url.starts_with("http://127.0.0.1:")
            && !url.starts_with("http://localhost:")
        {
            return Err(io::Error::other(
                "SUPABASE_URL must use HTTPS (except local development)",
            ));
        }
        let llm_url = env::var("LLM_BASE_URL")
            .unwrap_or_else(|_| "https://generativelanguage.googleapis.com/v1beta/openai".into());
        if !llm_url.starts_with("https://") {
            return Err(io::Error::other("LLM_BASE_URL must use HTTPS"));
        }
        Ok(Self {
            host: env::var("HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            port: env::var("PORT")
                .unwrap_or_else(|_| "8080".into())
                .parse()
                .map_err(io::Error::other)?,
            database_url: required("DATABASE_URL")?,
            worker_database_url: required("WORKER_DATABASE_URL")?,
            supabase_url: url,
            storage_key: required("SUPABASE_SECRET_KEY")?,
            bucket: env::var("SUPABASE_CV_BUCKET").unwrap_or_else(|_| "cv-private".into()),
            encryption_key: required("CV_ENCRYPTION_KEY")?,
            llm_url,
            llm_key: required("LLM_API_KEY")?,
            llm_model: required("LLM_MODEL")?,
            cors_origins: required("CORS_ORIGINS")?
                .split(',')
                .map(|s| s.trim().to_owned())
                .collect(),
            model_cache: env::var("MODEL_CACHE_DIR").unwrap_or_else(|_| ".cache/fastembed".into()),
        })
    }
}
