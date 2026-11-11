//! Bounded, cookie-free employer vacancy importer. No credentials sent to sources.
use super::{browsing, config, db, error::AppError};
use crate::domain::{aliases, skills_in_text};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::LazyLock,
    time::Duration,
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug)]
pub enum Provider {
    Smart,
    Greenhouse,
    Breezy,
    /// Google for Jobs via OpenWeb Ninja: multi-employer, API-key and quota billed.
    GoogleJobs,
    /// Native ports of the production Python import sources.
    Extract,
}
#[derive(Clone, Copy, Debug)]
pub struct Source {
    pub key: &'static str,
    pub company: &'static str,
    pub board: &'static str,
    pub provider: Provider,
}
pub const SOURCES: [Source; 48] = [
    Source {
        key: "grab",
        company: "Grab",
        board: "Grab",
        provider: Provider::Smart,
    },
    Source {
        key: "cermati",
        company: "Cermati.com",
        board: "Cermaticom",
        provider: Provider::Smart,
    },
    Source {
        key: "dkatalis",
        company: "DKatalis",
        board: "dkatalislabs",
        provider: Provider::Greenhouse,
    },
    Source {
        key: "softwareseni",
        company: "SoftwareSeni",
        board: "softwareseni",
        provider: Provider::Breezy,
    },
    Source {
        key: "kredivo",
        company: "Kredivo Group",
        board: "kredivo-group",
        provider: Provider::Breezy,
    },
    // Company comes from each listing, not the registry.
    Source {
        key: "openwebninja",
        company: "",
        board: "",
        provider: Provider::GoogleJobs,
    },
    Source {
        key: "kalibrr",
        company: "",
        board: "",
        provider: Provider::Extract,
    },
    Source {
        key: "techinasia",
        company: "",
        board: "",
        provider: Provider::Extract,
    },
    Source {
        key: "jobstreet",
        company: "",
        board: "",
        provider: Provider::Extract,
    },
    Source {
        key: "glints",
        company: "",
        board: "",
        provider: Provider::Extract,
    },
    Source {
        key: "dealls",
        company: "",
        board: "",
        provider: Provider::Extract,
    },
    Source {
        key: "linkedin",
        company: "",
        board: "",
        provider: Provider::Extract,
    },
    Source {
        key: "greenhouse-xendit",
        company: "Xendit",
        board: "xendit",
        provider: Provider::Extract,
    },
    Source {
        key: "lever-ninjavan",
        company: "Ninja Van",
        board: "ninjavan",
        provider: Provider::Extract,
    },
    Source {
        key: "lever-lalamove",
        company: "Lalamove",
        board: "lalamove",
        provider: Provider::Extract,
    },
    Source {
        key: "workable-amartha",
        company: "Amartha",
        board: "amartha",
        provider: Provider::Extract,
    },
    Source {
        key: "workable-ajaib",
        company: "Ajaib",
        board: "ajaib",
        provider: Provider::Extract,
    },
    Source {
        key: "workable-fundingsocieties",
        company: "Funding Societies",
        board: "fundingsocieties",
        provider: Provider::Extract,
    },
    Source {
        key: "kitalulus",
        company: "",
        board: "",
        provider: Provider::Extract,
    },
    Source {
        key: "karir",
        company: "",
        board: "",
        provider: Provider::Extract,
    },
    Source {
        key: "indeed",
        company: "",
        board: "",
        provider: Provider::Extract,
    },
    Source {
        key: "glassdoor",
        company: "",
        board: "",
        provider: Provider::Extract,
    },
    Source {
        key: "remotive",
        company: "",
        board: "",
        provider: Provider::Extract,
    },
    Source {
        key: "remoteok",
        company: "",
        board: "",
        provider: Provider::Extract,
    },
    Source {
        key: "arbeitnow",
        company: "",
        board: "",
        provider: Provider::Extract,
    },
    Source {
        key: "hackernews",
        company: "",
        board: "",
        provider: Provider::Extract,
    },
    Source {
        key: "adzuna",
        company: "",
        board: "",
        provider: Provider::Extract,
    },
    Source {
        key: "greenhouse-binance",
        company: "binance",
        board: "binance",
        provider: Provider::Extract,
    },
    Source {
        key: "greenhouse-okx",
        company: "okx",
        board: "okx",
        provider: Provider::Extract,
    },
    Source {
        key: "greenhouse-datadog",
        company: "datadog",
        board: "datadog",
        provider: Provider::Extract,
    },
    Source {
        key: "greenhouse-airbnb",
        company: "airbnb",
        board: "airbnb",
        provider: Provider::Extract,
    },
    Source {
        key: "greenhouse-stripe",
        company: "stripe",
        board: "stripe",
        provider: Provider::Extract,
    },
    Source {
        key: "greenhouse-gitlab",
        company: "gitlab",
        board: "gitlab",
        provider: Provider::Extract,
    },
    Source {
        key: "lever-nium",
        company: "nium",
        board: "nium",
        provider: Provider::Extract,
    },
    Source {
        key: "lever-dlocal",
        company: "dlocal",
        board: "dlocal",
        provider: Provider::Extract,
    },
    Source {
        key: "lever-palantir",
        company: "palantir",
        board: "palantir",
        provider: Provider::Extract,
    },
    Source {
        key: "lever-spotify",
        company: "spotify",
        board: "spotify",
        provider: Provider::Extract,
    },
    Source {
        key: "ashby-airwallex",
        company: "airwallex",
        board: "airwallex",
        provider: Provider::Extract,
    },
    Source {
        key: "ashby-notion",
        company: "notion",
        board: "notion",
        provider: Provider::Extract,
    },
    Source {
        key: "ashby-openai",
        company: "openai",
        board: "openai",
        provider: Provider::Extract,
    },
    Source {
        key: "ashby-ramp",
        company: "ramp",
        board: "ramp",
        provider: Provider::Extract,
    },
    Source {
        key: "smartrecruiters-alodokter",
        company: "alodokter",
        board: "alodokter",
        provider: Provider::Extract,
    },
    Source {
        key: "smartrecruiters-stockbit",
        company: "stockbit",
        board: "stockbit",
        provider: Provider::Extract,
    },
    Source {
        key: "smartrecruiters-bukuwarung",
        company: "bukuwarung",
        board: "bukuwarung",
        provider: Provider::Extract,
    },
    Source {
        key: "smartrecruiters-sayurbox",
        company: "sayurbox",
        board: "sayurbox",
        provider: Provider::Extract,
    },
    Source {
        key: "smartrecruiters-dekoruma",
        company: "dekoruma",
        board: "dekoruma",
        provider: Provider::Extract,
    },
    Source {
        key: "smartrecruiters-julo",
        company: "julo",
        board: "julo",
        provider: Provider::Extract,
    },
    Source {
        key: "smartrecruiters-pasarpolis",
        company: "pasarpolis",
        board: "pasarpolis",
        provider: Provider::Extract,
    },
];
const DAY: Duration = Duration::from_secs(86_400);
impl Source {
    /// Metered feeds run at most daily and wait a full interval after failures,
    /// so an outage cannot burn the monthly request quota.
    pub fn metered(self) -> bool {
        matches!(self.provider, Provider::GoogleJobs)
    }
}
const GOOGLE_JOBS_URL: &str = "https://api.openwebninja.com/realtime-jobs-data/google-jobs/search";
#[derive(Clone, Default)]
pub struct GoogleJobs {
    api_key: Option<String>,
    per_run: i64,
}
impl GoogleJobs {
    fn from_env() -> std::io::Result<Self> {
        let api_key = std::env::var("OPENWEBNINJA_API_KEY")
            .ok()
            .map(|k| k.trim().to_owned())
            .filter(|k| !k.is_empty());
        // Each query is one billed request. Default 2/day ~ 60/month (Basic plan: 100).
        let per_run = std::env::var("OPENWEBNINJA_QUERIES_PER_RUN")
            .unwrap_or_else(|_| "2".into())
            .parse::<i64>()
            .ok()
            .filter(|n| (1..=10).contains(n))
            .ok_or_else(|| std::io::Error::other("OPENWEBNINJA_QUERIES_PER_RUN must be 1 to 10"))?;
        Ok(Self { api_key, per_run })
    }
}
/// One row of the `job_search_queries` queue: a keyword tied to an occupation.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct SearchQuery {
    pub id: Uuid,
    pub occupation_id: Uuid,
    pub keyword: String,
    pub title_pattern: Option<String>,
}
/// Round-robin: the queries that waited longest go first; never-run rows lead.
pub async fn next_queries(pool: &PgPool, limit: i64) -> Result<Vec<SearchQuery>, AppError> {
    Ok(sqlx::query_as("SELECT id,occupation_id,keyword,title_pattern FROM job_search_queries WHERE enabled ORDER BY last_run_at NULLS FIRST,keyword LIMIT $1")
        .bind(limit).fetch_all(pool).await?)
}
#[derive(Clone)]
pub struct Settings {
    pub enabled: bool,
    pub interval: Duration,
    pub sources: Vec<Source>,
    pub google: GoogleJobs,
}
impl Settings {
    pub fn from_env() -> std::io::Result<Self> {
        let enabled = match std::env::var("JOB_SCRAPE_ENABLED").as_deref() {
            Ok("true") => true,
            Ok("false") | Err(_) => false,
            _ => {
                return Err(std::io::Error::other(
                    "JOB_SCRAPE_ENABLED must be true or false",
                ));
            }
        };
        let seconds: u64 = std::env::var("JOB_SCRAPE_INTERVAL_SECONDS")
            .unwrap_or_else(|_| "21600".into())
            .parse()
            .map_err(|_| std::io::Error::other("Invalid JOB_SCRAPE_INTERVAL_SECONDS"))?;
        if !(3600..=604800).contains(&seconds) {
            return Err(std::io::Error::other(
                "Job interval must be 1 hour to 7 days",
            ));
        }
        let google = GoogleJobs::from_env()?;
        let keys = std::env::var("JOB_SCRAPE_SOURCES").unwrap_or_else(|_| {
            let mut keys = String::from("grab,cermati,dkatalis,softwareseni,kredivo,kalibrr,techinasia,jobstreet,glints,dealls,linkedin,greenhouse-xendit,lever-ninjavan,lever-lalamove,workable-amartha,workable-ajaib,workable-fundingsocieties");
            if google.api_key.is_some() {
                keys.push_str(",openwebninja");
            }
            keys
        });
        let mut sources = vec![];
        for key in keys.split(',').map(str::trim) {
            let source = SOURCES
                .iter()
                .find(|s| s.key == key)
                .ok_or_else(|| std::io::Error::other("Unknown JOB_SCRAPE_SOURCES entry"))?;
            if !sources.iter().any(|s: &Source| s.key == key) {
                sources.push(*source);
            }
        }
        if sources.iter().any(|s| s.metered()) && google.api_key.is_none() {
            return Err(std::io::Error::other(
                "OPENWEBNINJA_API_KEY is required for the openwebninja source",
            ));
        }
        Ok(Self {
            enabled,
            interval: Duration::from_secs(seconds),
            sources,
            google,
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Vacancy {
    pub source: String,
    pub source_id: String,
    pub company: String,
    pub title: String,
    pub country: String,
    pub city: String,
    pub region: String,
    pub workplace: String,
    pub employment_type: String,
    pub description: String,
    pub requirements: String,
    pub apply_url: String,
    pub source_url: String,
    pub posted_at: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    /// Set by the search queue; employer feeds are classified by title at persist time.
    #[serde(default)]
    pub occupation_id: Option<Uuid>,
}
/// Employer feeds carry only software roles; map them to catalog occupation names.
fn feed_occupation(title: &str) -> &'static str {
    static DEVOPS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)devops|devsecops|infraops|site reliability|\bsre\b").unwrap()
    });
    static DATA: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)data[ -]?analyst|business intelligence analyst|\bbi analyst").unwrap()
    });
    if DEVOPS.is_match(title) {
        "DevOps Engineer"
    } else if DATA.is_match(title) {
        "Data Analyst"
    } else {
        "Software Engineer"
    }
}
static ROLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:software\s+(?:engineer\w*|develop\w*|quality\s+assurance)|data\s+analyst\w*|(?:back[ -]?end|front[ -]?end|full[ -]?stack|mobile|android|ios|web|application)\s+(?:engineer\w*|develop\w*)|(?:[a-z.]+\s+)?developer|devops|devsecops|infraops|site\s+reliability|sre|(?:qa|test|automation)\s+engineer\w*|programmer)\b").unwrap()
});
static EXCLUDE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:sales|recruiter|designer|product\s+manager|business\s+developer|business\s+development)\b").unwrap()
});
pub fn software(title: &str) -> bool {
    ROLE.is_match(title) && !EXCLUDE.is_match(title)
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn identifier(v: &Value) -> String {
    v.as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| v.as_u64().map(|n| n.to_string()).unwrap_or_default())
}
fn date(v: &Value) -> Option<DateTime<Utc>> {
    v.as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}
