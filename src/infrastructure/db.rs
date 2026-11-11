pub(super) const SKILLS_SQL: &str = "SELECT jsonb_build_object('id',s.id,'nama',s.nama,'kategori',s.kategori,'alias',coalesce((SELECT jsonb_agg(alias ORDER BY alias) FROM skill_aliases WHERE skill_id=s.id),'[]')) data FROM skills s ORDER BY s.nama,s.id";
pub(super) const OCCUPATIONS_SQL: &str = "SELECT jsonb_build_object('id',o.id,'nama',o.nama,'kodeKbji',o.kode_kbji,'bidang',o.bidang,'deskripsi',o.deskripsi,'skills',coalesce((SELECT jsonb_agg(jsonb_build_object('skillId',skill_id,'tipe',tipe) ORDER BY skill_id) FROM occupation_skills WHERE occupation_id=o.id),'[]')) data FROM occupations o ORDER BY o.nama,o.id";
pub(super) const INSTITUTIONS_SQL: &str = "SELECT jsonb_build_object('id',i.id,'nama',i.nama,'jenis',i.jenis,'provinsi',p.nama,'kota',i.kota,'website',i.website) data FROM institutions i JOIN provinces p ON p.id=i.province_id ORDER BY i.nama,i.id";
pub(super) const TRAININGS_SQL: &str = "SELECT jsonb_build_object('id',t.id,'institutionId',t.institution_id,'nama',t.nama,'biaya',t.biaya,'moda',t.moda,'durasi',t.durasi,'urlDaftar',t.url_daftar,'sample',t.sample,'skillIds',coalesce((SELECT jsonb_agg(skill_id ORDER BY skill_id) FROM training_skills WHERE training_id=t.id),'[]')) data FROM trainings t WHERE t.aktif ORDER BY t.nama,t.id";
use super::error::AppError;
use crate::domain::*;
use serde::de::DeserializeOwned;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;
/// Select only hardcoded runtime roles; never interpolate a role from input.
#[derive(Clone, Copy)]
pub enum RuntimeRole {
    Api,
    Worker,
    Importer,
    CatalogImporter,
}
impl RuntimeRole {
    fn name(self) -> &'static str {
        match self {
            Self::Api => "careerbridge_api",
            Self::Worker => "careerbridge_worker",
            Self::Importer => "careerbridge_importer",
            Self::CatalogImporter => "careerbridge_catalog_importer",
        }
    }
    fn setup(self) -> &'static str {
        match self {
            Self::Api => "SET ROLE careerbridge_api",
            Self::Worker => "SET ROLE careerbridge_worker",
            Self::Importer => "SET ROLE careerbridge_importer",
            Self::CatalogImporter => "SET ROLE careerbridge_catalog_importer",
        }
    }
}

/// Requires a session pooler/direct connection. Every physical connection drops
/// to its restricted role before the pool can hand it to application code.
pub async fn connect_runtime_pool(url: &str, role: RuntimeRole) -> Result<PgPool, AppError> {
    let options = super::config::database_options(url).map_err(|_| {
        AppError::Invalid("Invalid SUPABASE_DB_URL or unsupported transaction pooling.")
    })?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(match role { RuntimeRole::Api => 8, RuntimeRole::Worker | RuntimeRole::Importer | RuntimeRole::CatalogImporter => 2 })
        .acquire_timeout(std::time::Duration::from_secs(5))
        .after_connect(move |connection, _| Box::pin(async move {
            sqlx::query(role.setup()).execute(&mut *connection).await?;
            // SET ROLE does not apply ALTER ROLE ... SET login defaults.
            sqlx::query("SET search_path TO public, extensions").execute(&mut *connection).await?;
            let valid: bool = sqlx::query_scalar("SELECT current_user=$1 AND NOT rolsuper AND NOT rolbypassrls FROM pg_roles WHERE rolname=current_user")
                .bind(role.name()).fetch_one(&mut *connection).await?;
            if !valid { return Err(sqlx::Error::Protocol("Restricted runtime role validation failed".into())); }
            Ok(())
        }))
        .connect_with(options)
        .await?;
    validate_runtime_role(&pool, role.name()).await?;
    Ok(pool)
}

pub async fn user_tx(pool: &PgPool, id: Uuid) -> Result<Transaction<'_, Postgres>, AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.user_id',$1,true)")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}
pub async fn active_tx(pool: &PgPool, id: Uuid) -> Result<Transaction<'_, Postgres>, AppError> {
    let mut tx = user_tx(pool, id).await?;
    let active: Option<bool> = sqlx::query_scalar("SELECT aktif FROM users WHERE id=$1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    if active != Some(true) {
        return Err(AppError::Unauthorized);
    }
    Ok(tx)
}
async fn json_rows<T: DeserializeOwned>(
    pool: &PgPool,
    sql: &'static str,
) -> Result<Vec<T>, AppError> {
    sqlx::query(sql)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| serde_json::from_value(row.get("data")).map_err(|_| AppError::Internal))
        .collect()
}
pub async fn skills(pool: &PgPool) -> Result<Vec<Skill>, AppError> {
    json_rows(pool, SKILLS_SQL).await
}
pub async fn occupations(pool: &PgPool) -> Result<Vec<Occupation>, AppError> {
    json_rows(pool, OCCUPATIONS_SQL).await
}
pub async fn institutions(pool: &PgPool) -> Result<Vec<Institution>, AppError> {
    json_rows(pool, INSTITUTIONS_SQL).await
}
pub async fn trainings(pool: &PgPool) -> Result<Vec<Training>, AppError> {
    json_rows(pool, TRAININGS_SQL).await
}
pub async fn bump(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<(), AppError> {
    sqlx::query("UPDATE users SET revision=revision+1,updated_at=now() WHERE id=$1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM analyses WHERE user_id=$1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub async fn validate_runtime_role(pool: &PgPool, expected: &str) -> Result<(), AppError> {
    let valid: bool = sqlx::query_scalar("SELECT current_user=$1 AND NOT rolsuper AND NOT rolbypassrls FROM pg_roles WHERE rolname=current_user").bind(expected).fetch_one(pool).await?;
    if valid {
        Ok(())
    } else {
        Err(AppError::Invalid(
            "Runtime database role must be the restricted application role.",
        ))
    }
}

pub async fn catalog_page(
    pool: &PgPool,
    kind: &str,
    offset: i64,
    limit: i64,
) -> Result<serde_json::Value, AppError> {
    let (base, count_sql) = match kind {
        "skills" => (SKILLS_SQL, "SELECT count(*) FROM skills"),
        "occupations" => (OCCUPATIONS_SQL, "SELECT count(*) FROM occupations"),
        "institutions" => (INSTITUTIONS_SQL, "SELECT count(*) FROM institutions"),
        "trainings" => (TRAININGS_SQL, "SELECT count(*) FROM trainings WHERE aktif"),
        "provinces" => (
            "SELECT to_jsonb(nama) AS data FROM provinces ORDER BY nama",
            "SELECT count(*) FROM provinces",
        ),
        _ => return Err(AppError::NotFound),
    };
    // SQL fragments above are static, audited statements; user input is bound only as data.
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!("{base} LIMIT $1 OFFSET $2")))
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;
    let items: Vec<serde_json::Value> = rows.iter().map(|r| r.get("data")).collect();
    let total: i64 = sqlx::query_scalar(count_sql).fetch_one(pool).await?;
    Ok(serde_json::json!({"items":items,"total":total,"offset":offset,"limit":limit}))
}
