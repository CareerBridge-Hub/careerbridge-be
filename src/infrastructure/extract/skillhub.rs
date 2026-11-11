//! Skillhub institutions, registered programs, published courses and realization history.
//! Stable IDs and changed-row-only writes match the Python production loader.
use super::{geo, http::Http};
use crate::{
    domain::{aliases, skills_in_text},
    infrastructure::{db, error::AppError},
};
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use uuid::Uuid;

const ROOT: &str = "https://api.kemnaker.go.id";
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Institution {
    pub id: Uuid,
    pub nama: String,
    pub jenis: String,
    pub province_id: Option<i16>,
    pub kota: String,
    pub website: String,
    pub source_url: String,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Training {
    pub id: Uuid,
    pub institution_id: Uuid,
    pub source_kind: String,
    pub source_id: String,
    pub nama: String,
    pub biaya: Option<i64>,
    pub moda: Option<String>,
    pub url_daftar: String,
    pub tag_text: String,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct History {
    pub source_id: Uuid,
    pub institution_id: Uuid,
    pub payload: Value,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Detail {
    pub institution_id: Uuid,
    pub payload: Value,
}
#[derive(Default, Serialize, Deserialize, Debug)]
pub struct Snapshot {
    pub institutions: Vec<Institution>,
    pub trainings: Vec<Training>,
    pub history: Vec<History>,
    pub details: Vec<Detail>,
}
#[derive(Default, Serialize, Debug)]
pub struct Report {
    pub institutions_in_snapshot: usize,
    pub trainings_in_snapshot: usize,
    pub institutions: u64,
    pub trainings: u64,
    pub deactivated: u64,
    pub tags_added: u64,
    pub tags_removed: u64,
    pub history: u64,
    pub details: u64,
    pub revision_bumps: i64,
}
fn text(v: &Value) -> String {
    v.as_str().unwrap_or("").trim().to_owned()
}
fn uuid(v: &Value) -> Result<Uuid, AppError> {
    Uuid::parse_str(v.as_str().ok_or(AppError::Provider)?).map_err(|_| AppError::Provider)
}
pub fn training_id(kind: &str, source: &str, institution: Uuid) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("skillhub:{kind}:{source}:{institution}").as_bytes(),
    )
}
fn names(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().or_else(|| v["name"].as_str()))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
pub fn modality(v: &Value) -> Option<String> {
    let modes: HashSet<String> = names(v).into_iter().map(|s| s.to_lowercase()).collect();
    if modes.len() != 1 {
        return None;
    }
    if modes.contains("offline") {
        Some("luring".into())
    } else if modes.contains("online") {
        Some("daring".into())
    } else {
        None
    }
}
pub fn website(raw: &str, fallback: &str) -> String {
    let mut value = raw.split(',').next().unwrap_or("").trim().to_owned();
    while value.to_lowercase().starts_with("http://http")
        || value.to_lowercase().starts_with("https://http")
    {
        value = value.split_once("://").unwrap().1.into();
    }
    if let Some(v) = value.strip_prefix("http:")
        && !v.starts_with("//")
    {
        value = format!("http://{}", v.trim_start_matches('/'));
    }
    if let Some(v) = value.strip_prefix("https:")
        && !v.starts_with("//")
    {
        value = format!("https://{}", v.trim_start_matches('/'));
    }
    if !value.to_lowercase().starts_with("http://") && !value.to_lowercase().starts_with("https://")
    {
        value = format!("https://{value}");
    }
    reqwest::Url::parse(&value)
        .ok()
        .filter(|u| {
            matches!(u.scheme(), "http" | "https")
                && u.username().is_empty()
                && u.password().is_none()
                && u.domain().is_some_and(|d| d.contains('.'))
                && !value.chars().any(char::is_whitespace)
        })
        .map(|_| value)
        .unwrap_or_else(|| fallback.into())
}
fn slug(raw: &str) -> String {
    regex::Regex::new("[^a-z0-9]+")
        .unwrap()
        .replace_all(&raw.to_lowercase(), "-")
        .trim_matches('-')
        .into()
}
fn price(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| {
            v.as_f64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
                .filter(|v| v.is_finite() && *v < i64::MAX as f64)
                .map(|v| v as i64)
        })
        .map(|v| v.max(0))
}
fn training(inst: &Institution, kind: &str, v: &Value) -> Result<Training, AppError> {
    let id = text(&v["id"]);
    if id.is_empty() {
        return Err(AppError::Provider);
    }
    let name = if kind == "programs" {
        text(&v["name"])
    } else {
        text(&v["title"])
    };
    let sub = if kind == "programs" {
        text(&v["sub_vocational"]["name"])
    } else {
        text(&v["sub_vocational_name"])
    };
    let tag_text = std::iter::once(name.clone())
        .chain(std::iter::once(sub))
        .chain(names(&v["skills"]))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Training {
        id: training_id(kind, &id, inst.id),
        institution_id: inst.id,
        source_kind: kind.into(),
        source_id: id,
        nama: name,
        biaya: if kind == "programs" {
            None
        } else {
            price(&v["price"])
        },
        moda: modality(if kind == "vokasi_nasional" {
            &v["media"]
        } else {
            &v["medias"]
        }),
        url_daftar: inst.source_url.clone(),
        tag_text,
    })
}
fn institution(listing: &Value, detail: &Value) -> Result<Institution, AppError> {
    let p = &detail["public"];
    let id = uuid(&listing["institutionable_id"])?;
    let name = if text(&p["name"]).is_empty() {
        text(&listing["name"])
    } else {
        text(&p["name"])
    };
    if name.is_empty() {
        return Err(AppError::Invalid("Skillhub institution has no name"));
    }
    let source_url = format!(
        "https://skillhub.kemnaker.go.id/mitra/temukan-mitra/{}-{id}/program?catalogue={}",
        slug(&name),
        text(&listing["id"])
    );
    let city = &p["postal_code"]["village"]["sub_district"]["city"];
    Ok(Institution {
        id,
        nama: name,
        jenis: if text(&listing["type"]) == "government" {
            "BLK"
        } else {
            "LPK"
        }
        .into(),
        province_id: geo::province_id(&text(&city["province"]["name"])),
        kota: if text(&city["name"]).is_empty() {
            text(&listing["city_name"])
        } else {
            text(&city["name"])
        },
        website: website(&text(&p["website"]), &source_url),
        source_url,
    })
}
fn meta(v: &Value, key: &str) -> usize {
    v["meta"]["pagination"][key]
        .as_u64()
        .or_else(|| {
            v["meta"]["pagination"][key]
                .as_str()
                .and_then(|s| s.parse().ok())
        })
        .or_else(|| v["meta"][key].as_u64())
        .or_else(|| v["meta"][key].as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(if key == "last_page" { 1 } else { 0 }) as usize
}
async fn page_list(
    http: &Http,
    path: &str,
    params: &[(&str, String)],
    size: usize,
    stable: bool,
) -> Result<(HashMap<String, Value>, usize), AppError> {
    let mut seen = HashMap::new();
    let mut total = 0;
    for page in 1..=300 {
        let mut q = params.to_vec();
        q.push(("page", page.to_string()));
        q.push(("limit", size.to_string()));
        if stable {
            q.push(("order_by", "title".into()));
        }
        let data = http.json(&format!("{ROOT}/{path}"), &q, None).await?;
        let items = data["data"].as_array().ok_or(AppError::Provider)?;
        total = total.max(meta(&data, "total"));
        for item in items {
            let id = text(&item["id"]);
            if id.is_empty() {
                return Err(AppError::Provider);
            }
            seen.insert(id, item.clone());
        }
        if seen.len() > 30_000 || total > 30_000 {
            return Err(AppError::Invalid("Skillhub catalogue exceeds limit"));
        }
        if items.is_empty() || page >= meta(&data, "last_page") {
            return Ok((seen, total));
        }
    }
    Err(AppError::Invalid("Skillhub pagination exceeds limit"))
}
async fn complete_list(
    http: &Http,
    path: &str,
    params: &[(&str, String)],
    stable: bool,
) -> Result<Vec<Value>, AppError> {
    let (mut rows, total) = page_list(http, path, params, 100, stable).await?;
    for size in [50, 20, 75, 30, 12] {
        if rows.len() >= total {
            break;
        }
        let (more, _) = page_list(http, path, params, size, stable).await?;
        rows.extend(more);
    }
    if rows.len() < total {
        return Err(AppError::Invalid(
            "Incomplete Skillhub catalogue; previous data preserved",
        ));
    }
    Ok(rows.into_values().collect())
}
async fn detail(http: &Http, listing: Value) -> Result<(Value, Value), AppError> {
    let id = uuid(&listing["institutionable_id"])?;
    let base = format!("lemlat/v1/institutions/{id}");
    let mut detail = json!({});
    for (key, path) in [("public", "public"), ("employees", "summary-employees")] {
        let body = http
            .json(&format!("{ROOT}/{base}/{path}"), &[], None)
            .await?;
        detail[key] = body
            .get("data")
            .cloned()
            .filter(|v| v.is_object())
            .ok_or(AppError::Provider)?;
    }
    for (key, path) in [
        ("programs", "programs"),
        ("trainings", "trainings"),
        ("buildings", "buildings"),
        ("equipments", "equipments"),
        ("accredited_programs", "public-accredited-programs"),
    ] {
        detail[key] = json!(
            complete_list(
                http,
                &format!("{base}/{path}"),
                &[("keyword", String::new())],
                false
            )
            .await?
        );
    }
    Ok((listing, detail))
}
pub async fn collect(pool: &PgPool) -> Result<Snapshot, AppError> {
    let scope = std::env::var("LPK_SCOPE").unwrap_or_else(|_| "published".into());
    if !matches!(scope.as_str(), "published" | "all") {
        return Err(AppError::Invalid("LPK_SCOPE must be published or all"));
    }
    let http = Http::new(Duration::from_millis(150))?;
    let nasional = complete_list(
        &http,
        "skillhub/catalogue/v1/national-batch-trainings",
        &[],
        true,
    )
    .await?;
    let reguler = complete_list(
        &http,
        "skillhub/catalogue/v1/training-catalogues",
        &[],
        true,
    )
    .await?;
    let mut wanted: HashSet<Uuid> =
        sqlx::query_scalar("SELECT id FROM institutions WHERE source='skillhub'")
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();
    for v in &nasional {
        wanted.insert(uuid(&v["institution_id"])?);
    }
    for v in &reguler {
        for id in v["institution_ids"].as_array().ok_or(AppError::Provider)? {
            wanted.insert(uuid(id)?);
        }
    }
    let mut listings = complete_list(
        &http,
        "skillhub/catalogue/v1/institution-catalogues",
        &[("conditions[]", "operational".into())],
        false,
    )
    .await?;
    if scope == "published" {
        listings.retain(|v| uuid(&v["institutionable_id"]).is_ok_and(|id| wanted.contains(&id)));
    }
    let mut all = stream::iter(listings)
        .map(|l| detail(&http, l))
        .buffer_unordered(4);
    let mut snapshot = Snapshot::default();
    let mut insts = HashMap::new();
    let mut count = 0usize;
    while let Some(item) = all.next().await {
        let (listing, raw) = item?;
        count += raw.to_string().len();
        if count > 128 * 1024 * 1024 {
            return Err(AppError::Invalid("Skillhub snapshot exceeds memory budget"));
        }
        let inst = institution(&listing, &raw)?;
        if insts.contains_key(&inst.id) {
            continue;
        }
        for v in raw["programs"].as_array().ok_or(AppError::Provider)? {
            snapshot.trainings.push(training(&inst, "programs", v)?);
        }
        for v in raw["trainings"].as_array().ok_or(AppError::Provider)? {
            snapshot.history.push(History {
                source_id: uuid(&v["id"])?,
                institution_id: inst.id,
                payload: v.clone(),
            });
        }
        snapshot.details.push(Detail {
            institution_id: inst.id,
            payload: json!({"listing":listing,"detail":raw}),
        });
        insts.insert(inst.id, inst);
    }
    for v in &nasional {
        if let Some(inst) = insts.get(&uuid(&v["institution_id"])?) {
            snapshot
                .trainings
                .push(training(inst, "vokasi_nasional", v)?);
        }
    }
    for v in &reguler {
        for id in v["institution_ids"].as_array().ok_or(AppError::Provider)? {
            if let Some(inst) = insts.get(&uuid(id)?) {
                snapshot
                    .trainings
                    .push(training(inst, "vokasi_reguler", v)?);
            }
        }
    }
    snapshot.institutions = insts.into_values().collect();
    let mut seen = HashSet::new();
    snapshot
        .trainings
        .retain(|t| !t.nama.is_empty() && seen.insert(t.id));
    Ok(snapshot)
}

/// Three small read-only requests for verifying current API availability/schema.
pub async fn probe() -> Result<Value, AppError> {
    let http = Http::new(Duration::from_millis(150))?;
    let mut result = json!({"databaseWrites":0});
    for path in [
        "institution-catalogues",
        "national-batch-trainings",
        "training-catalogues",
    ] {
        let data = http
            .json(
                &format!("{ROOT}/skillhub/catalogue/v1/{path}"),
                &[("limit", "1".into()), ("order_by", "title".into())],
                None,
            )
            .await?;
        if !data["data"].is_array() {
            return Err(AppError::Invalid("Skillhub catalogue schema changed"));
        }
        result[path] = json!({"reportedTotal":meta(&data,"total"),"sampleRows":data["data"].as_array().unwrap().len()});
    }
    Ok(result)
}

/// One atomic write. Statements which fire catalogue triggers are skipped entirely
/// when their set is unchanged, preserving embeddings and analysis cache keys.
pub async fn persist(
    pool: &PgPool,
    snapshot: &Snapshot,
    dry_run: bool,
) -> Result<Report, AppError> {
    if snapshot.institutions.is_empty() {
        return Err(AppError::Invalid(
            "Empty Skillhub snapshot; refusing deactivation",
        ));
    }
    let index = aliases(&db::skills(pool).await?);
    let tags: Vec<Value> = snapshot
        .trainings
        .iter()
        .flat_map(|t| {
            skills_in_text(&t.tag_text, &index)
                .into_iter()
                .map(move |s| json!({"training_id":t.id,"skill_id":s}))
        })
        .collect();
    let mut tx = pool.begin().await?;
    // Also serializes manual dry-runs with scheduled writes, on every replica.
    sqlx::query("SELECT pg_advisory_xact_lock(112842,1)")
        .execute(&mut *tx)
        .await?;
    let before: i64 = sqlx::query_scalar("SELECT revision FROM catalog_revision")
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query("CREATE TEMP TABLE stage_extract_institution ON COMMIT DROP AS SELECT * FROM jsonb_to_recordset($1) AS x(id uuid,nama text,jenis text,province_id smallint,kota text,website text,source_url text)").bind(json!(snapshot.institutions)).execute(&mut *tx).await?;
    sqlx::query("CREATE TEMP TABLE stage_extract_training ON COMMIT DROP AS SELECT * FROM jsonb_to_recordset($1) AS x(id uuid,institution_id uuid,source_kind text,source_id text,nama text,biaya bigint,moda text,url_daftar text)").bind(json!(snapshot.trainings)).execute(&mut *tx).await?;
    sqlx::query("CREATE TEMP TABLE stage_extract_tags ON COMMIT DROP AS SELECT DISTINCT * FROM jsonb_to_recordset($1) AS x(training_id uuid,skill_id uuid)").bind(json!(tags)).execute(&mut *tx).await?;
    let active: i64 =
        sqlx::query_scalar("SELECT count(*) FROM trainings WHERE source='skillhub' AND aktif")
            .fetch_one(&mut *tx)
            .await?;
    let removed:i64=sqlx::query_scalar("SELECT count(*) FROM trainings t WHERE t.source='skillhub' AND t.aktif AND t.institution_id IN(SELECT id FROM stage_extract_institution) AND NOT EXISTS(SELECT 1 FROM stage_extract_training s WHERE s.id=t.id)").fetch_one(&mut *tx).await?;
    if removed > 0 && removed * 2 > active {
        return Err(AppError::Invalid(
            "Skillhub count dropped by more than half; previous catalogue preserved",
        ));
    }
    let mut report = Report {
        institutions_in_snapshot: snapshot.institutions.len(),
        trainings_in_snapshot: snapshot.trainings.len(),
        ..Default::default()
    };
    let changes:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM stage_extract_institution s LEFT JOIN institutions i ON i.id=s.id WHERE i.id IS NULL OR (i.nama,i.jenis,i.province_id,i.kota,i.website,i.source_url) IS DISTINCT FROM (s.nama,s.jenis,s.province_id,s.kota,s.website,s.source_url))").fetch_one(&mut *tx).await?;
    if changes {
        report.institutions=sqlx::query("INSERT INTO institutions(id,nama,jenis,province_id,kota,website,source_url,source,source_id,imported_at) SELECT id,nama,jenis,province_id,kota,website,source_url,'skillhub',id::text,now() FROM stage_extract_institution ON CONFLICT(id) DO UPDATE SET nama=EXCLUDED.nama,jenis=EXCLUDED.jenis,province_id=EXCLUDED.province_id,kota=EXCLUDED.kota,website=EXCLUDED.website,source_url=EXCLUDED.source_url,imported_at=now() WHERE (institutions.nama,institutions.jenis,institutions.province_id,institutions.kota,institutions.website,institutions.source_url) IS DISTINCT FROM (EXCLUDED.nama,EXCLUDED.jenis,EXCLUDED.province_id,EXCLUDED.kota,EXCLUDED.website,EXCLUDED.source_url)").execute(&mut *tx).await?.rows_affected();
    }
    let changes:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM stage_extract_training s LEFT JOIN trainings t ON t.source='skillhub' AND t.source_kind=s.source_kind AND t.source_id=s.source_id AND t.institution_id=s.institution_id WHERE t.id IS NULL OR (t.nama,t.biaya,t.moda,t.url_daftar,t.source_url,t.aktif,t.sample) IS DISTINCT FROM (s.nama,s.biaya,s.moda,s.url_daftar,s.url_daftar,true,false))").fetch_one(&mut *tx).await?;
    if changes {
        report.trainings=sqlx::query("INSERT INTO trainings(id,institution_id,source_kind,source_id,nama,biaya,moda,url_daftar,source_url,durasi,aktif,sample,source,imported_at) SELECT id,institution_id,source_kind,source_id,nama,biaya,moda,url_daftar,url_daftar,'',true,false,'skillhub',now() FROM stage_extract_training ON CONFLICT(source,source_kind,source_id,institution_id) DO UPDATE SET nama=EXCLUDED.nama,biaya=EXCLUDED.biaya,moda=EXCLUDED.moda,url_daftar=EXCLUDED.url_daftar,source_url=EXCLUDED.source_url,aktif=true,sample=false,imported_at=now() WHERE (trainings.nama,trainings.biaya,trainings.moda,trainings.url_daftar,trainings.source_url,trainings.aktif,trainings.sample) IS DISTINCT FROM (EXCLUDED.nama,EXCLUDED.biaya,EXCLUDED.moda,EXCLUDED.url_daftar,EXCLUDED.source_url,true,false)").execute(&mut *tx).await?.rows_affected();
    }
    if removed > 0 {
        report.deactivated=sqlx::query("UPDATE trainings t SET aktif=false,imported_at=now() WHERE t.source='skillhub' AND t.aktif AND t.institution_id IN(SELECT id FROM stage_extract_institution) AND NOT EXISTS(SELECT 1 FROM stage_extract_training s WHERE s.id=t.id)").execute(&mut *tx).await?.rows_affected();
    }
    let remove:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM training_skills ts JOIN stage_extract_training s ON s.id=ts.training_id WHERE NOT EXISTS(SELECT 1 FROM stage_extract_tags x WHERE x.training_id=ts.training_id AND x.skill_id=ts.skill_id))").fetch_one(&mut *tx).await?;
    if remove {
        report.tags_removed=sqlx::query("DELETE FROM training_skills ts USING stage_extract_training s WHERE s.id=ts.training_id AND NOT EXISTS(SELECT 1 FROM stage_extract_tags x WHERE x.training_id=ts.training_id AND x.skill_id=ts.skill_id)").execute(&mut *tx).await?.rows_affected();
    }
    let add:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM stage_extract_tags x WHERE NOT EXISTS(SELECT 1 FROM training_skills ts WHERE ts.training_id=x.training_id AND ts.skill_id=x.skill_id))").fetch_one(&mut *tx).await?;
    if add {
        report.tags_added=sqlx::query("INSERT INTO training_skills(training_id,skill_id) SELECT training_id,skill_id FROM stage_extract_tags ON CONFLICT DO NOTHING").execute(&mut *tx).await?.rows_affected();
    }
    report.history=sqlx::query("INSERT INTO skillhub_training_history(source_id,institution_id,payload) SELECT source_id,institution_id,payload FROM jsonb_to_recordset($1) AS x(source_id uuid,institution_id uuid,payload jsonb) ON CONFLICT(source_id) DO UPDATE SET institution_id=EXCLUDED.institution_id,payload=EXCLUDED.payload,imported_at=now() WHERE (skillhub_training_history.institution_id,skillhub_training_history.payload) IS DISTINCT FROM (EXCLUDED.institution_id,EXCLUDED.payload)").bind(json!(snapshot.history)).execute(&mut *tx).await?.rows_affected();
    report.details=sqlx::query("INSERT INTO skillhub_institution_details(institution_id,payload) SELECT institution_id,payload FROM jsonb_to_recordset($1) AS x(institution_id uuid,payload jsonb) ON CONFLICT(institution_id) DO UPDATE SET payload=EXCLUDED.payload,imported_at=now() WHERE skillhub_institution_details.payload IS DISTINCT FROM EXCLUDED.payload").bind(json!(snapshot.details)).execute(&mut *tx).await?.rows_affected();
    let after: i64 = sqlx::query_scalar("SELECT revision FROM catalog_revision")
        .fetch_one(&mut *tx)
        .await?;
    report.revision_bumps = after - before;
    if dry_run {
        tx.rollback().await?;
    } else {
        tx.commit().await?;
    }
    Ok(report)
}