// Store text only. The frontend must render these values as text, never innerHTML.
pub fn plain(raw: &str) -> String {
    static HIDDEN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?is)<(?:script|style)\b[^>]*>.*?</(?:script|style)\s*>").unwrap()
    });
    static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").unwrap());
    let mut value = raw.to_owned();
    // Two passes cover entity-encoded Greenhouse markup.
    for _ in 0..2 {
        value = value
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&nbsp;", " ")
            .replace("&amp;", "&");
    }
    let value = HIDDEN.replace_all(&value, "");
    TAGS.replace_all(&value, "\n")
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
// Google listings carry no country field; accept only locations naming Indonesia or a province/island.
static INDONESIA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:indonesia|jakarta|jawa|java|banten|yogyakarta|bali|sumatera|sumatra|aceh|riau|jambi|bengkulu|lampung|bangka|belitung|kalimantan|sulawesi|gorontalo|maluku|papua|nusa\s+tenggara)\b").unwrap()
});
static FOREIGN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:new\s+guinea|nugini)\b").unwrap());
fn allowed_url(source: Source, raw: &str) -> bool {
    reqwest::Url::parse(raw).is_ok_and(|u| {
        u.scheme() == "https"
            && u.username().is_empty()
            && u.password().is_none()
            && u.port_or_known_default() == Some(443)
            && match source.provider {
                Provider::Smart => {
                    u.host_str() == Some("jobs.smartrecruiters.com")
                        && u.path().starts_with(&format!("/{}/", source.board))
                }
                Provider::Greenhouse => {
                    matches!(
                        u.host_str(),
                        Some("job-boards.greenhouse.io" | "boards.greenhouse.io")
                    ) && u.path().starts_with(&format!("/{}/", source.board))
                }
                Provider::Breezy => {
                    u.host_str() == Some(format!("{}.breezy.hr", source.board).as_str())
                        && u.path().starts_with("/p/")
                }
                // Publishers vary (LinkedIn, Indeed, employer sites). The importer never
                // fetches these links; require a public DNS name, not an IP literal.
                Provider::Extract => super::extract::jobs::allowed_posting(source.key, &u),
                Provider::GoogleJobs => u
                    .domain()
                    .is_some_and(|d| d.contains('.') && !d.eq_ignore_ascii_case("localhost")),
            }
    })
}
impl Vacancy {
    pub(crate) fn validate(&self, source: Source) -> Result<(), AppError> {
        if self.source != source.key
            || self.source_id.is_empty()
            || self.source_id.len() > 128
            || !self
                .source_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            || self.country != "ID"
            || self.city.is_empty()
            || self.city.chars().count() > 200
            || self.region.chars().count() > 200
            || self.title.is_empty()
            || self.title.chars().count() > 300
            || self.company.is_empty()
            || self.company.chars().count() > 200
            || self.workplace.len() > 80
            || self.employment_type.len() > 100
            || self.description.trim().is_empty()
            || self.description.chars().count() > 100_000
            || self.requirements.chars().count() > 100_000
            || !allowed_url(source, &self.apply_url)
            || !allowed_url(source, &self.source_url)
        {
            return Err(AppError::Invalid("Vacancy schema or source URL changed"));
        }
        Ok(())
    }
}
pub struct Collector {
    client: reqwest::Client,
    google: GoogleJobs,
}
impl Collector {
    pub fn new(google: GoogleJobs) -> Result<Self, AppError> {
        let client = reqwest::Client::builder()
            .user_agent("CareerBridge-Vacancy-Importer/1.0")
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(25))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| AppError::Internal)?;
        Ok(Self { client, google })
    }
    async fn body(&self, url: &str, json: bool) -> Result<String, AppError> {
        let parsed = reqwest::Url::parse(url).map_err(|_| AppError::Internal)?;
        // URLs are built only from the fixed source registry and validated posting IDs.
        if parsed.scheme() != "https"
            || !matches!(
                parsed.host_str(),
                Some(
                    "api.smartrecruiters.com"
                        | "boards-api.greenhouse.io"
                        | "softwareseni.breezy.hr"
                        | "kredivo-group.breezy.hr"
                        | "api.openwebninja.com"
                )
            )
        {
            return Err(AppError::Invalid("Unexpected vacancy host"));
        }
        let mut request = self.client.get(url);
        if parsed.host_str() == Some("api.openwebninja.com") {
            let key = self
                .google
                .api_key
                .as_deref()
                .ok_or(AppError::Invalid("OPENWEBNINJA_API_KEY is not configured"))?;
            let mut value =
                reqwest::header::HeaderValue::from_str(key).map_err(|_| AppError::Internal)?;
            value.set_sensitive(true);
            request = request.header("X-API-Key", value);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
        let response = request.send().await.map_err(|_| AppError::Provider)?;
        if !response.status().is_success() {
            return Err(AppError::Provider);
        }
        if json
            && !response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|s| s.to_str().ok())
                .is_some_and(|s| s.starts_with("application/json"))
        {
            return Err(AppError::Invalid("Vacancy feed returned non-JSON"));
        }
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| AppError::Provider)?;
            if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                return Err(AppError::Invalid("Vacancy feed exceeds size limit"));
            }
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes).map_err(|_| AppError::Provider)
    }
    async fn json(&self, url: &str) -> Result<Value, AppError> {
        serde_json::from_str(&self.body(url, true).await?).map_err(|_| AppError::Provider)
    }
    pub async fn collect(&self, source: Source) -> Result<Vec<Vacancy>, AppError> {
        let jobs = match source.provider {
            Provider::Smart => self.smart(source).await?,
            Provider::Greenhouse => self.greenhouse(source).await?,
            Provider::Breezy => self.breezy(source).await?,
            Provider::Extract | Provider::GoogleJobs => {
                return Err(AppError::Invalid(
                    "Metered sources run from the search queue",
                ));
            }
        };
        checked(source, jobs)
    }
}
fn checked(source: Source, jobs: Vec<Vacancy>) -> Result<Vec<Vacancy>, AppError> {
    let mut ids = HashSet::new();
    for job in &jobs {
        job.validate(source)?;
        if !ids.insert(&job.source_id) {
            return Err(AppError::Invalid("Duplicate vacancy source ID"));
        }
    }
    if jobs.len() > 500 {
        return Err(AppError::Invalid("Too many vacancies in source"));
    }
    Ok(jobs)
}
impl Collector {
    async fn smart(&self, s: Source) -> Result<Vec<Vacancy>, AppError> {
        let mut result = vec![];
        let mut scanned = 0;
        for offset in (0..1000).step_by(100) {
            let data=self.json(&format!("https://api.smartrecruiters.com/v1/companies/{}/postings?country=id&limit=100&offset={offset}",s.board)).await?;
            let total = data["totalFound"].as_u64().ok_or(AppError::Provider)?;
            let jobs = data["content"].as_array().ok_or(AppError::Provider)?;
            scanned += jobs.len() as u64;
            for job in jobs {
                if text(&job["location"], "country").to_lowercase() != "id"
                    || !software(text(job, "name"))
                {
                    continue;
                }
                let id = identifier(&job["id"]);
                if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
                    return Err(AppError::Provider);
                }
                let detail = self
                    .json(&format!(
                        "https://api.smartrecruiters.com/v1/companies/{}/postings/{id}",
                        s.board
                    ))
                    .await?;
                if detail["active"].as_bool() != Some(true)
                    || text(&detail, "visibility") != "PUBLIC"
                {
                    continue;
                }
                result.push(smart_record(s, &detail)?);
            }
            if scanned >= total {
                return Ok(result);
            }
            if jobs.is_empty() {
                return Err(AppError::Invalid("Incomplete vacancy pagination"));
            }
        }
        Err(AppError::Invalid("Vacancy pagination exceeds limit"))
    }
    async fn greenhouse(&self, s: Source) -> Result<Vec<Vacancy>, AppError> {
        let data = self
            .json(&format!(
                "https://boards-api.greenhouse.io/v1/boards/{}/jobs?content=true",
                s.board
            ))
            .await?;
        let jobs = data["jobs"].as_array().ok_or(AppError::Provider)?;
        let mut result = vec![];
        for job in jobs {
            if !software(text(job, "title")) {
                continue;
            }
            // Keep only explicitly Indonesian locations; do not infer remote eligibility.
            let location = text(&job["location"], "name");
            let Some(indonesia_location) = location.split(';').find(|part| {
                part.split(',')
                    .any(|token| token.trim().eq_ignore_ascii_case("Indonesia"))
            }) else {
                continue;
            };
            let valid_until = date(&job["application_deadline"]);
            if valid_until.is_some_and(|d| d <= Utc::now()) {
                continue;
            }
            result.push(Vacancy {
                source: s.key.into(),
                source_id: identifier(&job["id"]),
                company: s.company.into(),
                title: text(job, "title").into(),
                country: "ID".into(),
                city: indonesia_location
                    .split(',')
                    .next()
                    .unwrap_or(indonesia_location)
                    .trim()
                    .into(),
                region: String::new(),
                workplace: String::new(),
                employment_type: String::new(),
                description: plain(text(job, "content")),
                requirements: String::new(),
                apply_url: text(job, "absolute_url").into(),
                source_url: text(job, "absolute_url").into(),
                posted_at: date(&job["first_published"]),
                valid_until,
                occupation_id: None,
            });
        }
        Ok(result)
    }
    async fn breezy(&self, s: Source) -> Result<Vec<Vacancy>, AppError> {
        let data = self
            .json(&format!("https://{}.breezy.hr/json", s.board))
            .await?;
        let jobs = data.as_array().ok_or(AppError::Provider)?;
        let mut result = vec![];
        for job in jobs {
            if !software(text(job, "name")) || text(&job["location"]["country"], "id") != "ID" {
                continue;
            }
            let url = text(job, "url");
            if !allowed_url(s, url) {
                return Err(AppError::Invalid("Unexpected vacancy URL"));
            }
            let document = self.body(url, false).await?;
            let ld = job_posting(&document);
            let description = ld
                .as_ref()
                .map(|v| plain(text(v, "description")))
                .filter(|v| !v.is_empty())
                .or_else(|| description_body(&document).map(plain))
                .ok_or(AppError::Invalid("Missing vacancy description"))?;
            let ld = ld.unwrap_or(Value::Null);
            let valid_until = date(&ld["validThrough"]);
            if valid_until.is_some_and(|d| d <= Utc::now()) {
                continue;
            }
            // The employer page links to this same-site application form.
            result.push(Vacancy {
                source: s.key.into(),
                source_id: identifier(&job["id"]),
                company: s.company.into(),
                title: text(job, "name").into(),
                country: "ID".into(),
                city: text(&job["location"], "city").into(),
                region: String::new(),
                workplace: if job["location"]["is_remote"].as_bool() == Some(true) {
                    "remote"
                } else {
                    "onsite"
                }
                .into(),
                employment_type: text(&job["type"], "name").into(),
                description,
                requirements: plain(text(&ld, "qualifications")),
                apply_url: format!("{}/apply", url.trim_end_matches('/')),
                source_url: url.into(),
                posted_at: date(&job["published_date"]),
                valid_until,
                occupation_id: None,
            });
        }
        Ok(result)
    }
}
impl Collector {
    /// One billed request. Returns the raw `data` array (used by the queue and `--test-query`).
    pub async fn search(&self, keyword: &str) -> Result<Vec<Value>, AppError> {
        let url = reqwest::Url::parse_with_params(
            GOOGLE_JOBS_URL,
            [
                ("query", keyword),
                ("country", "id"),
                ("language", "id"),
                ("page", "1"),
                ("num_pages", "1"),
            ],
        )
        .map_err(|_| AppError::Internal)?;
        let mut data = self.json(url.as_str()).await?;
        if text(&data, "status") != "OK" {
            return Err(AppError::Provider);
        }
        match data["data"].take() {
            Value::Array(jobs) => Ok(jobs),
            _ => Err(AppError::Provider),
        }
    }
    /// Runs one queue batch. Returns accepted vacancies plus (query id, accepted count).
    pub async fn google_jobs(
        &self,
        s: Source,
        queries: &[SearchQuery],
    ) -> Result<(Vec<Vacancy>, Vec<(Uuid, i32)>), AppError> {
        let now = Utc::now();
        let mut ids = HashSet::new();
        let mut result = vec![];
        let mut stats = vec![];
        for query in queries {
            let pattern = match query.title_pattern.as_deref().map(Regex::new).transpose() {
                Ok(p) => p,
                Err(_) => {
                    // Skip without spending a request; zero_streak exposes it in --status.
                    tracing::warn!(query_id=%query.id, "invalid title_pattern; query skipped");
                    stats.push((query.id, 0));
                    continue;
                }
            };
            let mut accepted = 0;
            // Aggregated data is messy: skip unusable listings instead of failing the run.
            for job in self.search(&query.keyword).await? {
                if let Some(v) = google_record(s, &job, now, query.occupation_id, pattern.as_ref())
                    && ids.insert(v.source_id.clone())
                {
                    accepted += 1;
                    result.push(v);
                }
            }
            stats.push((query.id, accepted));
        }
        Ok((checked(s, result)?, stats))
    }
}
pub fn google_record(
    s: Source,
    job: &Value,
    now: DateTime<Utc>,
    occupation_id: Uuid,
    title_pattern: Option<&Regex>,
) -> Option<Vacancy> {
    let title = text(job, "job_title").trim();
    if title.is_empty() || title_pattern.is_some_and(|p| !p.is_match(title)) {
        return None;
    }
    // e.g. "Jakarta Barat, Kota Jakarta Barat, Daerah Khusus Ibukota Jakarta  •  melalui LinkedIn"
    // or "Kota Semarang, Jawa Tengah   (+2 lainnya)" for multi-location listings.
    let location = text(job, "job_location")
        .split('•')
        .next()?
        .split("(+")
        .next()?
        .trim();
    if !INDONESIA.is_match(location) || FOREIGN.is_match(location) {
        return None;
    }
    let parts: Vec<&str> = location
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    let apply_url = text(job, "job_apply_link");
    let vacancy = Vacancy {
        source: s.key.into(),
        // Google docid (URL-safe base64), shared by every query that finds the listing.
        source_id: text(job, "job_uid").trim_end_matches('=').into(),
        company: text(job, "employer_name").trim().into(),
        title: title.into(),
        country: "ID".into(),
        city: (*parts.first()?).into(),
        region: if parts.len() > 1 {
            parts[parts.len() - 1]
        } else {
            ""
        }
        .into(),
        workplace: if job["job_is_remote"].as_bool() == Some(true) {
            "remote"
        } else {
            ""
        }
        .into(),
        employment_type: text(job, "job_employment_type").into(),
        description: plain(text(job, "job_description")),
        requirements: job["job_highlights"]["Qualifications"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default(),
        apply_url: apply_url.into(),
        source_url: apply_url.into(),
        posted_at: job["job_posted_at_timestamp"]
            .as_i64()
            .and_then(|t| DateTime::from_timestamp(t, 0))
            .or_else(|| relative_date(text(job, "job_posted_at"), now)),
        valid_until: None,
        occupation_id: Some(occupation_id),
    };
    vacancy.validate(s).ok()?;
    Some(vacancy)
}
/// With language=id Google gives only relative ages such as "4 hari yang lalu".
fn relative_date(raw: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    static AGO: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)^(\d{1,4})\+?\s*(menit|jam|hari|minggu|bulan|minutes?|hours?|days?|weeks?|months?)\s+(?:yang\s+lalu|ago)$").unwrap()
    });
    let c = AGO.captures(raw.trim())?;
    let n: i64 = c[1].parse().ok()?;
    let unit = match c[2].to_lowercase().as_str() {
        "menit" | "minute" | "minutes" => 60,
        "jam" | "hour" | "hours" => 3_600,
        "hari" | "day" | "days" => 86_400,
        "minggu" | "week" | "weeks" => 604_800,
        _ => 2_592_000,
    };
    Some(now - chrono::Duration::seconds(n * unit))
}
fn smart_record(s: Source, d: &Value) -> Result<Vacancy, AppError> {
    if text(&d["location"], "country").to_lowercase() != "id" {
        return Err(AppError::Invalid("Unexpected vacancy country"));
    }
    let sections = &d["jobAd"]["sections"];
    let description = [
        "companyDescription",
        "jobDescription",
        "qualifications",
        "additionalInformation",
    ]
    .iter()
    .map(|k| plain(text(&sections[*k], "text")))
    .filter(|t| !t.is_empty())
    .collect::<Vec<_>>()
    .join("\n\n");
    Ok(Vacancy {
        source: s.key.into(),
        source_id: identifier(&d["id"]),
        company: s.company.into(),
        title: text(d, "name").into(),
        country: "ID".into(),
        city: text(&d["location"], "city").into(),
        region: text(&d["location"], "region").into(),
        workplace: if d["location"]["remote"].as_bool() == Some(true) {
            "remote"
        } else if d["location"]["hybrid"].as_bool() == Some(true) {
            "hybrid"
        } else {
            "onsite"
        }
        .into(),
        employment_type: text(&d["typeOfEmployment"], "label").into(),
        description,
        requirements: plain(text(&sections["qualifications"], "text")),
        apply_url: text(d, "applyUrl").into(),
        source_url: text(d, "postingUrl").into(),
        posted_at: date(&d["releasedDate"]),
        valid_until: None,
        occupation_id: None,
    })
}
fn description_body(raw: &str) -> Option<&str> {
    static START: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?is)<div\b[^>]*\bclass=["']description["'][^>]*>"#).unwrap()
    });
    static DIV: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<(/?)div\b[^>]*>").unwrap());
    let start = START.find(raw)?.end();
    let mut depth = 1;
    for tag in DIV.captures_iter(&raw[start..]) {
        if &tag[1] == "/" {
            depth -= 1;
        } else {
            depth += 1;
        }
        if depth == 0 {
            return Some(&raw[start..start + tag.get(0)?.start()]);
        }
    }
    None
}
fn job_posting(raw: &str) -> Option<Value> {
    static SCRIPT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?is)<script\b[^>]*type=["']application/ld\+json["'][^>]*>(.*?)</script\s*>"#)
            .unwrap()
    });
    fn find(v: Value) -> Option<Value> {
        if v["@type"].as_str() == Some("JobPosting") {
            return Some(v);
        }
        if let Some(array) = v.as_array().or_else(|| v["@graph"].as_array()) {
            return array.iter().find_map(|x| find(x.clone()));
        }
        None
    }
    SCRIPT
        .captures_iter(raw)
        .find_map(|m| serde_json::from_str(&m[1]).ok().and_then(find))
}

