//! Bounded, cookie-free employer vacancy importer. No credentials sent to sources.
use super::{config, db, error::AppError};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::{collections::HashSet, sync::LazyLock, time::Duration};
use uuid::Uuid;

#[derive(Clone, Copy, Debug)]
pub enum Provider {
    Smart,
    Greenhouse,
    Breezy,
}
#[derive(Clone, Copy, Debug)]
pub struct Source {
    pub key: &'static str,
    pub company: &'static str,
    pub board: &'static str,
    pub provider: Provider,
}
pub const SOURCES: [Source; 5] = [
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
];
#[derive(Clone)]
pub struct Settings {
    pub enabled: bool,
    pub interval: Duration,
    pub sources: Vec<Source>,
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
        let keys = std::env::var("JOB_SCRAPE_SOURCES")
            .unwrap_or_else(|_| "grab,cermati,dkatalis,softwareseni,kredivo".into());
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
        Ok(Self {
            enabled,
            interval: Duration::from_secs(seconds),
            sources,
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
}
static ROLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:software\s+(?:engineer\w*|develop\w*|quality\s+assurance)|(?:back[ -]?end|front[ -]?end|full[ -]?stack|mobile|android|ios|web|application)\s+(?:engineer\w*|develop\w*)|(?:[a-z.]+\s+)?developer|devops|devsecops|infraops|site\s+reliability|sre|(?:qa|test|automation)\s+engineer\w*|programmer)\b").unwrap()
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
            }
    })
}
impl Vacancy {
    fn validate(&self, source: Source) -> Result<(), AppError> {
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
}
impl Collector {
    pub fn new() -> Result<Self, AppError> {
        let client = reqwest::Client::builder()
            .user_agent("CareerBridge-Vacancy-Importer/1.0")
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(25))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| AppError::Internal)?;
        Ok(Self { client })
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
                )
            )
        {
            return Err(AppError::Invalid("Unexpected vacancy host"));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| AppError::Provider)?;
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
        };
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
            });
        }
        Ok(result)
    }
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
    let run = Uuid::new_v4();
    let claimed=sqlx::query("UPDATE job_sources SET run_id=$2,lease_until=now()+interval '10 minutes' WHERE key=$1 AND (lease_until IS NULL OR lease_until<now()) AND ($3 OR next_run_at<=now())").bind(source.key).bind(run).bind(force).execute(pool).await?.rows_affected();
    if claimed == 0 {
        return Ok(Outcome {
            source: source.key.into(),
            status: "not_due_or_leased",
            count: 0,
        });
    }
    tracing::info!(source = source.key, "vacancy collection started");
    let result = tokio::time::timeout(Duration::from_secs(300), collector.collect(source))
        .await
        .unwrap_or(Err(AppError::Provider));
    let result = match result {
        Ok(jobs) => persist(pool, source, run, &jobs, interval)
            .await
            .map(|_| jobs.len()),
        Err(e) => Err(e),
    };
    match result {
        Ok(count) => {
            tracing::info!(source = source.key, count, "vacancy import completed");
            Ok(Outcome {
                source: source.key.into(),
                status: "completed",
                count,
            })
        }
        Err(error) => {
            // AppError messages are fixed/sanitized; source bodies and URLs are never logged.
            sqlx::query("UPDATE job_sources SET lease_until=NULL,run_id=NULL,last_error=$3,next_run_at=now()+interval '15 minutes' WHERE key=$1 AND run_id=$2").bind(source.key).bind(run).bind(error.to_string()).execute(pool).await?;
            tracing::warn!(source=source.key,error=%error,"vacancy import failed; previous records preserved");
            Err(error)
        }
    }
}
async fn persist(
    pool: &PgPool,
    source: Source,
    run: Uuid,
    jobs: &[Vacancy],
    interval: Duration,
) -> Result<(), AppError> {
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
    if previous > 10 && (jobs.len() as i64) * 2 < previous {
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
        sqlx::query("INSERT INTO job_vacancies(id,source,source_id,company,title,country,city,region,workplace,employment_type,description,requirements,apply_url,source_url,posted_at,valid_until,last_seen_run) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17) ON CONFLICT(source,source_id) DO UPDATE SET company=EXCLUDED.company,title=EXCLUDED.title,country=EXCLUDED.country,city=EXCLUDED.city,region=EXCLUDED.region,workplace=EXCLUDED.workplace,employment_type=EXCLUDED.employment_type,description=EXCLUDED.description,requirements=EXCLUDED.requirements,apply_url=EXCLUDED.apply_url,source_url=EXCLUDED.source_url,posted_at=EXCLUDED.posted_at,valid_until=EXCLUDED.valid_until,last_seen_run=EXCLUDED.last_seen_run,last_seen_at=now(),updated_at=now(),status='active'")
            .bind(id).bind(source.key).bind(&job.source_id).bind(&job.company).bind(&job.title).bind(&job.country).bind(&job.city).bind(&job.region).bind(&job.workplace).bind(&job.employment_type).bind(&job.description).bind(&job.requirements).bind(&job.apply_url).bind(&job.source_url).bind(job.posted_at).bind(job.valid_until).bind(run).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE job_vacancies SET status='closed',updated_at=now() WHERE source=$1 AND last_seen_run!=$2 AND status='active'").bind(source.key).bind(run).execute(&mut *tx).await?;
    sqlx::query("UPDATE job_sources SET last_success_at=now(),last_count=$3,last_error=NULL,lease_until=NULL,run_id=NULL,next_run_at=now()+make_interval(secs=>$4) WHERE key=$1 AND run_id=$2").bind(source.key).bind(run).bind(jobs.len() as i32).bind(interval.as_secs() as f64).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}
pub async fn run_pass(pool: &PgPool, settings: &Settings, force: bool) -> bool {
    let Ok(collector) = Collector::new() else {
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
}
#[derive(Serialize, utoipa::ToSchema)]
pub struct VacancyPage {
    pub items: Vec<VacancyItem>,
    pub total: i64,
    pub offset: i64,
    pub limit: i64,
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
}
fn default_page() -> i64 {
    20
}
impl VacancyQuery {
    pub fn validate(&self) -> Result<(), AppError> {
        if !(0..=100000).contains(&self.offset)
            || !(1..=100).contains(&self.limit)
            || self.q.len() > 120
            || self.city.len() > 200
            || self.source.len() > 40
            || (!self.source.is_empty() && !SOURCES.iter().any(|s| s.key == self.source))
        {
            return Err(AppError::Invalid("Invalid vacancy page filters"));
        }
        Ok(())
    }
}
pub async fn page(pool: &PgPool, p: &VacancyQuery) -> Result<Value, AppError> {
    p.validate()?;
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
    let total:i64=sqlx::query_scalar("SELECT count(*) FROM job_vacancies WHERE ($1='' OR title ILIKE $2 OR company ILIKE $2) AND ($3='' OR lower(city)=lower($3)) AND ($4='' OR source=$4)").bind(&p.q).bind(&pattern).bind(&p.city).bind(&p.source).fetch_one(&mut *tx).await?;
    let rows=sqlx::query("SELECT jsonb_build_object('id',id,'source',source,'sourceId',source_id,'company',company,'title',title,'country',country,'city',city,'region',region,'workplace',workplace,'employmentType',employment_type,'description',description,'requirements',requirements,'applyUrl',apply_url,'sourceUrl',source_url,'postedAt',posted_at,'validUntil',valid_until,'lastSeenAt',last_seen_at) data FROM job_vacancies WHERE ($1='' OR title ILIKE $2 OR company ILIKE $2) AND ($3='' OR lower(city)=lower($3)) AND ($4='' OR source=$4) ORDER BY posted_at DESC NULLS LAST,id LIMIT $5 OFFSET $6").bind(&p.q).bind(&pattern).bind(&p.city).bind(&p.source).bind(p.limit).bind(p.offset).fetch_all(&mut *tx).await?;
    let items: Vec<Value> = rows.iter().map(|r| r.get("data")).collect();
    tx.commit().await?;
    Ok(serde_json::json!({"items":items,"total":total,"offset":p.offset,"limit":p.limit}))
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
    #[test]
    fn rejects_unbounded_pages_and_unknown_sources() {
        let mut page = VacancyQuery {
            offset: 0,
            limit: 20,
            q: "Rust".into(),
            city: "Yogyakarta".into(),
            source: "softwareseni".into(),
        };
        assert!(page.validate().is_ok());
        page.limit = 101;
        assert!(page.validate().is_err());
        page.limit = 20;
        page.source = "untrusted".into();
        assert!(page.validate().is_err());
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
        sqlx::query("TRUNCATE job_vacancies")
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
            &VacancyQuery {
                offset: 0,
                limit: 1,
                q: "Senior".into(),
                city: "jakarta".into(),
                source: "grab".into(),
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
                Duration::from_secs(3600)
            )
            .await
            .is_err()
        );
        let outcome = sync_source(
            &importer,
            &Collector::new().unwrap(),
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
        sqlx::query("TRUNCATE job_vacancies")
            .execute(&owner)
            .await
            .unwrap();
        sqlx::query("UPDATE job_sources SET run_id=NULL,lease_until=NULL,next_run_at=now(),last_error=NULL,last_success_at=NULL,last_count=0").execute(&owner).await.unwrap();
    }
}