pub async fn once(pool: &PgPool, force: bool, dry_run: bool) -> Result<Option<Report>, AppError> {
    let run = Uuid::new_v4();
    let claim=sqlx::query("UPDATE job_sources SET run_id=$1,lease_until=now()+interval '2 hours',managed_by=CASE WHEN $3 THEN managed_by ELSE 'rust' END WHERE key='skillhub' AND (lease_until IS NULL OR lease_until<now()) AND ($2 OR next_run_at<=now())").bind(run).bind(force||dry_run).bind(dry_run).execute(pool).await?.rows_affected();
    if claim == 0 {
        return Ok(None);
    }
    tracing::info!(source = "skillhub", "LPK import started");
    let result = tokio::time::timeout(Duration::from_secs(5400), async {
        let snapshot = collect(pool).await?;
        persist(pool, &snapshot, dry_run).await
    })
    .await
    .unwrap_or(Err(AppError::Provider));
    let error = result.as_ref().err().map(ToString::to_string);
    let seconds = if dry_run {
        0.0
    } else if error.is_some() {
        21600.0
    } else {
        604800.0
    };
    let count = result
        .as_ref()
        .ok()
        .map(|r| r.trainings_in_snapshot as i32)
        .unwrap_or(0);
    sqlx::query("UPDATE job_sources SET run_id=NULL,lease_until=NULL,last_error=$2,last_success_at=CASE WHEN $2::text IS NULL AND NOT $5 THEN now() ELSE last_success_at END,last_count=CASE WHEN $2::text IS NULL AND NOT $5 THEN $3 ELSE last_count END,next_run_at=CASE WHEN $5 THEN next_run_at ELSE now()+make_interval(secs=>$4) END WHERE key='skillhub' AND run_id=$1").bind(run).bind(error).bind(count).bind(seconds).bind(dry_run).execute(pool).await?;
    result.map(Some)
}
pub async fn start(url: &str) -> Result<(), AppError> {
    match std::env::var("LPK_SCRAPE_ENABLED").as_deref() {
        Ok("true") => {}
        Ok("false") | Err(_) => return Ok(()),
        _ => {
            return Err(AppError::Invalid(
                "LPK_SCRAPE_ENABLED must be true or false",
            ));
        }
    }
    let pool = db::connect_runtime_pool(url, db::RuntimeRole::CatalogImporter).await?;
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            match once(&pool, false, false).await {
                Ok(Some(report)) => {
                    tracing::info!(source="skillhub",report=?report,"LPK import completed")
                }
                Err(error) => {
                    tracing::warn!(source="skillhub",error=%error,"LPK import failed; previous data preserved")
                }
                _ => {}
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_ids_unknown_facts_and_safe_websites() {
        let iid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        // Golden value from the Python loader's existing HOUSEKEEPER row.
        assert_eq!(
            training_id(
                "vokasi_reguler",
                "7d64aff3-4da3-4014-908a-215eca98b6f3",
                Uuid::parse_str("d384839e-25df-48ec-82b7-766566c9ac40").unwrap()
            )
            .to_string(),
            "09c97f4b-d4cc-568e-83c0-7c0e13a400ed"
        );
        assert_eq!(
            training_id("programs", "123", iid),
            Uuid::new_v5(
                &Uuid::NAMESPACE_URL,
                b"skillhub:programs:123:550e8400-e29b-41d4-a716-446655440000"
            )
        );
        assert_eq!(modality(&json!(["online"])), Some("daring".into()));
        assert_eq!(modality(&json!(["online", "offline"])), None);
        assert_eq!(price(&Value::Null), None);
        assert_eq!(price(&json!(123.5)), Some(123));
        assert_eq!(
            website("http://a.id, http://b.id,", "fallback"),
            "http://a.id"
        );
        assert_eq!(
            website("HTTP://WWW.EXAMPLE.COM", "fallback"),
            "HTTP://WWW.EXAMPLE.COM"
        );
        for bad in [
            "-",
            "0",
            "x@example.com",
            "javascript:alert(1)",
            "Tiktok : example.com",
        ] {
            assert_eq!(
                website(bad, "https://skillhub.kemnaker.go.id/x"),
                "https://skillhub.kemnaker.go.id/x"
            );
        }
        assert_eq!(
            website("http://https://example.com", "fallback"),
            "https://example.com"
        );
    }
}