#[derive(Debug, Serialize)]
pub struct Outcome {
    pub source: String,
    pub status: &'static str,
    pub count: usize,
}
pub async fn sync_source(
    pool: &PgPool,
    collector: &Collector,
    source: Source,
    interval: Duration,
    force: bool,
) -> Result<Outcome, AppError> {
    let interval = if source.metered() {
        interval.max(DAY)
    } else if matches!(source.provider, Provider::Extract) {
        interval.max(Duration::from_secs(43200))
    } else {
        interval
    };
    let retry = if source.metered() || matches!(source.provider, Provider::Extract) {
        interval
    } else {
        Duration::from_secs(900)
    };
    let run = Uuid::new_v4();
    let claimed=sqlx::query("UPDATE job_sources SET run_id=$2,lease_until=now()+interval '35 minutes' WHERE key=$1 AND (lease_until IS NULL OR lease_until<now()) AND ($3 OR next_run_at<=now())").bind(source.key).bind(run).bind(force).execute(pool).await?.rows_affected();
    if claimed == 0 {
        return Ok(Outcome {
            source: source.key.into(),
            status: "not_due_or_leased",
            count: 0,
        });
    }
    tracing::info!(source = source.key, "vacancy collection started");
    let collected = async {
        if matches!(source.provider, Provider::Extract) {
            super::extract::jobs::collect(pool, source).await
        } else if source.metered() {
            let batch = next_queries(pool, collector.google.per_run).await?;
            collector.google_jobs(source, &batch).await
        } else {
            Ok((collector.collect(source).await?, vec![]))
        }
    };
    let result = tokio::time::timeout(
        Duration::from_secs(if matches!(source.provider, Provider::Extract) {
            1800
        } else {
            300
        }),
        collected,
    )
    .await
    .unwrap_or(Err(AppError::Provider));
    let result = match result {
        Ok((jobs, stats)) => persist(pool, source, run, &jobs, &stats, interval)
            .await
            .map(|_| jobs.len()),
        Err(e) => Err(e),
    };
    match result {
        Ok(count) => {
            if source.key == "linkedin"
                && let Err(error) = super::extract::jobs::enrich_linkedin(pool).await
            {
                tracing::warn!(source="linkedin",error=%error,"detail backlog retained for retry");
            }
            tracing::info!(source = source.key, count, "vacancy import completed");
            Ok(Outcome {
                source: source.key.into(),
                status: "completed",
                count,
            })
        }
        Err(error) => {
            // AppError messages are fixed/sanitized; source bodies and URLs are never logged.
            sqlx::query("UPDATE job_sources SET lease_until=NULL,run_id=NULL,last_error=$3,next_run_at=now()+make_interval(secs=>$4) WHERE key=$1 AND run_id=$2").bind(source.key).bind(run).bind(error.to_string()).bind(retry.as_secs() as f64).execute(pool).await?;
            tracing::warn!(source=source.key,error=%error,"vacancy import failed; previous records preserved");
            Err(error)
        }
    }
}
pub(crate) async fn persist(
    pool: &PgPool,
    source: Source,
    run: Uuid,
    jobs: &[Vacancy],
    queue_stats: &[(Uuid, i32)],
    interval: Duration,
) -> Result<(), AppError> {
    let index = aliases(&db::skills(pool).await?);
    let mut tx = pool.begin().await?;
    let valid = sqlx::query_scalar::<_, bool>(
        "SELECT run_id=$2 AND lease_until>now() FROM job_sources WHERE key=$1 FOR UPDATE",
    )
    .bind(source.key)
    .bind(run)
    .fetch_one(&mut *tx)
    .await?;
    if !valid {
        return Err(AppError::Busy);
    }
    let previous: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM job_vacancies WHERE source=$1 AND status='active'",
    )
    .bind(source.key)
    .fetch_one(&mut *tx)
    .await?;
    // Search results are a rotating sample, not a full employer snapshot: never close
    // unseen rows; RLS hides rows not re-seen within 7 days.
    let snapshot = !source.metered()
        && (!matches!(source.provider, Provider::Extract) || !source.board.is_empty());
    if snapshot && previous > 10 && (jobs.len() as i64) * 2 < previous {
        return Err(AppError::Invalid(
            "Unexpected vacancy count drop; review source before closing records",
        ));
    }
    for job in jobs {
        job.validate(source)?;
        let id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("careerbridge:vacancy:{}:{}", source.key, job.source_id).as_bytes(),
        );
        // A missing catalog occupation leaves occupation_id NULL rather than failing the import.
        let stored=sqlx::query("INSERT INTO job_vacancies(id,source,source_id,company,title,country,city,region,workplace,employment_type,description,requirements,apply_url,source_url,posted_at,valid_until,last_seen_run,occupation_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,COALESCE($18,(SELECT id FROM occupations WHERE nama=$19))) ON CONFLICT(source,source_id) DO UPDATE SET company=EXCLUDED.company,title=EXCLUDED.title,country=EXCLUDED.country,city=EXCLUDED.city,region=EXCLUDED.region,workplace=EXCLUDED.workplace,employment_type=EXCLUDED.employment_type,description=CASE WHEN job_vacancies.source='linkedin' AND job_vacancies.detail_complete AND position('Deskripsi lengkap:' in EXCLUDED.description)>0 THEN job_vacancies.description ELSE EXCLUDED.description END,requirements=CASE WHEN job_vacancies.source='linkedin' AND job_vacancies.detail_complete AND position('Deskripsi lengkap:' in EXCLUDED.description)>0 THEN job_vacancies.requirements ELSE EXCLUDED.requirements END,apply_url=EXCLUDED.apply_url,source_url=EXCLUDED.source_url,posted_at=EXCLUDED.posted_at,valid_until=EXCLUDED.valid_until,last_seen_run=EXCLUDED.last_seen_run,occupation_id=COALESCE(EXCLUDED.occupation_id,job_vacancies.occupation_id),last_seen_at=now(),updated_at=now(),status='active' RETURNING description,requirements")
            .bind(id).bind(source.key).bind(&job.source_id).bind(&job.company).bind(&job.title).bind(&job.country).bind(&job.city).bind(&job.region).bind(&job.workplace).bind(&job.employment_type).bind(&job.description).bind(&job.requirements).bind(&job.apply_url).bind(&job.source_url).bind(job.posted_at).bind(job.valid_until).bind(run).bind(job.occupation_id).bind(feed_occupation(&job.title)).fetch_one(&mut *tx).await?;
        if source.key == "linkedin" {
            sqlx::query("UPDATE job_vacancies SET detail_complete=position('Deskripsi lengkap:' in description)=0 WHERE id=$1")
                .bind(id).execute(&mut *tx).await?;
        }
        let text = format!(
            "{}\n{}\n{}",
            job.title,
            stored.get::<String, _>("description"),
            stored.get::<String, _>("requirements")
        );
        tag_vacancy(&mut tx, id, &text, &index).await?;
    }
    if matches!(source.provider, Provider::Extract) {
        for (query_id, count) in queue_stats {
            sqlx::query("INSERT INTO extract_query_runs(source,query_id,last_run_at,last_count) VALUES($1,$2,now(),$3) ON CONFLICT(source,query_id) DO UPDATE SET last_run_at=EXCLUDED.last_run_at,last_count=EXCLUDED.last_count")
                .bind(source.key).bind(query_id).bind(count).execute(&mut *tx).await?;
        }
    } else if !queue_stats.is_empty() {
        let (ids, counts): (Vec<Uuid>, Vec<i32>) = queue_stats.iter().copied().unzip();
        sqlx::query("UPDATE job_search_queries q SET last_run_at=now(),last_count=s.n,zero_streak=CASE WHEN s.n=0 THEN q.zero_streak+1 ELSE 0 END FROM unnest($1::uuid[],$2::int[]) s(id,n) WHERE q.id=s.id")
            .bind(ids).bind(counts).execute(&mut *tx).await?;
    }
    if snapshot {
        sqlx::query("UPDATE job_vacancies SET status='closed',updated_at=now() WHERE source=$1 AND last_seen_run!=$2 AND status='active'").bind(source.key).bind(run).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE job_sources SET last_success_at=now(),last_count=$3,last_error=NULL,lease_until=NULL,run_id=NULL,next_run_at=now()+make_interval(secs=>$4) WHERE key=$1 AND run_id=$2").bind(source.key).bind(run).bind(jobs.len() as i32).bind(interval.as_secs() as f64).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}
/// Replaces a vacancy's skill tags with the catalog skills its text mentions.
pub(crate) async fn tag_vacancy(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    vacancy: Uuid,
    text: &str,
    index: &HashMap<String, Uuid>,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM vacancy_skills WHERE vacancy_id=$1")
        .bind(vacancy)
        .execute(&mut **tx)
        .await?;
    let skills: Vec<Uuid> = skills_in_text(text, index).into_iter().collect();
    if !skills.is_empty() {
        sqlx::query("INSERT INTO vacancy_skills(vacancy_id,skill_id) SELECT $1,unnest($2::uuid[])")
            .bind(vacancy)
            .bind(skills)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}
/// Re-tags every active vacancy against the current alias catalog; returns rows tagged.
pub async fn retag_all(pool: &PgPool) -> Result<u64, AppError> {
    let index = aliases(&db::skills(pool).await?);
    // ponytail: one transaction over all active rows; batch by id ranges if active vacancies exceed ~50k.
    let mut tx = pool.begin().await?;
    let rows: Vec<(Uuid, String, String, String)> = sqlx::query_as(
        "SELECT id,title,description,requirements FROM job_vacancies WHERE status='active'",
    )
    .fetch_all(&mut *tx)
    .await?;
    for (id, title, description, requirements) in &rows {
        tag_vacancy(
            &mut tx,
            *id,
            &format!("{title}\n{description}\n{requirements}"),
            &index,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(rows.len() as u64)
}
pub async fn run_pass(pool: &PgPool, settings: &Settings, force: bool) -> bool {
    let keys: Vec<&str> = settings.sources.iter().map(|s| s.key).collect();
    if sqlx::query(
        "UPDATE job_sources SET managed_by='rust' WHERE key=ANY($1) AND managed_by<>'rust'",
    )
    .bind(keys)
    .execute(pool)
    .await
    .is_err()
    {
        return false;
    }
    let Ok(collector) = Collector::new(settings.google.clone()) else {
        return false;
    };
    let mut success = true;
    for source in &settings.sources {
        if sync_source(pool, &collector, *source, settings.interval, force)
            .await
            .is_err()
        {
            success = false;
        }
    }
    success
}
pub async fn schedule(pool: PgPool, settings: Settings) {
    let mut ticker = tokio::time::interval(Duration::from_secs(60));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        run_pass(&pool, &settings, false).await;
    }
}
pub async fn start(database_url: &str, settings: Settings) -> Result<(), AppError> {
    if settings.enabled {
        let pool = db::connect_runtime_pool(database_url, db::RuntimeRole::Importer).await?;
        tokio::spawn(schedule(pool, settings));
    }
    Ok(())
}
pub async fn importer_pool() -> Result<PgPool, AppError> {
    let url =
        config::database_url().map_err(|_| AppError::Invalid("Invalid database configuration"))?;
    db::connect_runtime_pool(&url, db::RuntimeRole::Importer).await
}

#[derive(Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct VacancyItem {
    pub id: Uuid,
    #[serde(flatten)]
    pub vacancy: Vacancy,
    pub last_seen_at: DateTime<Utc>,
    /// Percent of the vacancy's detected skills the caller has; null when none detected.
    pub match_score: Option<i64>,
    pub owned_skill_ids: Vec<Uuid>,
    pub missing_skill_ids: Vec<Uuid>,
}
#[derive(Serialize, utoipa::ToSchema)]
pub struct VacancySourceOption {
    pub key: String,
    pub name: String,
}
#[derive(Serialize, utoipa::ToSchema)]
pub struct VacancyOccupationOption {
    pub id: Uuid,
    pub nama: String,
}
#[derive(Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct VacancyPage {
    pub items: Vec<VacancyItem>,
    pub total: i64,
    pub offset: i64,
    pub limit: i64,
    /// Labels for every skill id referenced by the items on this page.
    pub skills: Vec<crate::domain::Skill>,
    pub source_options: Vec<VacancySourceOption>,
    pub occupation_options: Vec<VacancyOccupationOption>,
}
#[derive(Deserialize, utoipa::IntoParams)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VacancyQuery {
    #[serde(default)]
    #[param(minimum = 0, maximum = 100000)]
    pub offset: i64,
    #[serde(default = "default_page")]
    #[param(minimum = 1, maximum = 100)]
    pub limit: i64,
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub source: String,
    /// Occupation (job category) id from `occupationOptions`; empty = all.
    #[serde(default)]
    pub occupation: String,
    /// `match` = best skill match first; empty or `recent` = newest first.
    #[serde(default)]
    pub sort: String,
}
fn default_page() -> i64 {
    20
}
impl VacancyQuery {
    /// Returns the parsed occupation filter.
    pub fn validate(&self) -> Result<Option<Uuid>, AppError> {
        let invalid = AppError::Invalid("Invalid vacancy page filters");
        if !(0..=100000).contains(&self.offset)
            || !(1..=100).contains(&self.limit)
            || self.q.len() > 120
            || self.city.len() > 200
            || self.source.len() > 40
            || (!self.source.is_empty() && !SOURCES.iter().any(|s| s.key == self.source))
            || !["", "recent", "match"].contains(&self.sort.as_str())
        {
            return Err(invalid);
        }
        if self.occupation.is_empty() {
            return Ok(None);
        }
        Uuid::parse_str(&self.occupation)
            .map(Some)
            .map_err(|_| invalid)
    }
}
/// `have` = the caller's catalog skill ids; scores and `sort=match` are computed against it.
pub async fn page(pool: &PgPool, have: &[Uuid], p: &VacancyQuery) -> Result<Value, AppError> {
    let occupation = p.validate()?;
    let pattern = format!(
        "%{}%",
        p.q.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    // RLS hides closed, expired, and >7-day unrefreshed rows. Consistent page/count snapshot.
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    const FILTER: &str = "($1='' OR v.title ILIKE $2 OR v.company ILIKE $2) AND ($3='' OR lower(v.city)=lower($3)) AND ($4='' OR v.source=$4) AND ($5::uuid IS NULL OR v.occupation_id=$5)";
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM job_vacancies v WHERE {FILTER}"
    )))
    .bind(&p.q)
    .bind(&pattern)
    .bind(&p.city)
    .bind(&p.source)
    .bind(occupation)
    .fetch_one(&mut *tx)
    .await?;
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!("SELECT jsonb_build_object('id',v.id,'source',v.source,'sourceId',v.source_id,'company',v.company,'title',v.title,'occupationId',v.occupation_id,'occupationName',o.nama,'country',v.country,'city',v.city,'region',v.region,'workplace',v.workplace,'employmentType',v.employment_type,'description',v.description,'requirements',v.requirements,'applyUrl',v.apply_url,'sourceUrl',v.source_url,'postedAt',v.posted_at,'validUntil',v.valid_until,'lastSeenAt',v.last_seen_at,'matchScore',m.score,'ownedSkillIds',m.owned,'missingSkillIds',m.missing) data FROM job_vacancies v LEFT JOIN occupations o ON o.id=v.occupation_id LEFT JOIN LATERAL (SELECT round(100.0*count(*) FILTER (WHERE skill_id=ANY($8))/nullif(count(*),0))::int AS score,coalesce(array_agg(skill_id ORDER BY skill_id) FILTER (WHERE skill_id=ANY($8)),'{{}}') AS owned,coalesce(array_agg(skill_id ORDER BY skill_id) FILTER (WHERE NOT skill_id=ANY($8)),'{{}}') AS missing FROM vacancy_skills WHERE vacancy_id=v.id) m ON true WHERE {FILTER} ORDER BY CASE WHEN $9='match' THEN m.score END DESC NULLS LAST,v.posted_at DESC NULLS LAST,v.id LIMIT $6 OFFSET $7")))
        // ponytail: lateral score per filtered row; precompute per-user scores if active vacancies exceed ~20k.
        .bind(&p.q).bind(&pattern).bind(&p.city).bind(&p.source).bind(occupation).bind(p.limit).bind(p.offset).bind(have).bind(&p.sort).fetch_all(&mut *tx).await?;
    let items: Vec<Value> = rows.iter().map(|r| r.get("data")).collect();
    // Every catalog occupation is a filter option, even before its first vacancy arrives.
    let options: Value = sqlx::query_scalar("SELECT COALESCE(jsonb_agg(jsonb_build_object('id',id,'nama',nama) ORDER BY nama),'[]') FROM occupations")
        .fetch_one(&mut *tx).await?;
    tx.commit().await?;
    let skill_ids: Vec<Uuid> = items
        .iter()
        .flat_map(|i| {
            ["ownedSkillIds", "missingSkillIds"]
                .into_iter()
                .flat_map(move |k| i[k].as_array().into_iter().flatten())
        })
        .filter_map(|v| v.as_str()?.parse().ok())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let skills = browsing::by_ids(pool, "skills", &skill_ids).await?;
    Ok(
        serde_json::json!({"items":items,"total":total,"offset":p.offset,"limit":p.limit,"occupationOptions":options,"sourceOptions":SOURCES.iter().map(|s|serde_json::json!({"key":s.key,"name":if s.company.is_empty(){s.key}else{s.company}})).collect::<Vec<_>>(),"skills":skills}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn record() -> Vacancy {
        smart_record(SOURCES[0],&json!({"id":"123456","name":"Software Engineer","location":{"country":"id","city":"Jakarta"},"visibility":"PUBLIC","active":true,"postingUrl":"https://jobs.smartrecruiters.com/Grab/123456-software-engineer","applyUrl":"https://jobs.smartrecruiters.com/Grab/123456-software-engineer?oga=true","jobAd":{"sections":{"jobDescription":{"text":"<p>Build APIs</p>"},"qualifications":{"text":"<ul><li>Rust and SQL</li></ul>"}}}})).unwrap()
    }
    #[test]
    fn filters_software_roles_and_preserves_application_links() {
        for title in [
            "Software Engineer Intern",
            "Software Quality Assurance - Manual Testing (Senior)",
            "Senior Node.js Developer",
            "Android Engineer - SDE 2",
            "Software QA Engineer (Automation)",
            "DevOps Engineer",
        ] {
            assert!(software(title), "{title}");
        }
        for title in [
            "Product Manager - Developer Experience",
            "Business Developer",
            "Sales Engineer",
            "Mechanical Engineer",
            "UX Designer",
        ] {
            assert!(!software(title), "{title}");
        }
        let job = record();
        job.validate(SOURCES[0]).unwrap();
        assert!(job.apply_url.ends_with("?oga=true"));
        assert_eq!(job.requirements, "Rust and SQL");
    }
    #[test]
    fn rejects_cross_origin_urls_and_non_indonesia_records() {
        let mut job = record();
        for url in [
            "javascript:alert(1)",
            "https://127.0.0.1/private",
            "https://jobs.smartrecruiters.com.evil.test/Grab/123",
            "https://jobs.smartrecruiters.com/Other/123",
            "https://user:pass@jobs.smartrecruiters.com/Grab/123",
        ] {
            job.apply_url = url.into();
            assert!(job.validate(SOURCES[0]).is_err());
        }
        job = record();
        job.country = "SG".into();
        assert!(job.validate(SOURCES[0]).is_err());
    }
    #[test]
    fn parses_structured_and_nested_html_descriptions_without_scripts() {
        let value=job_posting(r#"<script type="application/ld+json">{"@type":"Organization"}</script><script type='application/ld+json'>{"@graph":[{"@type":"JobPosting","description":"&lt;p&gt;Rust &amp;amp; SQL&lt;/p&gt;"}]}</script>"#).unwrap();
        assert_eq!(plain(text(&value, "description")), "Rust & SQL");
        let body=description_body(r#"<div class="description"><div><p>Build APIs</p></div><p>Rust</p><script>secret</script></div><div>Ignore</div>"#).unwrap();
        assert_eq!(plain(body), "Build APIs\nRust");
        assert!(description_body("<html>Firewall</html>").is_none());
        assert!(job_posting("<html>Firewall</html>").is_none());
    }
    // Shape copied from a live response (2026-11-08, query "developer jobs in Jakarta").
    fn google_job(title: &str, location: &str) -> Value {
        json!({"job_id":"Ml9RTTVF","job_uid":"2_QM5EX3z4Up8QJ3AAAAAA==","job_title":title,"employer_name":"Neuram.ai","job_publisher":"LinkedIn","job_employment_type":"Pekerjaan tetap","job_apply_link":"https://id.linkedin.com/jobs/view/urgent-ai-programmer-jakarta-at-neuram-ai-4485082212","job_description":"Company Description\n\nNeuram.ai builds AI.","job_is_remote":false,"job_posted_at":"4 hari yang lalu","job_posted_at_timestamp":null,"job_location":location,"job_city":null,"job_country":null,"job_highlights":{}})
    }
    #[test]
    fn maps_google_jobs_listings_to_indonesian_vacancies_only() {
        let source = SOURCES[5];
        let occupation = Uuid::from_u128(7);
        let now = DateTime::parse_from_rfc3339("2026-11-08T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let developer = Regex::new(r"(?i)\b(developer|programmer)\b").unwrap();
        let rec = |job: Value| google_record(source, &job, now, occupation, Some(&developer));
        let job = rec(google_job(
            "[Urgent] AI Programmer [Jakarta]",
            "Jakarta Barat, Kota Jakarta Barat, Daerah Khusus Ibukota Jakarta     •  melalui LinkedIn",
        ))
        .unwrap();
        assert_eq!(job.source_id, "2_QM5EX3z4Up8QJ3AAAAAA");
        assert_eq!(job.company, "Neuram.ai");
        assert_eq!(job.occupation_id, Some(occupation));
        assert_eq!(
            (job.city.as_str(), job.region.as_str()),
            ("Jakarta Barat", "Daerah Khusus Ibukota Jakarta")
        );
        assert_eq!(
            job.posted_at.unwrap().to_rfc3339(),
            "2026-11-04T00:00:00+00:00"
        );
        for location in [
            "Singapura  •  melalui LinkedIn",
            "Port Moresby, Papua New Guinea",
            "",
            "Di mana saja",
        ] {
            assert!(
                rec(google_job("Golang Backend Developer", location)).is_none(),
                "{location}"
            );
        }
        // title_pattern rejects off-category listings; no pattern accepts any title.
        assert!(rec(google_job("Sales Manager", "Jakarta")).is_none());
        assert!(
            google_record(
                source,
                &google_job("Sales Manager", "Jakarta"),
                now,
                occupation,
                None
            )
            .is_some()
        );
        let multi = rec(google_job(
            "Web Developer",
            "Kota Semarang, Jawa Tengah   (+2 lainnya)  •  melalui Indeed",
        ))
        .unwrap();
        assert_eq!(multi.region, "Jawa Tengah");
        let mut bad = google_job("Golang Backend Developer", "Bandung, Jawa Barat");
        bad["job_apply_link"] = json!("https://10.0.0.1/apply");
        assert!(rec(bad).is_none());
        assert_eq!(
            relative_date("30+ hari yang lalu", now)
                .unwrap()
                .to_rfc3339(),
            "2026-10-09T00:00:00+00:00"
        );
        assert!(relative_date("kemarin", now).is_none());
    }
    #[test]
    fn seeded_title_patterns_compile_and_feed_titles_map_to_occupations() {
        let sql = include_str!("../../migrations/202611080004_openwebninja_source.sql");
        let patterns: Vec<_> = Regex::new(r"'(\(\?i\)[^']*)'")
            .unwrap()
            .captures_iter(sql)
            .map(|c| c[1].to_owned())
            .collect();
        assert_eq!(patterns.len(), 11);
        for p in &patterns {
            Regex::new(p).unwrap_or_else(|e| panic!("{p}: {e}"));
        }
        assert_eq!(feed_occupation("Senior DevOps Engineer"), "DevOps Engineer");
        assert_eq!(feed_occupation("SRE II"), "DevOps Engineer");
        assert_eq!(feed_occupation("Data Analyst Operations"), "Data Analyst");
        assert_eq!(feed_occupation("Backend Developer"), "Software Engineer");
    }
    #[test]
    fn rejects_unbounded_pages_and_unknown_sources() {
        let mut page = VacancyQuery {
            offset: 0,
            limit: 20,
            q: "Rust".into(),
            city: "Yogyakarta".into(),
            source: "softwareseni".into(),
            occupation: "".into(),
            sort: "".into(),
        };
        assert!(page.validate().is_ok());
        page.limit = 101;
        assert!(page.validate().is_err());
        page.limit = 20;
        page.source = "untrusted".into();
        assert!(page.validate().is_err());
    }
    // Parses through actix's own Query extractor: a param the frontend sends but this struct
    // lacks fails here instead of as "Parameter tidak valid." in the browser.
    fn vacancy_query(qs: &str) -> Result<VacancyQuery, actix_web::error::QueryPayloadError> {
        actix_web::web::Query::<VacancyQuery>::from_query(qs).map(|q| q.into_inner())
    }
    #[test]
    fn accepts_occupation_filter_sent_by_frontend() {
        let id = "96f251a2-ee07-5722-9da1-5a2e83e3361d";
        // PekerjaanDetail.tsx: { occupation: id, limit: 5 }
        let page = vacancy_query(&format!("occupation={id}&limit=5")).unwrap();
        assert_eq!((page.limit, page.offset), (5, 0));
        assert_eq!(page.validate().unwrap(), Some(Uuid::parse_str(id).unwrap()));
        // Lowongan.tsx: full filter set
        let page = vacancy_query(&format!(
            "q=Rust&city=Jakarta&source=grab&occupation={id}&offset=20&limit=20"
        ))
        .unwrap();
        assert_eq!(page.validate().unwrap(), Some(Uuid::parse_str(id).unwrap()));
        // No filter: defaults, all occupations
        let page = vacancy_query("").unwrap();
        assert_eq!((page.limit, page.offset), (20, 0));
        assert_eq!(page.validate().unwrap(), None);
    }
    #[test]
    fn rejects_stale_or_malformed_vacancy_params() {
        // The old `category` param must fail loudly, not be silently ignored.
        assert!(vacancy_query("category=data-analyst&limit=5").is_err());
        assert!(vacancy_query("limit=abc").is_err());
        let page = vacancy_query("occupation=data-analyst").unwrap();
        assert!(page.validate().is_err(), "occupation must be a UUID");
        assert!(vacancy_query("sort=popular").unwrap().validate().is_err());
        assert!(vacancy_query("sort=match").unwrap().validate().is_ok());
    }
    #[tokio::test]
    #[ignore = "requires a disposable migrated TEST_DATABASE_URL; never reads .env"]
    async fn database_upserts_closes_only_on_success_and_enforces_role_isolation() {
        let url = std::env::var("TEST_DATABASE_URL").unwrap();
        let owner = PgPool::connect(&url).await.unwrap();
        let dbname: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&owner)
            .await
            .unwrap();
        assert!(dbname.contains("test"), "disposable test database required");
        sqlx::query("TRUNCATE vacancy_skills, job_vacancies")
            .execute(&owner)
            .await
            .unwrap();
        let importer = db::connect_runtime_pool(&url, db::RuntimeRole::Importer)
            .await
            .unwrap();
        let api = db::connect_runtime_pool(&url, db::RuntimeRole::Api)
            .await
            .unwrap();
        let source = SOURCES[0];
        let run = Uuid::new_v4();
        sqlx::query("UPDATE job_sources SET run_id=$1,lease_until=now()+interval '10 minutes',next_run_at=now()+interval '1 day' WHERE key='grab'").bind(run).execute(&importer).await.unwrap();
        let mut first = record();
        let mut second = record();
        second.source_id = "222222".into();
        persist(
            &importer,
            source,
            run,
            &[first.clone(), second],
            &[],
            Duration::from_secs(3600),
        )
        .await
        .unwrap();
        let run = Uuid::new_v4();
        sqlx::query("UPDATE job_sources SET run_id=$1,lease_until=now()+interval '10 minutes' WHERE key='grab'").bind(run).execute(&importer).await.unwrap();
        first.title = "Senior Software Engineer".into();
        persist(
            &importer,
            source,
            run,
            std::slice::from_ref(&first),
            &[],
            Duration::from_secs(3600),
        )
        .await
        .unwrap();
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM job_vacancies")
            .fetch_one(&importer)
            .await
            .unwrap();
        assert_eq!(count, 2);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM job_vacancies")
            .fetch_one(&api)
            .await
            .unwrap();
        assert_eq!(count, 1);
        let page = page(
            &api,
            &[],
            &VacancyQuery {
                offset: 0,
                limit: 1,
                q: "Senior".into(),
                city: "jakarta".into(),
                source: "grab".into(),
                occupation: "".into(),
                sort: "".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(page["total"], 1);
        assert_eq!(page["items"][0]["applyUrl"], first.apply_url);
        assert!(
            persist(
                &importer,
                source,
                Uuid::new_v4(),
                &[],
                &[],
                Duration::from_secs(3600)
            )
            .await
            .is_err()
        );
        let outcome = sync_source(
            &importer,
            &Collector::new(GoogleJobs::default()).unwrap(),
            source,
            Duration::from_secs(3600),
            false,
        )
        .await
        .unwrap();
        assert_eq!(outcome.status, "not_due_or_leased");
        assert!(
            sqlx::query("UPDATE job_vacancies SET title='bad'")
                .execute(&api)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("SELECT * FROM users")
                .fetch_all(&importer)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("SELECT * FROM cv_uploads")
                .fetch_all(&importer)
                .await
                .is_err()
        );
        sqlx::query("UPDATE job_vacancies SET last_seen_at=now()-interval '8 days'")
            .execute(&importer)
            .await
            .unwrap();
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM job_vacancies")
            .fetch_one(&api)
            .await
            .unwrap();
        assert_eq!(count, 0);
        sqlx::query("TRUNCATE vacancy_skills, job_vacancies")
            .execute(&owner)
            .await
            .unwrap();
        sqlx::query("UPDATE job_sources SET run_id=NULL,lease_until=NULL,next_run_at=now(),last_error=NULL,last_success_at=NULL,last_count=0").execute(&owner).await.unwrap();
    }
}
