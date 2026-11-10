use std::{env, io};
#[derive(Clone)]
pub struct Config {
    pub signup_otp_enabled: bool,
    pub cookie_secure: bool,
    pub auth_origins: Vec<String>,
    pub host: String,
    pub port: u16,
    pub database_url: String,
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
        let signup_otp_enabled =
            parse_signup_otp(env::var("AUTH_SIGNUP_OTP_ENABLED").ok().as_deref())?;
        let cookie_secure = match env::var("AUTH_COOKIE_SECURE").as_deref() {
            Ok("false") => false,
            Ok("true") | Err(_) => true,
            _ => return Err(io::Error::other("AUTH_COOKIE_SECURE must be true or false")),
        };
        let cors_origins: Vec<String> = required("CORS_ORIGINS")?
            .split(',')
            .map(|s| s.trim().to_owned())
            .collect();
        let auth_origins: Vec<String> = env::var("AUTH_ORIGINS")
            .unwrap_or_else(|_| cors_origins.join(","))
            .split(',')
            .map(|s| s.trim().to_owned())
            .collect();
        let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".into());
        validate_browser_origins(cookie_secure, &host, &cors_origins, &auth_origins)?;
        Ok(Self {
            signup_otp_enabled,
            cookie_secure,
            auth_origins,
            host,
            port: env::var("PORT")
                .unwrap_or_else(|_| "8080".into())
                .parse()
                .map_err(io::Error::other)?,
            database_url: database_url()?,
            supabase_url: url,
            storage_key: required("SUPABASE_SECRET_KEY")?,
            bucket: env::var("SUPABASE_CV_BUCKET").unwrap_or_else(|_| "cv-private".into()),
            encryption_key: required("CV_ENCRYPTION_KEY")?,
            llm_url,
            llm_key: required("LLM_API_KEY")?,
            llm_model: required("LLM_MODEL")?,
            cors_origins,
            model_cache: env::var("MODEL_CACHE_DIR").unwrap_or_else(|_| ".cache/fastembed".into()),
        })
    }
}

fn parse_signup_otp(value: Option<&str>) -> io::Result<bool> {
    match value {
        None | Some("true") => Ok(true),
        Some("false") => Ok(false),
        _ => Err(io::Error::other(
            "AUTH_SIGNUP_OTP_ENABLED must be true or false",
        )),
    }
}

fn validate_browser_origins(
    cookie_secure: bool,
    host: &str,
    cors_origins: &[String],
    auth_origins: &[String],
) -> io::Result<()> {
    for origin in cors_origins.iter().chain(auth_origins.iter()) {
        let url = reqwest::Url::parse(origin)
            .map_err(|_| io::Error::other("Invalid authentication origin"))?;
        let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if url.origin().ascii_serialization() != *origin
            || (!loopback && url.scheme() != "https")
            || (!cookie_secure && (!loopback || !matches!(host, "127.0.0.1" | "localhost" | "::1")))
        {
            return Err(io::Error::other(
                "Use exact HTTPS origins; insecure cookies require loopback-only development",
            ));
        }
    }
    Ok(())
}

/// The only database environment variable used by the application and migrator.
pub fn database_url() -> io::Result<String> {
    let url = required("SUPABASE_DB_URL")?;
    database_options(&url)?;
    Ok(url)
}

pub fn database_options(url: &str) -> io::Result<sqlx::postgres::PgConnectOptions> {
    use std::str::FromStr;
    if !url.starts_with("postgres://") && !url.starts_with("postgresql://") {
        return Err(io::Error::other(
            "SUPABASE_DB_URL must be a PostgreSQL connection string",
        ));
    }
    let options = sqlx::postgres::PgConnectOptions::from_str(url).map_err(|_| {
        io::Error::other("SUPABASE_DB_URL is not a valid PostgreSQL connection string")
    })?;
    if options.get_host().ends_with(".pooler.supabase.com") && options.get_port() == 6543 {
        return Err(io::Error::other(
            "SUPABASE_DB_URL must use the Supabase session pooler on port 5432 or a direct connection; transaction pooling cannot preserve runtime roles",
        ));
    }
    let hosted = options.get_host().ends_with(".pooler.supabase.com")
        || options.get_host().ends_with(".supabase.co");
    if hosted {
        let custom_root = reqwest::Url::parse(url)
            .is_ok_and(|parsed| parsed.query_pairs().any(|(key, _)| key == "sslrootcert"))
            || env::var_os("PGSSLROOTCERT").is_some();
        let options = options.ssl_mode(sqlx::postgres::PgSslMode::VerifyFull);
        return Ok(if custom_root {
            options
        } else if let Ok(pem) = env::var("SUPABASE_DB_CA_CERT") {
            if pem.trim().is_empty() {
                return Err(io::Error::other("SUPABASE_DB_CA_CERT must not be empty"));
            }
            options.ssl_root_cert_from_pem(pem.into_bytes())
        } else {
            options
        });
    }
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_session_and_direct_connections_but_rejects_transaction_pooling() {
        assert!(
            database_options(
                "postgresql://postgres:secret@aws-0-example.pooler.supabase.com:5432/postgres"
            )
            .is_ok()
        );
        assert!(
            database_options("postgresql://postgres:secret@db.example.supabase.co:5432/postgres")
                .is_ok()
        );
        let error = database_options(
            "postgresql://postgres:secret@aws-0-example.pooler.supabase.com:6543/postgres",
        )
        .unwrap_err();
        let verified = database_options("postgresql://postgres:secret@aws-0-example.pooler.supabase.com:5432/postgres?sslmode=disable").unwrap();
        assert!(matches!(
            verified.get_ssl_mode(),
            sqlx::postgres::PgSslMode::VerifyFull
        ));
        assert!(error.to_string().contains("session pooler"));
        assert!(!error.to_string().contains("secret"));
    }

    #[test]
    fn signup_otp_is_enabled_by_default_and_rejects_invalid_flags() {
        assert!(parse_signup_otp(None).unwrap());
        assert!(parse_signup_otp(Some("true")).unwrap());
        assert!(!parse_signup_otp(Some("false")).unwrap());
        assert!(parse_signup_otp(Some("")).is_err());
        assert!(parse_signup_otp(Some("off")).is_err());
    }
    #[test]
    fn insecure_cookies_are_limited_to_loopback_and_origins_are_exact() {
        let origins = |s: &str| vec![s.to_owned()];
        assert!(
            validate_browser_origins(false, "127.0.0.1", &origins("http://localhost:5173"), &[])
                .is_ok()
        );
        assert!(
            validate_browser_origins(false, "0.0.0.0", &origins("http://localhost:5173"), &[])
                .is_err()
        );
        assert!(
            validate_browser_origins(
                false,
                "127.0.0.1",
                &origins("https://app.example.test"),
                &[]
            )
            .is_err()
        );
        assert!(
            validate_browser_origins(true, "0.0.0.0", &origins("https://app.example.test"), &[])
                .is_ok()
        );
        for bad in [
            "*",
            "null",
            "http://app.example.test",
            "https://app.example.test/",
            "https://app.example.test/path",
            "https://user:password@app.example.test",
        ] {
            assert!(validate_browser_origins(true, "0.0.0.0", &origins(bad), &[]).is_err());
        }
    }
    #[test]
    fn invalid_connection_errors_do_not_include_credentials() {
        let error = database_options("not-postgres://private-password").unwrap_err();
        assert!(!error.to_string().contains("private-password"));
    }
}
