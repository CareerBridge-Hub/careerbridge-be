//! Database-loading path of the Python jobs extractor. Source IDs and UUIDs are unchanged.
use super::{geo, http::Http};
use crate::infrastructure::{
    error::AppError,
    jobs::{self, SearchQuery, Source, Vacancy},
};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{collections::HashSet, time::Duration};
use uuid::Uuid;

pub fn allowed_posting(source: &str, u: &reqwest::Url) -> bool {
    let host = u.host_str().unwrap_or("");
    match source {
        "linkedin" => host == "www.linkedin.com" && u.path().starts_with("/jobs/view/"),
        "kalibrr" => matches!(host, "www.kalibrr.com" | "kalibrr.com"),
        "techinasia" => matches!(host, "www.techinasia.com" | "techinasia.com"),
        "jobstreet" => host == "id.jobstreet.com",
        "glints" => host == "glints.com",
        "dealls" => host == "dealls.com",
        "kitalulus" => host == "www.kitalulus.com",
        "karir" => host == "karir.com",
        "indeed" => host == "id.indeed.com",
        "glassdoor" => matches!(host, "www.glassdoor.com" | "www.glassdoor.co.id"),
        "remotive" => host == "remotive.com",
        "remoteok" => host == "remoteok.com",
        "arbeitnow" => host == "www.arbeitnow.com",
        "hackernews" => host == "news.ycombinator.com",
        "adzuna" => u
            .domain()
            .is_some_and(|d| d.ends_with(".adzuna.com") || d == "adzuna.com"),
        _ if source.starts_with("greenhouse-") => {
            matches!(host, "boards.greenhouse.io" | "job-boards.greenhouse.io")
        }
        _ if source.starts_with("lever-") => host == "jobs.lever.co",
        _ if source.starts_with("workable-") => matches!(host, "apply.workable.com" | "wrkbl.ink"),
        _ if source.starts_with("ashby-") => host == "jobs.ashbyhq.com",
        _ if source.starts_with("smartrecruiters-") => host == "jobs.smartrecruiters.com",
        _ => false,
    }
}

#[derive(Default, Debug)]
pub struct RawJob {
    pub id: String,
    pub title: String,
    pub company: String,
    pub location: String,
    pub url: String,
    pub description: String,
    pub tags: Vec<String>,
    pub employment: String,
    pub posted: String,
    pub remote: Option<bool>,
}
fn s(v: &Value, path: &str) -> String {
    v.pointer(path)
        .map(|v| {
            v.as_str().map(str::to_owned).unwrap_or_else(|| {
                if v.is_number() {
                    v.to_string()
                } else {
                    String::new()
                }
            })
        })
        .unwrap_or_default()
}
fn pick(v: &Value, paths: &[&str]) -> String {
    paths
        .iter()
        .map(|p| s(v, p))
        .find(|s| !s.is_empty())
        .unwrap_or_default()
}
fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().or_else(|| v["name"].as_str()))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
fn join(v: &Value, paths: &[&str]) -> String {
    paths
        .iter()
        .map(|p| s(v, p))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}
