//! Catalog administration. Every operation rechecks the profile role inside its transaction.
use super::{db, error::AppError};
use crate::domain::{JobSkill, normalize};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Row, Transaction};
use std::collections::HashSet;
use uuid::Uuid;

type Tx<'a> = Transaction<'a, Postgres>;
const MAX_CATALOG: i64 = 10_000;
pub const MAX_IMPORT: usize = 500;

pub async fn authorized(pool: &PgPool, actor: Uuid) -> Result<Tx<'_>, AppError> {
    let mut tx = db::active_tx(pool, actor).await?;
    let admin: bool = sqlx::query_scalar("SELECT public.is_catalog_admin()")
        .fetch_one(&mut *tx)
        .await?;
    if !admin {
        return Err(AppError::Forbidden);
    }
    Ok(tx)
}
fn db_error(error: sqlx::Error) -> AppError {
    match error.as_database_error().and_then(|e| e.code()).as_deref() {
        Some("23505") => AppError::Invalid("Nama, alias, atau data tersebut sudah digunakan."),
        Some("23503") => AppError::Invalid("Data rujukan tidak ditemukan atau masih digunakan."),
        Some("23514" | "22001" | "22003") => {
            AppError::Invalid("Nilai data melewati batas yang diizinkan.")
        }
        Some("P0001") => AppError::Invalid(
            "Setiap pekerjaan harus memiliki minimal satu skill wajib. Ubah pekerjaan terkait dahulu.",
        ),
        _ => AppError::from(error),
    }
}
fn text(value: &str, max: usize, required: bool) -> bool {
    (!required || !value.trim().is_empty())
        && value.chars().count() <= max
        && !value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
}
fn url(value: &str, required: bool) -> bool {
    if value.is_empty() {
        return !required;
    }
    value.len() <= 2048
        && !value.chars().any(char::is_control)
        && reqwest::Url::parse(value).is_ok_and(|u| {
            matches!(u.scheme(), "http" | "https")
                && u.host_str().is_some()
                && u.username().is_empty()
                && u.password().is_none()
        })
}
fn invalid() -> AppError {
    AppError::Invalid("Data tidak valid. Periksa kolom wajib, panjang teks, dan nilai pilihan.")
}
fn parse<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, AppError> {
    serde_json::from_value(v).map_err(|_| invalid())
}
async fn values(tx: &mut Tx<'_>, sql: &'static str) -> Result<Vec<Value>, AppError> {
    let rows = sqlx::query(sql).fetch_all(&mut **tx).await?;
    Ok(rows.into_iter().map(|r| r.get("data")).collect())
}
pub async fn snapshot(pool: &PgPool, actor: Uuid) -> Result<Value, AppError> {
    let mut tx = authorized(pool, actor).await?;
    // The master UI needs the complete dictionary for CSV cross-reference validation.
    // Bound the admin-only snapshot explicitly; user login never downloads this data.
    let biggest:i64=sqlx::query_scalar("SELECT greatest((SELECT count(*) FROM skills),(SELECT count(*) FROM occupations),(SELECT count(*) FROM institutions),(SELECT count(*) FROM trainings),(SELECT count(*) FROM job_vacancies))")
        .fetch_one(&mut *tx).await?;
    if biggest > MAX_CATALOG {
        return Err(AppError::Invalid(
            "Katalog admin melebihi 10.000 baris. Gunakan API katalog berhalaman.",
        ));
    }
    let skills = values(&mut tx, db::SKILLS_SQL).await?;
    let mut jobs = values(&mut tx, db::OCCUPATIONS_SQL).await?;
    // An admin's own CV-inferred occupation is personal, not part of the catalog being edited.
    jobs.retain(|j| j["custom"] != true);
    let institutions = values(&mut tx, db::INSTITUTIONS_SQL).await?;
    let trainings=values(&mut tx,"SELECT jsonb_build_object('id',t.id,'institutionId',t.institution_id,'nama',t.nama,'biaya',t.biaya,'moda',t.moda,'durasi',t.durasi,'urlDaftar',t.url_daftar,'sample',t.sample,'skillIds',coalesce((SELECT jsonb_agg(skill_id ORDER BY skill_id) FROM training_skills WHERE training_id=t.id),'[]')) data FROM trainings t ORDER BY t.nama,t.id").await?;
    let postings=values(&mut tx,"SELECT jsonb_build_object('id',id,'jobRoleId',CASE WHEN title ~* '(devops|devsecops|infraops|site reliability|sre)' THEN 'a9619ac0-8e03-4ddd-bc4f-b31bfb190ec5' WHEN title ~* 'data[ -]?analyst' THEN '96f251a2-ee07-5722-9da1-5a2e83e3361d' ELSE 'a9619ac0-8e03-4ddd-bc4f-b31bfb190ec4' END,'judul',title,'perusahaan',company,'lokasi',city,'url',apply_url,'sumber',source,'tanggal',coalesce(posted_at,last_seen_at),'hidden',hidden,'status',status,'validUntil',valid_until,'lastSeenAt',last_seen_at) data FROM job_vacancies ORDER BY posted_at DESC NULLS LAST,id").await?;
    let sources=values(&mut tx,"SELECT jsonb_build_object('key',key,'lastSuccessAt',last_success_at,'nextRunAt',next_run_at,'lastError',last_error,'lastCount',last_count,'running',lease_until>now()) data FROM job_sources ORDER BY key").await?;
    tx.commit().await?;
    Ok(
        json!({"skills":skills,"jobs":jobs,"institutions":institutions,"trainings":trainings,"postings":postings,"sources":sources}),
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SkillInput {
    id: Option<Uuid>,
    nama: String,
    kategori: String,
    #[serde(default)]
    alias: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JobInput {
    id: Option<Uuid>,
    nama: String,
    kode_kbji: String,
    bidang: String,
    deskripsi: String,
    skills: Vec<JobSkill>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InstitutionInput {
    id: Option<Uuid>,
    nama: String,
    jenis: String,
    provinsi: String,
    kota: String,
    website: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TrainingInput {
    id: Option<Uuid>,
    institution_id: Uuid,
    nama: String,
    biaya: Option<i64>,
    moda: Option<String>,
    durasi: String,
    url_daftar: String,
    skill_ids: Vec<Uuid>,
    sample: Option<bool>,
}

async fn skill(tx: &mut Tx<'_>, v: Value) -> Result<Uuid, AppError> {
    let mut x: SkillInput = parse(v)?;
    if !text(&x.nama, 100, true)
        || !text(&x.kategori, 100, true)
        || x.alias.len() > 50
        || x.alias.iter().any(|a| !text(a, 100, true))
    {
        return Err(invalid());
    }
    x.nama = x.nama.trim().into();
    x.kategori = x.kategori.trim().into();
    let id = x.id.unwrap_or_else(Uuid::new_v4);
    let canonical = normalize(&x.nama);
    if canonical.is_empty() {
        return Err(invalid());
    }
    let mut keys = HashSet::from([canonical.clone()]);
    let mut aliases = Vec::new();
    for alias in x.alias {
        let alias = alias.trim().to_owned();
        let key = normalize(&alias);
        if key.is_empty() {
            return Err(invalid());
        }
        if key != canonical && keys.insert(key) {
            aliases.push(alias);
        }
    }
    let others=sqlx::query("SELECT id,nama FROM skills WHERE id!=$1 UNION ALL SELECT skill_id,alias FROM skill_aliases WHERE skill_id!=$1")
        .bind(id).fetch_all(&mut **tx).await?;
    if others
        .iter()
        .any(|r| keys.contains(&normalize(r.get::<&str, _>("nama"))))
    {
        return Err(AppError::Invalid(
            "Nama atau alias sudah dipakai skill lain.",
        ));
    }
    sqlx::query("INSERT INTO skills(id,nama,kategori) VALUES($1,$2,$3) ON CONFLICT(id) DO UPDATE SET nama=EXCLUDED.nama,kategori=EXCLUDED.kategori")
        .bind(id).bind(x.nama).bind(x.kategori).execute(&mut **tx).await.map_err(db_error)?;
    sqlx::query("DELETE FROM skill_aliases WHERE skill_id=$1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(db_error)?;
    for alias in aliases {
        sqlx::query("INSERT INTO skill_aliases(alias,skill_id) VALUES($1,$2)")
            .bind(alias)
            .bind(id)
            .execute(&mut **tx)
            .await
            .map_err(db_error)?;
    }
    Ok(id)
}
async fn unique_name(
    tx: &mut Tx<'_>,
    sql: &'static str,
    id: Uuid,
    name: &str,
) -> Result<(), AppError> {
    let key = normalize(name);
    if key.is_empty() {
        return Err(invalid());
    }
    let rows = sqlx::query(sql).bind(id).fetch_all(&mut **tx).await?;
    if rows
        .iter()
        .any(|r| normalize(r.get::<&str, _>("nama")) == key)
    {
        return Err(AppError::Invalid(
            "Nama tersebut sudah digunakan. Pilih data yang ada untuk memperbaruinya.",
        ));
    }
    Ok(())
}
async fn job(tx: &mut Tx<'_>, v: Value) -> Result<Uuid, AppError> {
    let x: JobInput = parse(v)?;
    let mut seen = HashSet::new();
    if !text(&x.nama, 100, true)
        || !text(&x.bidang, 100, true)
        || !text(&x.deskripsi, 5000, false)
        || x.kode_kbji.len() != 4
        || !x.kode_kbji.bytes().all(|c| c.is_ascii_digit())
        || x.skills.len() > 100
        || !x.skills.iter().any(|s| s.tipe == "wajib")
        || x.skills
            .iter()
            .any(|s| !matches!(s.tipe.as_str(), "wajib" | "opsional") || !seen.insert(s.skill_id))
    {
        return Err(invalid());
    }
    let id = x.id.unwrap_or_else(Uuid::new_v4);
    unique_name(
        tx,
        "SELECT nama FROM occupations WHERE id!=$1 AND owner_id IS NULL",
        id,
        &x.nama,
    )
    .await?;
    sqlx::query("INSERT INTO occupations(id,nama,kode_kbji,bidang,deskripsi) VALUES($1,$2,$3,$4,$5) ON CONFLICT(id) DO UPDATE SET nama=EXCLUDED.nama,kode_kbji=EXCLUDED.kode_kbji,bidang=EXCLUDED.bidang,deskripsi=EXCLUDED.deskripsi")
        .bind(id).bind(x.nama.trim()).bind(x.kode_kbji).bind(x.bidang.trim()).bind(x.deskripsi.trim()).execute(&mut **tx).await.map_err(db_error)?;
    sqlx::query("DELETE FROM occupation_skills WHERE occupation_id=$1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(db_error)?;
    for s in x.skills {
        sqlx::query("INSERT INTO occupation_skills(occupation_id,skill_id,tipe) VALUES($1,$2,$3)")
            .bind(id)
            .bind(s.skill_id)
            .bind(s.tipe)
            .execute(&mut **tx)
            .await
            .map_err(db_error)?;
    }
    Ok(id)
}
async fn institution(tx: &mut Tx<'_>, v: Value) -> Result<Uuid, AppError> {
    let x: InstitutionInput = parse(v)?;
    if !text(&x.nama, 200, true)
        || !text(&x.kota, 100, true)
        || !url(&x.website, false)
        || !matches!(x.jenis.as_str(), "LPK" | "BLK" | "Platform")
    {
        return Err(invalid());
    }
    let province: Option<i16> = sqlx::query_scalar("SELECT id FROM provinces WHERE nama=$1")
        .bind(x.provinsi)
        .fetch_optional(&mut **tx)
        .await?;
    let province = province.ok_or(AppError::Invalid("Provinsi tidak dikenal."))?;
    let id = x.id.unwrap_or_else(Uuid::new_v4);
    unique_name(
        tx,
        "SELECT nama FROM institutions WHERE id!=$1",
        id,
        &x.nama,
    )
    .await?;
    sqlx::query("INSERT INTO institutions(id,nama,jenis,province_id,kota,website) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(id) DO UPDATE SET nama=EXCLUDED.nama,jenis=EXCLUDED.jenis,province_id=EXCLUDED.province_id,kota=EXCLUDED.kota,website=EXCLUDED.website")
        .bind(id).bind(x.nama.trim()).bind(x.jenis).bind(province).bind(x.kota.trim()).bind(x.website).execute(&mut **tx).await.map_err(db_error)?;
    Ok(id)
}
async fn training(tx: &mut Tx<'_>, v: Value) -> Result<Uuid, AppError> {
    let x: TrainingInput = parse(v)?;
    let mut seen = HashSet::new();
    if !text(&x.nama, 200, true)
        || !text(&x.durasi, 200, false)
        || !url(&x.url_daftar, true)
        || x.biaya
            .is_some_and(|c| !(0..=1_000_000_000_000).contains(&c))
        || x.moda
            .as_ref()
            .is_some_and(|m| !matches!(m.as_str(), "daring" | "luring"))
        || x.skill_ids.len() > 100
        || x.skill_ids.iter().any(|id| !seen.insert(id))
    {
        return Err(invalid());
    }
    let id = x.id.unwrap_or_else(Uuid::new_v4);
    let names: Vec<String> =
        sqlx::query_scalar("SELECT nama FROM trainings WHERE id!=$1 AND institution_id=$2")
            .bind(id)
            .bind(x.institution_id)
            .fetch_all(&mut **tx)
            .await?;
    if normalize(&x.nama).is_empty() || names.iter().any(|n| normalize(n) == normalize(&x.nama)) {
        return Err(AppError::Invalid(
            "Program dengan nama yang sama sudah ada di lembaga ini.",
        ));
    }
    sqlx::query("INSERT INTO trainings(id,institution_id,nama,biaya,moda,durasi,url_daftar,sample) VALUES($1,$2,$3,$4,$5,$6,$7,coalesce($8,false)) ON CONFLICT(id) DO UPDATE SET institution_id=EXCLUDED.institution_id,nama=EXCLUDED.nama,biaya=EXCLUDED.biaya,moda=EXCLUDED.moda,durasi=EXCLUDED.durasi,url_daftar=EXCLUDED.url_daftar,sample=coalesce($8,trainings.sample)")
        .bind(id).bind(x.institution_id).bind(x.nama.trim()).bind(x.biaya).bind(x.moda).bind(x.durasi.trim()).bind(x.url_daftar).bind(x.sample).execute(&mut **tx).await.map_err(db_error)?;
    sqlx::query("DELETE FROM training_skills WHERE training_id=$1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(db_error)?;
    for s in x.skill_ids {
        sqlx::query("INSERT INTO training_skills(training_id,skill_id) VALUES($1,$2)")
            .bind(id)
            .bind(s)
            .execute(&mut **tx)
            .await
            .map_err(db_error)?;
    }
    Ok(id)
}
async fn audit(
    tx: &mut Tx<'_>,
    actor: Uuid,
    entity: &str,
    action: &str,
    ids: &[Uuid],
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO admin_audit(id,actor_id,action,entity,entity_ids) VALUES($1,$2,$3,$4,$5)",
    )
    .bind(Uuid::new_v4())
    .bind(actor)
    .bind(action)
    .bind(entity)
    .bind(ids)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
pub async fn save(
    pool: &PgPool,
    actor: Uuid,
    kind: &str,
    items: Vec<Value>,
) -> Result<Vec<Uuid>, AppError> {
    let mut tx = authorized(pool, actor).await?;
    if items.is_empty() || items.len() > MAX_IMPORT {
        return Err(AppError::Invalid("Import harus berisi 1–500 baris."));
    }
    // Serialize edits so canonical-name/alias conflict checks cannot race.
    sqlx::query("SELECT pg_advisory_xact_lock(674291)")
        .execute(&mut *tx)
        .await?;
    let mut ids = Vec::new();
    for item in items {
        ids.push(match kind {
            "skills" => skill(&mut tx, item).await?,
            "jobs" => job(&mut tx, item).await?,
            "institutions" => institution(&mut tx, item).await?,
            "trainings" => training(&mut tx, item).await?,
            _ => return Err(AppError::NotFound),
        });
    }
    if ids.iter().copied().collect::<HashSet<_>>().len() != ids.len() {
        return Err(AppError::Invalid("ID duplikat dalam import."));
    }
    audit(&mut tx, actor, kind, "save", &ids).await?;
    tx.commit().await.map_err(db_error)?;
    Ok(ids)
}
pub async fn remove(pool: &PgPool, actor: Uuid, kind: &str, id: Uuid) -> Result<(), AppError> {
    let mut tx = authorized(pool, actor).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(674291)")
        .execute(&mut *tx)
        .await?;
    let sql = match kind {
        "skills" => "DELETE FROM skills WHERE id=$1",
        "jobs" => "DELETE FROM occupations WHERE id=$1",
        "institutions" => "DELETE FROM institutions WHERE id=$1",
        "trainings" => "DELETE FROM trainings WHERE id=$1",
        _ => return Err(AppError::NotFound),
    };
    if sqlx::query(sql)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?
        .rows_affected()
        == 0
    {
        return Err(AppError::NotFound);
    }
    audit(&mut tx, actor, kind, "delete", &[id]).await?;
    tx.commit().await.map_err(db_error)?;
    Ok(())
}
pub async fn visibility(
    pool: &PgPool,
    actor: Uuid,
    id: Uuid,
    hidden: bool,
) -> Result<(), AppError> {
    let mut tx = authorized(pool, actor).await?;
    if sqlx::query("UPDATE job_vacancies SET hidden=$2 WHERE id=$1")
        .bind(id)
        .bind(hidden)
        .execute(&mut *tx)
        .await?
        .rows_affected()
        == 0
    {
        return Err(AppError::NotFound);
    }
    audit(
        &mut tx,
        actor,
        "vacancies",
        if hidden { "hide" } else { "show" },
        &[id],
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unsafe_urls_and_controls_but_allows_description_newlines() {
        assert!(!url("javascript:alert(1)", true));
        assert!(!url("https://user:secret@example.test", true));
        assert!(url("https://example.test/course", true));
        assert!(!text("bad\0value", 100, true));
        assert!(text("Course\nDescription", 100, true));
        assert!(!text("   ", 100, true));
    }
    #[test]
    fn rejects_unknown_fields_and_non_uuid_import_ids() {
        assert!(parse::<SkillInput>(json!({"id":"sk-123","nama":"A","kategori":"B"})).is_err());
        assert!(parse::<SkillInput>(json!({"nama":"A","kategori":"B","role":"admin"})).is_err());
    }
}