pub fn timestamp(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|d| d.with_timezone(&Utc))
        .or_else(|| {
            NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|d| d.and_utc())
        })
        .or_else(|| {
            raw.parse::<i64>().ok().and_then(|n| {
                Utc.timestamp_opt(if n > 10_000_000_000 { n / 1000 } else { n }, 0)
                    .single()
            })
        })
}
fn css(raw: &str) -> Selector {
    Selector::parse(raw).expect("constant selector")
}
fn node_text(node: ElementRef<'_>, selector: &str) -> String {
    node.select(&css(selector))
        .next()
        .map(|n| {
            n.text()
                .collect::<Vec<_>>()
                .join(" ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}
pub fn linkedin_cards(raw: &str) -> Vec<RawJob> {
    let html = Html::parse_fragment(raw);
    html.select(&css("[data-entity-urn^='urn:li:jobPosting:']"))
        .filter_map(|n| {
            let id = n
                .value()
                .attr("data-entity-urn")?
                .rsplit(':')
                .next()?
                .to_owned();
            if !id.bytes().all(|b| b.is_ascii_digit()) || id.is_empty() {
                return None;
            }
            Some(RawJob {
                url: format!("https://www.linkedin.com/jobs/view/{id}"),
                id,
                title: node_text(n, ".base-search-card__title"),
                company: node_text(n, ".base-search-card__subtitle"),
                location: node_text(n, ".job-search-card__location"),
                posted: n
                    .select(&css("time"))
                    .next()
                    .and_then(|t| t.value().attr("datetime"))
                    .unwrap_or("")
                    .into(),
                ..Default::default()
            })
        })
        .collect()
}
pub fn linkedin_description(raw: &str) -> Option<(String, String)> {
    let html = Html::parse_document(raw);
    let node = html.select(&css(".show-more-less-html__markup")).next()?;
    let description = jobs::plain(&node.inner_html());
    if description.is_empty() || description.chars().count() > 100_000 {
        return None;
    }
    let employment = html
        .select(&css(".description__job-criteria-item"))
        .find_map(|n| {
            (node_text(n, ".description__job-criteria-subheader")
                .eq_ignore_ascii_case("employment type"))
            .then(|| node_text(n, ".description__job-criteria-text"))
        })
        .unwrap_or_default();
    Some((description, employment))
}
pub fn keyword(raw: &str) -> String {
    Regex::new(r"(?i)\s+jobs?\s+in\s+.+$")
        .unwrap()
        .replace(raw, "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
/// Maps vacancy titles to occupations through the search queue's title patterns.
pub struct Classifier(Vec<(Uuid, Uuid, bool, Option<Regex>)>);
impl Classifier {
    pub fn new(queries: &[SearchQuery]) -> Self {
        Self(
            queries
                .iter()
                .map(|q| {
                    (
                        q.id,
                        q.occupation_id,
                        q.title_pattern.is_none(),
                        q.title_pattern.as_ref().and_then(|p| Regex::new(p).ok()),
                    )
                })
                .collect(),
        )
    }
    pub fn occupation(&self, title: &str, preferred: Option<Uuid>) -> Option<Uuid> {
        // A query without a pattern accepts every title it finds.
        if let Some((_, occupation, _, _)) = self
            .0
            .iter()
            .find(|(id, _, unfiltered, _)| Some(*id) == preferred && *unfiltered)
        {
            return Some(*occupation);
        }
        // The longest match wins, so "Asisten Apoteker" found by the "apoteker" query is still
        // Asisten Apoteker; the query that found the title only breaks ties.
        self.0
            .iter()
            .filter_map(|(id, occupation, _, regex)| {
                let n = regex
                    .as_ref()?
                    .find_iter(title)
                    .map(|m| m.len())
                    .sum::<usize>();
                (n > 0).then_some((*occupation, (n, Some(*id) == preferred)))
            })
            .max_by_key(|(_, rank)| *rank)
            .map(|(id, _)| id)
    }
}
pub fn normalize(
    raw: RawJob,
    source: Source,
    preferred: Option<&SearchQuery>,
    all: &[SearchQuery],
) -> Option<Vacancy> {
    let mut queries = all.to_vec();
    if let Some(q) = preferred
        && !queries.iter().any(|p| p.id == q.id)
    {
        queries.push(q.clone());
    }
    let occupation = Classifier::new(&queries).occupation(&raw.title, preferred.map(|q| q.id))?;
    normalized(raw, source, occupation)
}
fn normalized(raw: RawJob, source: Source, occupation: Uuid) -> Option<Vacancy> {
    let location = raw.location.to_lowercase();
    let (city, region) = geo::city_region(&raw.location);
    if region.is_empty() && !location.contains("indonesia") {
        return None;
    }
    let mut description = jobs::plain(&raw.description);
    let requirements = raw.tags.join("\n");
    if !matches!(
        source.key,
        "linkedin" | "kitalulus" | "karir" | "indeed" | "glassdoor"
    ) && description.chars().count() < 200
        && raw.tags.len() < 2
    {
        return None;
    }
    if description.is_empty() {
        description = format!(
            "{}\n{}\n{}\nDeskripsi lengkap: {}",
            raw.title, raw.company, raw.location, raw.url
        );
    }
    let job = Vacancy {
        source: source.key.into(),
        source_id: raw
            .id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .take(128)
            .collect(),
        title: raw
            .title
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(300)
            .collect(),
        company: raw
            .company
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(200)
            .collect(),
        country: "ID".into(),
        city,
        region,
        workplace: if location.contains("hybrid") {
            "hybrid"
        } else {
            match raw.remote {
                Some(true) => "remote",
                Some(false) => "onsite",
                None => "",
            }
        }
        .into(),
        employment_type: raw.employment.chars().take(100).collect(),
        description: description.chars().take(100_000).collect(),
        requirements: requirements.chars().take(100_000).collect(),
        apply_url: raw.url.clone(),
        source_url: raw.url,
        posted_at: timestamp(&raw.posted),
        valid_until: None,
        occupation_id: Some(occupation),
    };
    job.validate(source).ok()?;
    Some(job)
}

fn rows<'a>(v: &'a Value, path: &str) -> Result<&'a Vec<Value>, AppError> {
    v.pointer(path)
        .and_then(Value::as_array)
        .ok_or(AppError::Invalid("Job feed schema changed"))
}
fn map_record(source: Source, j: &Value, body: &Value) -> RawJob {
    let mut r = RawJob {
        id: pick(j, &["/id", "/shortcode"]),
        ..Default::default()
    };
    match source.key {
        "kalibrr" => {
            r.title = s(j, "/name");
            r.company = pick(j, &["/company_name", "/company/name"]);
            r.url = format!(
                "https://www.kalibrr.com/c/{}/jobs/{}/{}",
                s(j, "/company/code"),
                r.id,
                s(j, "/slug")
            );
            r.location = join(
                j,
                &[
                    "/google_location/address_components/city",
                    "/google_location/address_components/region",
                    "/google_location/address_components/country",
                ],
            );
            r.description = s(j, "/description");
            r.posted = s(j, "/activation_date");
            r.employment = s(j, "/tenure");
            r.remote = j["is_work_from_home"].as_bool();
        }
        "techinasia" => {
            r.title = s(j, "/title");
            r.company = s(j, "/company/name");
            r.url = format!("https://www.techinasia.com/jobs/{}", r.id);
            r.location = join(j, &["/city/name", "/city/country/name"]);
            r.description = s(j, "/description");
            r.posted = s(j, "/published_at");
            r.employment = s(j, "/job_type/name");
            r.remote = j["is_remote"].as_bool();
            r.tags = j["job_skills"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|v| pick(v, &["/skill/name", "/name"]))
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
        }
        "jobstreet" => {
            r.title = s(j, "/title");
            r.company = pick(j, &["/companyName", "/advertiser/description"]);
            r.url = format!("https://id.jobstreet.com/job/{}", r.id);
            r.location = j["locations"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|v| s(v, "/label"))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            r.description = std::iter::once(s(j, "/teaser"))
                .chain(strings(&j["bulletPoints"]))
                .collect::<Vec<_>>()
                .join("\n");
            r.posted = s(j, "/listingDate");
            r.employment = strings(&j["workTypes"]).join(", ");
        }
        "dealls" => {
            r.title = s(j, "/role");
            r.company = s(j, "/company/name");
            r.url = format!(
                "https://dealls.com/loker/{}~{}",
                s(j, "/slug"),
                s(j, "/company/slug")
            );
            r.location = join(j, &["/city/name", "/country/name"]);
            r.posted = s(j, "/publishedAt");
            r.tags = strings(&j["skills"]);
            r.employment = strings(&j["employmentTypes"]).join(", ");
            r.remote = Some(s(j, "/workplaceType") == "remote");
        }
        "glints" => {
            r.title = s(j, "/title");
            r.company = s(j, "/company/name");
            r.url = format!("https://glints.com/id/en/opportunities/jobs/job/{}", r.id);
            r.location = join(j, &["/location/formattedName", "/country/name"]);
            r.posted = s(j, "/createdAt");
            r.employment = s(j, "/type");
            r.remote = Some(s(j, "/workArrangementOption") == "REMOTE");
            r.tags = j["skills"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|v| s(v, "/skill/name"))
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
        }
        "kitalulus" => {
            r.title = s(j, "/positionName");
            r.company = s(j, "/company/name");
            r.url = format!(
                "https://www.kitalulus.com/lowongan/detail/{}",
                s(j, "/slug")
            );
            r.location = join(j, &["/city/name", "/province/name"]);
            r.employment = s(j, "/typeStr");
            r.posted = relative_posted(&s(j, "/updatedAtStr"));
            r.tags = vec![s(j, "/educationLevelStr")]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect();
            r.description = format!(
                "{}\n{}\n{}",
                r.title,
                r.employment,
                s(j, "/jobRole/displayName")
            );
        }
        "karir" => {
            r.title = s(j, "/job_position");
            r.company = s(j, "/company_name");
            r.url = format!("https://karir.com/opportunities/{}", r.id);
            let parts: Vec<String> = s(j, "/description")
                .split('•')
                .map(str::trim)
                .map(str::to_owned)
                .collect();
            r.location = parts
                .first()
                .cloned()
                .unwrap_or_default()
                .trim_start_matches("Prov. ")
                .into();
            r.employment = parts.get(1).cloned().unwrap_or_default();
            r.description = s(j, "/description");
            r.posted = s(j, "/posted_at");
        }
        "remotive" => {
            r.title = s(j, "/title");
            r.company = s(j, "/company_name");
            r.url = s(j, "/url");
            r.location = s(j, "/candidate_required_location");
            r.description = s(j, "/description");
            r.posted = s(j, "/publication_date");
            r.tags = strings(&j["tags"]);
            r.employment = s(j, "/job_type");
            r.remote = Some(true);
        }
        "remoteok" => {
            r.title = s(j, "/position");
            r.company = s(j, "/company");
            r.url = s(j, "/url");
            r.location = s(j, "/location");
            r.description = s(j, "/description");
            r.posted = s(j, "/date");
            r.tags = strings(&j["tags"]);
            r.remote = Some(true);
        }
        "arbeitnow" => {
            r.id = s(j, "/slug");
            r.title = s(j, "/title");
            r.company = s(j, "/company_name");
            r.url = s(j, "/url");
            r.location = s(j, "/location");
            r.description = s(j, "/description");
            r.posted = s(j, "/created_at");
            r.tags = strings(&j["tags"]);
            r.employment = strings(&j["job_types"]).join(", ");
            r.remote = j["remote"].as_bool();
        }
        "hackernews" => {
            let text = jobs::plain(&s(j, "/text"));
            let header: Vec<&str> = text
                .lines()
                .next()
                .unwrap_or("")
                .split('|')
                .map(str::trim)
                .collect();
            r.company = header.first().unwrap_or(&"").to_string();
            r.title = header.get(1).unwrap_or(&"").to_string();
            r.location = header.get(2).unwrap_or(&"").to_string();
            r.url = format!("https://news.ycombinator.com/item?id={}", r.id);
            r.posted = s(j, "/created_at");
            r.remote = Some(text.to_lowercase().contains("remote"));
            r.description = text;
        }
        "adzuna" => {
            r.title = s(j, "/title");
            r.company = s(j, "/company/display_name");
            r.url = s(j, "/redirect_url");
            r.location = s(j, "/location/display_name");
            r.description = s(j, "/description");
            r.posted = s(j, "/created");
            r.employment = pick(j, &["/contract_time", "/contract_type"]);
        }
        _ if source.key.starts_with("greenhouse-") => {
            r.title = s(j, "/title");
            r.company = pick(j, &["/company_name"]);
            if r.company.is_empty() {
                r.company = source.company.into();
            }
            r.url = s(j, "/absolute_url");
            r.location = s(j, "/location/name");
            r.description = s(j, "/content");
            r.posted = pick(j, &["/first_published", "/updated_at"]);
        }
        _ if source.key.starts_with("lever-") => {
            r.title = s(j, "/text");
            r.company = source.company.into();
            r.url = s(j, "/hostedUrl");
            r.location = s(j, "/categories/location");
            r.description = pick(j, &["/descriptionPlain", "/description"]);
            if let Some(lists) = j["lists"].as_array() {
                for l in lists {
                    r.description.push_str(&s(l, "/content"));
                }
            }
            r.posted = s(j, "/createdAt");
            r.employment = s(j, "/categories/commitment");
            r.remote = Some(s(j, "/workplaceType") == "remote");
        }
        _ if source.key.starts_with("workable-") => {
            r.title = s(j, "/title");
            r.company = s(body, "/name");
            if r.company.is_empty() {
                r.company = source.company.into();
            }
            r.url = pick(j, &["/url", "/shortlink"]);
            r.location = join(j, &["/city", "/country"]);
            r.description = s(j, "/description");
            r.posted = pick(j, &["/published_on", "/created_at"]);
            r.employment = s(j, "/employment_type");
            r.remote = j["telecommuting"].as_bool();
        }
        _ if source.key.starts_with("ashby-") => {
            r.title = s(j, "/title");
            r.company = source.company.into();
            r.url = s(j, "/jobUrl");
            r.location = s(j, "/location");
            r.description = pick(j, &["/descriptionPlain", "/descriptionHtml"]);
            r.posted = s(j, "/publishedAt");
            r.employment = s(j, "/employmentType");
            r.remote = j["isRemote"].as_bool();
        }
        _ if source.key.starts_with("smartrecruiters-") => {
            r.title = s(j, "/name");
            r.company = pick(j, &["/company/name"]);
            if r.company.is_empty() {
                r.company = source.company.into();
            }
            r.url = format!("https://jobs.smartrecruiters.com/{}/{}", source.board, r.id);
            r.location = join(j, &["/location/city", "/location/country"]);
            r.posted = s(j, "/releasedDate");
            r.employment = s(j, "/typeOfEmployment/label");
            r.remote = j["location"]["remote"].as_bool();
            r.description = j["jobAd"]["sections"]
                .as_object()
                .map(|o| {
                    o.values()
                        .map(|v| s(v, "/text"))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
        }
        _ => {}
    }
    r
}
const GLINTS: &str = "query searchJobs($data: JobSearchConditionInput!) { searchJobs(data: $data) { jobsInPage { id title workArrangementOption createdAt type company { name } country { name } location { formattedName } skills { skill { name } } } } }";
const KITALULUS: &str = "query Vacancies($keyword: String, $pagination: CommonFilter, $filters: [SearchCategoryFilter!], $intent: VacancySearchIntentInput) { vacanciesV5(keyword: $keyword, pagination: $pagination, filters: $filters, intent: $intent) { hasNextPage list { id slug positionName isClosed educationLevelStr minExperience typeStr updatedAtStr company { name } province { name } city { name } jobRole { displayName } } } }";
fn relative_posted(raw: &str) -> String {
    let now = Utc::now();
    let lower = raw.to_lowercase();
    if lower.contains("hari ini") {
        return now.to_rfc3339();
    }
    if lower.contains("kemarin") {
        return (now - chrono::Duration::days(1)).to_rfc3339();
    }
    if let Some(c) = Regex::new(r"(\d+)\s+(hari|minggu|bulan)")
        .unwrap()
        .captures(&lower)
    {
        let days = c[1].parse::<i64>().unwrap_or(0)
            * match &c[2] {
                "minggu" => 7,
                "bulan" => 30,
                _ => 1,
            };
        return (now - chrono::Duration::days(days.min(3650))).to_rfc3339();
    }
    String::new()
}
fn html_jobs(source: Source, raw: &str) -> Result<Vec<RawJob>, AppError> {
    if source.key == "indeed" {
        let prefix =
            Regex::new(r#"window\.mosaic\.providerData\["mosaic-provider-jobcards"\]\s*=\s*"#)
                .unwrap();
        let start = prefix
            .find(raw)
            .ok_or(AppError::Invalid("Indeed returned no public job data"))?
            .end();
        use serde::Deserialize;
        let value = Value::deserialize(&mut serde_json::Deserializer::from_str(&raw[start..]))
            .map_err(|_| AppError::Provider)?;
        return rows(&value, "/metaData/mosaicProviderJobCardsModel/results")?
            .iter()
            .map(|j| {
                Ok(RawJob {
                    id: s(j, "/jobkey"),
                    title: pick(j, &["/displayTitle", "/title"]),
                    company: s(j, "/company"),
                    url: format!("https://id.indeed.com/viewjob?jk={}", s(j, "/jobkey")),
                    location: s(j, "/formattedLocation"),
                    description: s(j, "/snippet"),
                    posted: s(j, "/pubDate"),
                    employment: strings(&j["jobTypes"]).join(", "),
                    ..Default::default()
                })
            })
            .collect();
    }
    let doc = Html::parse_document(raw);
    let mut result = Vec::new();
    for node in doc.select(&css("li[data-test='jobListing']")) {
        let Some(link) = node.select(&css("a[data-test='job-title']")).next() else {
            continue;
        };
        let href = link.value().attr("href").unwrap_or("");
        let url = reqwest::Url::parse("https://www.glassdoor.com")
            .unwrap()
            .join(href)
            .map_err(|_| AppError::Provider)?;
        result.push(RawJob {
            id: node.value().attr("data-jobid").unwrap_or("").into(),
            title: link.text().collect::<Vec<_>>().join(" "),
            company: node_text(node, "[class*='EmployerProfile_compactEmployerName']"),
            location: node_text(node, "[data-test='emp-location']"),
            description: node_text(node, "[data-test='descSnippet']"),
            url: url.into(),
            ..Default::default()
        });
    }
    if result.is_empty() {
        return Err(AppError::Invalid("Glassdoor returned no public job data"));
    }
    Ok(result)
}
async fn fetch(
    http: &Http,
    source: Source,
    term: &str,
    page: usize,
) -> Result<(Vec<RawJob>, bool), AppError> {
    if source.key == "linkedin" {
        let (status, html) = http
            .request(
                "https://www.linkedin.com/jobs-guest/jobs/api/seeMoreJobPostings/search",
                &[
                    ("keywords", term.into()),
                    ("location", "Indonesia".into()),
                    ("geoId", "102478259".into()),
                    ("start", (page * 10).to_string()),
                    ("f_TPR", "r86400".into()),
                ],
                None,
            )
            .await?;
        if status != 200 {
            return Err(AppError::Provider);
        }
        let jobs = linkedin_cards(&html);
        if jobs.is_empty() && !html.trim().is_empty() && !html.contains("no-results") {
            return Err(AppError::Invalid(
                "LinkedIn search returned unexpected markup",
            ));
        }
        let more = !jobs.is_empty();
        return Ok((jobs, more));
    }
    if matches!(source.key, "indeed" | "glassdoor") {
        let url = if source.key == "indeed" {
            "https://id.indeed.com/jobs".to_owned()
        } else {
            let slug = Regex::new("[^a-z0-9]+")
                .unwrap()
                .replace_all(term, "-")
                .into_owned();
            format!(
                "https://www.glassdoor.com/Job/indonesia-{slug}-jobs-SRCH_IL.0,9_IN113_KO10,{}.htm",
                10 + slug.len()
            )
        };
        let (status, body) = http
            .request(
                &url,
                &if source.key == "indeed" {
                    vec![("q", term.into()), ("l", "Jakarta".into())]
                } else {
                    vec![]
                },
                None,
            )
            .await?;
        if status != 200 {
            return Err(AppError::Provider);
        }
        return Ok((html_jobs(source, &body)?, false));
    }
    if source.key == "hackernews" {
        let search = http
            .json(
                "https://hn.algolia.com/api/v1/search_by_date",
                &[
                    ("tags", "story,author_whoishiring".into()),
                    ("query", "who is hiring".into()),
                    ("hitsPerPage", "5".into()),
                ],
                None,
            )
            .await?;
        let story = rows(&search, "/hits")?
            .iter()
            .find(|v| s(v, "/title").to_lowercase().contains("who is hiring"))
            .ok_or(AppError::Provider)?;
        let id = s(story, "/objectID");
        if !id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(AppError::Provider);
        }
        let body = http
            .json(
                &format!("https://hn.algolia.com/api/v1/items/{id}"),
                &[],
                None,
            )
            .await?;
        return Ok((
            rows(&body, "/children")?
                .iter()
                .take(200)
                .map(|j| map_record(source, j, &body))
                .collect(),
            false,
        ));
    }
    let (url, params, body, path, size) = match source.key {
        "kalibrr" => (
            "https://www.kalibrr.com/kjs/job_board/search".into(),
            vec![
                ("limit", "50".into()),
                ("offset", (page * 50).to_string()),
                ("text", term.into()),
            ],
            None,
            "/jobs",
            50,
        ),
        "techinasia" => (
            "https://www.techinasia.com/api/2.0/job-postings".into(),
            vec![
                ("page", (page + 1).to_string()),
                ("per_page", "30".into()),
                ("country_name[]", "Indonesia".into()),
            ],
            None,
            "/data",
            30,
        ),
        "jobstreet" => (
            "https://id.jobstreet.com/api/jobsearch/v5/search".into(),
            vec![
                ("siteKey", "ID-Main".into()),
                ("locale", "en-ID".into()),
                ("page", (page + 1).to_string()),
                ("pageSize", "30".into()),
                ("keywords", term.into()),
            ],
            None,
            "/data",
            30,
        ),
        "dealls" => (
            "https://api.sejutacita.id/v1/explore-job/job".into(),
            vec![
                ("page", (page + 1).to_string()),
                ("limit", "30".into()),
                ("published", "true".into()),
                ("status", "active".into()),
                ("search", term.into()),
                ("sortParam", "mostRelevant".into()),
                ("sortBy", "asc".into()),
                ("boostTheBoostedJob", "true".into()),
            ],
            None,
            "/data/docs",
            30,
        ),
        "glints" => (
            "https://glints.com/api/v2/graphql".into(),
            vec![],
            Some(
                json!({"operationName":"searchJobs","query":GLINTS,"variables":{"data":{"SearchTerm":term,"CountryCode":"ID","limit":30,"offset":0,"includeExternalJobs":true,"sources":["NATIVE","SUPER_POWERED"]}}}),
            ),
            "/data/searchJobs/jobsInPage",
            usize::MAX,
        ),
        "kitalulus" => (
            "https://gql.kitalulus.com/graphql".into(),
            vec![],
            Some(
                json!({"operationName":"Vacancies","query":KITALULUS,"variables":{"keyword":term,"pagination":{"page":page,"limit":30},"filters":[{"key":"sortBy","value":["isHighlighted"]}],"intent":{"type":"KEYWORD"}}}),
            ),
            "/data/vacanciesV5/list",
            30,
        ),
        "karir" => (
            "https://gateway2-beta.karir.com/v2/search/opportunities".into(),
            vec![],
            Some(
                json!({"keyword":term,"locale":"id","limit":20,"offset":page*20,"sort_order":"newest","is_opportunity":true}),
            ),
            "/data/opportunities",
            20,
        ),
        "remotive" => (
            "https://remotive.com/api/remote-jobs".into(),
            vec![("limit", "100".into())],
            None,
            "/jobs",
            usize::MAX,
        ),
        "remoteok" => (
            "https://remoteok.com/api".into(),
            vec![],
            None,
            "",
            usize::MAX,
        ),
        "arbeitnow" => (
            "https://www.arbeitnow.com/api/job-board-api".into(),
            vec![("page", (page + 1).to_string())],
            None,
            "/data",
            usize::MAX,
        ),
        "adzuna" => {
            let id = std::env::var("ADZUNA_APP_ID")
                .map_err(|_| AppError::Invalid("ADZUNA_APP_ID required"))?;
            let key = std::env::var("ADZUNA_APP_KEY")
                .map_err(|_| AppError::Invalid("ADZUNA_APP_KEY required"))?;
            (
                format!("https://api.adzuna.com/v1/api/jobs/sg/search/{}", page + 1),
                vec![
                    ("app_id", id),
                    ("app_key", key),
                    ("results_per_page", "50".into()),
                    ("what", term.into()),
                ],
                None,
                "/results",
                50,
            )
        }
        _ if source.key.starts_with("greenhouse-") => (
            format!(
                "https://boards-api.greenhouse.io/v1/boards/{}/jobs",
                source.board
            ),
            vec![("content", "true".into())],
            None,
            "/jobs",
            usize::MAX,
        ),
        _ if source.key.starts_with("lever-") => (
            format!("https://api.lever.co/v0/postings/{}", source.board),
            vec![("mode", "json".into())],
            None,
            "",
            usize::MAX,
        ),
        _ if source.key.starts_with("workable-") => (
            format!(
                "https://apply.workable.com/api/v1/widget/accounts/{}",
                source.board
            ),
            vec![("details", "true".into())],
            None,
            "/jobs",
            usize::MAX,
        ),
        _ if source.key.starts_with("ashby-") => (
            format!(
                "https://api.ashbyhq.com/posting-api/job-board/{}",
                source.board
            ),
            vec![("includeCompensation", "true".into())],
            None,
            "/jobs",
            usize::MAX,
        ),
        _ if source.key.starts_with("smartrecruiters-") => (
            format!(
                "https://api.smartrecruiters.com/v1/companies/{}/postings",
                source.board
            ),
            vec![
                ("limit", "100".into()),
                ("offset", (page * 100).to_string()),
            ],
            None,
            "/content",
            100,
        ),
        _ => return Err(AppError::Invalid("Unknown native job source")),
    };
    let data = http.json(&url, &params, body.as_ref()).await?;
    let list = rows(&data, path)?;
    let mut result = Vec::new();
    for j in list {
        if source.key == "remoteok" && j.get("id").is_none() {
            continue;
        }
        if source.key == "kitalulus"
            && (j["isClosed"].as_bool() == Some(true) || s(j, "/jobRole/displayName") == "MisiSeru")
        {
            continue;
        }
        let mut raw = map_record(source, j, &data);
        if source.key.starts_with("smartrecruiters-")
            && raw
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            let full = http
                .json(
                    &format!(
                        "https://api.smartrecruiters.com/v1/companies/{}/postings/{}",
                        source.board, raw.id
                    ),
                    &[],
                    None,
                )
                .await?;
            raw = map_record(source, &full, &data);
        }
        result.push(raw);
    }
    Ok((result, list.len() >= size))
}

/// One search page, no database access or billed API. ATS feeds may also fetch details.
pub async fn preview(source: Source, term: &str) -> Result<Vec<Vacancy>, AppError> {
    if !matches!(source.provider, jobs::Provider::Extract) || source.key == "adzuna" {
        return Err(AppError::Invalid("Use an unmetered native source"));
    }
    let http = Http::new(Duration::from_secs(1))?;
    let (raw, _) = fetch(&http, source, term, 0).await?;
    let q = SearchQuery {
        id: Uuid::nil(),
        occupation_id: Uuid::nil(),
        keyword: term.into(),
        title_pattern: None,
    };
    Ok(raw
        .into_iter()
        .filter_map(|r| normalize(r, source, Some(&q), &[]))
        .collect())
}

pub async fn collect(
    pool: &PgPool,
    source: Source,
) -> Result<(Vec<Vacancy>, Vec<(Uuid, i32)>), AppError> {
    let http = Http::new(Duration::from_secs(if source.key == "linkedin" {
        5
    } else {
        1
    }))?;
    let all = jobs::next_queries(pool, 1000).await?;
    if all.is_empty() {
        return Err(AppError::Invalid("No enabled job search terms"));
    }
    let board = !source.board.is_empty()
        || matches!(
            source.key,
            "techinasia" | "remotive" | "remoteok" | "arbeitnow" | "hackernews"
        );
    let batch: Vec<SearchQuery> = if board {
        vec![]
    } else {
        sqlx::query_as("SELECT q.id,q.occupation_id,q.keyword,q.title_pattern FROM job_search_queries q LEFT JOIN extract_query_runs r ON r.query_id=q.id AND r.source=$1 WHERE q.enabled ORDER BY r.last_run_at NULLS FIRST,q.keyword LIMIT 20").bind(source.key).fetch_all(pool).await?
    };
    let terms: Vec<Option<&SearchQuery>> = if board {
        vec![None]
    } else {
        batch.iter().map(Some).collect()
    };
    let mut queries = all.clone();
    for q in &batch {
        if !queries.iter().any(|p| p.id == q.id) {
            queries.push(q.clone());
        }
    }
    let classifier = Classifier::new(&queries);
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let mut stats = Vec::new();
    for q in terms {
        let term = q.map(|q| keyword(&q.keyword)).unwrap_or_default();
        let mut count = 0;
        let pages = if source.key.starts_with("smartrecruiters-") {
            20
        } else if board && source.key != "techinasia" {
            1
        } else if source.key == "techinasia" {
            10
        } else {
            2
        };
        for page in 0..pages {
            let (records, more) = fetch(&http, source, &term, page).await?;
            for raw in records {
                let occupation = classifier.occupation(&raw.title, q.map(|q| q.id));
                if let Some(occupation) = occupation
                    && let Some(job) = normalized(raw, source, occupation)
                    && seen.insert(job.source_id.clone())
                {
                    result.push(job);
                    count += 1;
                }
            }
            if result.len() > 5000 {
                return Err(AppError::Invalid("Job source exceeds record limit"));
            }
            if !more {
                break;
            }
            if page + 1 == pages && !source.board.is_empty() {
                return Err(AppError::Invalid("Incomplete company board snapshot"));
            }
        }
        if let Some(q) = q {
            stats.push((q.id, count));
        }
    }
    // A previously enriched LinkedIn job must never be downgraded by a card-only rediscovery.
    if source.key == "linkedin" {
        for job in &mut result {
            let cached:Option<(String,String)>=sqlx::query_as("SELECT description,employment_type FROM job_vacancies WHERE source='linkedin' AND source_id=$1 AND detail_complete").bind(&job.source_id).fetch_optional(pool).await?;
            if let Some((description, employment)) = cached {
                job.description = description;
                job.employment_type = employment;
            }
        }
    }
    Ok((result, stats))
}

/// Persistent retry queue: enrichment does not depend on a job appearing in search again.
pub async fn enrich_linkedin(pool: &PgPool) -> Result<(), AppError> {
    let http = Http::new(Duration::from_secs(5))?;
    tokio::time::sleep(Duration::from_secs(30)).await;
    let index = crate::domain::aliases(&crate::infrastructure::db::skills(pool).await?);
    for _ in 0..60 {
        let row:Option<(Uuid,String,String)>=sqlx::query_as("UPDATE job_vacancies SET detail_attempts=detail_attempts+1,detail_next_attempt_at=now()+interval '12 hours' WHERE id=(SELECT id FROM job_vacancies WHERE source='linkedin' AND status='active' AND NOT detail_complete AND detail_attempts<10 AND detail_next_attempt_at<=now() AND last_seen_at>now()-interval '7 days' ORDER BY detail_attempts,first_seen_at FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING id,source_id,title").fetch_optional(pool).await?;
        let Some((id, source_id, title)) = row else {
            break;
        };
        if source_id.is_empty() || !source_id.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let (status, body) = http
            .request(
                &format!("https://www.linkedin.com/jobs-guest/jobs/api/jobPosting/{source_id}"),
                &[],
                None,
            )
            .await?;
        if matches!(status, 404 | 410) {
            sqlx::query("UPDATE job_vacancies SET status='closed',updated_at=now() WHERE id=$1")
                .bind(id)
                .execute(pool)
                .await?;
            continue;
        }
        if status != 200 {
            tracing::warn!(
                source = "linkedin",
                status,
                "LinkedIn detail requests stopped; backlog retained"
            );
            break;
        }
        let Some((description, employment)) = linkedin_description(&body) else {
            tracing::warn!(
                source = "linkedin",
                "LinkedIn detail markup changed; backlog retained"
            );
            break;
        };
        let mut tx = pool.begin().await?;
        sqlx::query("UPDATE job_vacancies SET description=$2,employment_type=CASE WHEN employment_type='' THEN $3 ELSE employment_type END,detail_complete=true,updated_at=now() WHERE id=$1").bind(id).bind(&description).bind(employment).execute(&mut *tx).await?;
        jobs::tag_vacancy(&mut tx, id, &format!("{title}\n{description}"), &index).await?;
        tx.commit().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_cards_and_real_descriptions() {
        let cards = linkedin_cards(
            r#"<li data-entity-urn="urn:li:jobPosting:123"><h3 class="base-search-card__title">IT Support</h3><h4 class="base-search-card__subtitle">Example</h4><span class="job-search-card__location">Jakarta, Indonesia</span><time datetime="2026-11-10"></time></li>"#,
        );
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].id, "123");
        assert_eq!(cards[0].company, "Example");
        let (text,_)=linkedin_description(r#"<div class="show-more-less-html__markup"><p>Python &amp; SQL</p><script>evil()</script></div>"#).unwrap();
        assert!(text.contains("Python & SQL"));
        assert!(!text.contains("evil"));
        assert_eq!(keyword("data analyst jobs in Jakarta"), "data analyst");
    }
    #[test]
    fn stable_python_ids_and_no_foreign_or_misclassified_jobs() {
        let source = jobs::SOURCES
            .iter()
            .find(|s| s.key == "linkedin")
            .copied()
            .unwrap();
        let q = SearchQuery {
            id: Uuid::new_v4(),
            occupation_id: Uuid::new_v4(),
            keyword: "it support".into(),
            title_pattern: Some("(?i)it support".into()),
        };
        let raw = || RawJob {
            id: "123".into(),
            title: "IT Support".into(),
            company: "Example".into(),
            location: "Jakarta, Indonesia".into(),
            url: "https://www.linkedin.com/jobs/view/123".into(),
            ..Default::default()
        };
        assert!(
            normalize(raw(), source, Some(&q), &[])
                .unwrap()
                .description
                .contains("Deskripsi lengkap:")
        );
        let mut foreign = raw();
        foreign.location = "Singapore".into();
        assert!(normalize(foreign, source, Some(&q), &[]).is_none());
        let mut unrelated = raw();
        unrelated.title = "Mechanical Technician".into();
        assert!(normalize(unrelated, source, Some(&q), &[]).is_none());
    }

    #[test]
    fn source_adapters_map_reference_payloads_and_registry_has_no_duplicate_boards() {
        let fixtures = [
            (
                "kalibrr",
                json!({"id":1,"name":"Data Analyst","company_name":"Example","company":{"code":"example"},"slug":"analyst","google_location":{"address_components":{"city":"Jakarta","country":"Indonesia"}},"description":"SQL Python","activation_date":"2026-11-10"}),
            ),
            (
                "techinasia",
                json!({"id":"1","title":"Data Analyst","company":{"name":"Example"},"city":{"name":"Jakarta","country":{"name":"Indonesia"}},"job_skills":[{"skill":{"name":"SQL"}},{"skill":{"name":"Python"}}]}),
            ),
            (
                "jobstreet",
                json!({"id":1,"title":"Data Analyst","companyName":"Example","locations":[{"label":"Jakarta, Indonesia"}],"teaser":"SQL Python"}),
            ),
            (
                "glints",
                json!({"id":"1","title":"Data Analyst","company":{"name":"Example"},"location":{"formattedName":"Jakarta"},"country":{"name":"Indonesia"},"skills":[{"skill":{"name":"SQL"}},{"skill":{"name":"Python"}}]}),
            ),
            (
                "dealls",
                json!({"id":"1","role":"Data Analyst","slug":"analyst","company":{"name":"Example","slug":"example"},"city":{"name":"Jakarta"},"country":{"name":"Indonesia"},"skills":[{"name":"SQL"},{"name":"Python"}]}),
            ),
            (
                "kitalulus",
                json!({"id":"1","positionName":"Data Analyst","slug":"analyst","company":{"name":"Example"},"city":{"name":"Jakarta"},"province":{"name":"DKI Jakarta"}}),
            ),
            (
                "karir",
                json!({"id":1,"job_position":"Data Analyst","company_name":"Example","description":"Jakarta • Purna Waktu • On-Site"}),
            ),
            (
                "greenhouse-xendit",
                json!({"id":1,"title":"Data Analyst","company_name":"Example","absolute_url":"https://boards.greenhouse.io/xendit/jobs/1","location":{"name":"Jakarta, Indonesia"},"content":"SQL Python"}),
            ),
            (
                "lever-ninjavan",
                json!({"id":"1","text":"Data Analyst","hostedUrl":"https://jobs.lever.co/ninjavan/1","categories":{"location":"Jakarta, Indonesia"},"descriptionPlain":"SQL Python"}),
            ),
            (
                "workable-amartha",
                json!({"shortcode":"1","title":"Data Analyst","url":"https://apply.workable.com/amartha/j/1","city":"Jakarta","country":"Indonesia","description":"SQL Python"}),
            ),
            (
                "ashby-notion",
                json!({"id":"1","title":"Data Analyst","jobUrl":"https://jobs.ashbyhq.com/notion/1","location":"Jakarta, Indonesia","descriptionPlain":"SQL Python"}),
            ),
            (
                "smartrecruiters-julo",
                json!({"id":"1","name":"Data Analyst","location":{"city":"Jakarta","country":"ID"},"jobAd":{"sections":{"jobDescription":{"text":"SQL Python"}}}}),
            ),
            (
                "remotive",
                json!({"id":1,"title":"Data Analyst","company_name":"Example","url":"https://remotive.com/jobs/1","candidate_required_location":"Indonesia","tags":["SQL","Python"]}),
            ),
            (
                "remoteok",
                json!({"id":"1","position":"Data Analyst","company":"Example","url":"https://remoteok.com/jobs/1","location":"Indonesia","tags":["SQL","Python"]}),
            ),
            (
                "arbeitnow",
                json!({"slug":"1","title":"Data Analyst","company_name":"Example","url":"https://www.arbeitnow.com/jobs/1","location":"Jakarta","tags":["SQL","Python"]}),
            ),
            (
                "hackernews",
                json!({"id":1,"text":"Example | Data Analyst | Jakarta, Indonesia<br>SQL Python"}),
            ),
            (
                "adzuna",
                json!({"id":"1","title":"Data Analyst","company":{"display_name":"Example"},"redirect_url":"https://www.adzuna.com/jobs/1","location":{"display_name":"Jakarta, Indonesia"},"description":"SQL Python"}),
            ),
        ];
        for (key, payload) in fixtures {
            let source = jobs::SOURCES
                .iter()
                .find(|s| s.key == key)
                .copied()
                .unwrap();
            let raw = map_record(source, &payload, &json!({"name":"Example"}));
            assert!(!raw.id.is_empty(), "{key}");
            assert_eq!(raw.title, "Data Analyst", "{key}");
            assert!(!raw.company.is_empty(), "{key}");
            assert!(
                allowed_posting(key, &reqwest::Url::parse(&raw.url).unwrap()),
                "{key}"
            );
        }
        let keys: HashSet<&str> = jobs::SOURCES.iter().map(|s| s.key).collect();
        assert_eq!(keys.len(), jobs::SOURCES.len());
        assert!(!keys.contains("smartrecruiters-grab"));
    }

    #[tokio::test]
    #[ignore = "requires disposable migrated TEST_DATABASE_URL"]
    async fn linkedin_repeat_search_keeps_real_description_and_unseen_jobs() {
        let url = std::env::var("TEST_DATABASE_URL").unwrap();
        let owner = PgPool::connect(&url).await.unwrap();
        let database: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&owner)
            .await
            .unwrap();
        assert!(database.contains("_test"));
        let pool = crate::infrastructure::db::connect_runtime_pool(
            &url,
            crate::infrastructure::db::RuntimeRole::Importer,
        )
        .await
        .unwrap();
        let source = jobs::SOURCES
            .iter()
            .find(|s| s.key == "linkedin")
            .copied()
            .unwrap();
        let q = jobs::next_queries(&pool, 1).await.unwrap().remove(0);
        let sid = (Uuid::new_v4().as_u128() % 10_000_000_000).to_string();
        let job = Vacancy {
            source: "linkedin".into(),
            source_id: sid.clone(),
            title: "IT Support".into(),
            company: "Native fixture".into(),
            country: "ID".into(),
            city: "Jakarta".into(),
            region: "DKI Jakarta".into(),
            workplace: String::new(),
            employment_type: String::new(),
            description: "Real employer description requiring SQL and Python".into(),
            requirements: String::new(),
            apply_url: format!("https://www.linkedin.com/jobs/view/{sid}"),
            source_url: format!("https://www.linkedin.com/jobs/view/{sid}"),
            posted_at: Some(Utc::now()),
            valid_until: None,
            occupation_id: Some(q.occupation_id),
        };
        let run = Uuid::new_v4();
        sqlx::query("UPDATE job_sources SET run_id=$1,lease_until=now()+interval '1 hour' WHERE key='linkedin'").bind(run).execute(&owner).await.unwrap();
        jobs::persist(
            &pool,
            source,
            run,
            std::slice::from_ref(&job),
            &[],
            Duration::from_secs(43200),
        )
        .await
        .unwrap();
        sqlx::query("UPDATE job_sources SET run_id=$1,lease_until=now()+interval '1 hour' WHERE key='linkedin'").bind(run).execute(&owner).await.unwrap();
        let mut placeholder = job.clone();
        placeholder.description =
            "IT Support\nDeskripsi lengkap: https://www.linkedin.com/jobs/view/123".into();
        jobs::persist(
            &pool,
            source,
            run,
            &[placeholder],
            &[(q.id, 1)],
            Duration::from_secs(43200),
        )
        .await
        .unwrap();
        let stored:(String,bool,i64)=sqlx::query_as("SELECT description,detail_complete,(SELECT count(*) FROM job_vacancies WHERE source='linkedin' AND source_id=$1) FROM job_vacancies WHERE source='linkedin' AND source_id=$1").bind(&sid).fetch_one(&pool).await.unwrap();
        assert_eq!(stored.0, job.description);
        assert!(stored.1);
        assert_eq!(stored.2, 1);
        sqlx::query("UPDATE job_sources SET run_id=$1,lease_until=now()+interval '1 hour' WHERE key='linkedin'").bind(run).execute(&owner).await.unwrap();
        jobs::persist(&pool, source, run, &[], &[], Duration::from_secs(43200))
            .await
            .unwrap();
        let active: bool = sqlx::query_scalar(
            "SELECT status='active' FROM job_vacancies WHERE source='linkedin' AND source_id=$1",
        )
        .bind(&sid)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(active);
        sqlx::query("DELETE FROM job_vacancies WHERE source='linkedin' AND source_id=$1")
            .bind(&sid)
            .execute(&owner)
            .await
            .unwrap();
    }
}
