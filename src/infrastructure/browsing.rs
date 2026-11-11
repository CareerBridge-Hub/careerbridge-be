//! Bounded catalog selection: filter/rank IDs in PostgreSQL before JSON hydration.
use super::{db, error::AppError};
use crate::domain::*;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, QueryBuilder, Row};
use std::collections::HashSet;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Clone, Deserialize, ToSchema, IntoParams)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Browse {
    #[serde(default)]
    #[param(minimum = 0, maximum = 100000)]
    pub offset: i64,
    #[serde(default = "page_size")]
    #[param(minimum = 1, maximum = 100)]
    pub limit: i64,
    #[serde(default)]
    #[param(max_length = 120)]
    pub q: String,
    #[serde(default)]
    #[param(max_length = 120)]
    pub category: String,
    #[serde(default)]
    #[param(max_length = 120)]
    pub province: String,
    #[serde(default)]
    pub cost: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub ids: String,
    pub skill_id: Option<Uuid>,
    pub occupation_id: Option<Uuid>,
    #[serde(default)]
    pub recommended: bool,
}
fn page_size() -> i64 {
    20
}
impl Default for Browse {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: 20,
            q: String::new(),
            category: String::new(),
            province: String::new(),
            cost: String::new(),
            mode: String::new(),
            ids: String::new(),
            skill_id: None,
            occupation_id: None,
            recommended: false,
        }
    }
}
impl Browse {
    pub fn validate(&self) -> Result<Vec<Uuid>, AppError> {
        if !(1..=100).contains(&self.limit)
            || !(0..=100_000).contains(&self.offset)
            || [self.q.len(), self.category.len(), self.province.len()]
                .iter()
                .any(|n| *n > 120)
            || !matches!(self.cost.as_str(), "" | "gratis" | "berbayar" | "unknown")
            || !matches!(self.mode.as_str(), "" | "daring" | "luring" | "unknown")
        {
            return Err(AppError::Invalid("Parameter halaman tidak valid."));
        }
        if self.ids.is_empty() {
            return Ok(vec![]);
        }
        let ids: Vec<_> = self.ids.split(',').collect();
        if ids.len() > 100 {
            return Err(AppError::Invalid("Maksimal 100 ID."));
        }
        ids.into_iter()
            .map(|id| Uuid::parse_str(id).map_err(|_| AppError::Invalid("ID tidak valid.")))
            .collect()
    }
}
fn like(text: &str) -> String {
    format!(
        "%{}%",
        text.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}
fn base(kind: &str) -> Result<(&'static str, &'static str, &'static str), AppError> {
    match kind {
        "skills" => Ok((db::SKILLS_SQL, "skills s", "s")),
        "occupations" => Ok((db::OCCUPATIONS_SQL, "occupations o", "o")),
        "institutions" => Ok((
            db::INSTITUTIONS_SQL,
            "institutions i JOIN provinces p ON p.id=i.province_id",
            "i",
        )),
        "trainings" => Ok((
            db::TRAININGS_SQL,
            "trainings t JOIN institutions i ON i.id=t.institution_id JOIN provinces p ON p.id=i.province_id",
            "t",
        )),
        _ => Err(AppError::NotFound),
    }
}
/// `db` decides visibility: a user transaction also returns that user's own occupation.
pub async fn by_ids<'e>(
    db: impl sqlx::PgExecutor<'e>,
    kind: &str,
    ids: &[Uuid],
) -> Result<Vec<Value>, AppError> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let (base, _, alias) = base(kind)?;
    let (select, order) = base.rsplit_once(" ORDER BY ").ok_or(AppError::Internal)?;
    let join = if kind == "trainings" {
        " AND "
    } else {
        " WHERE "
    };
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
        "{select}{join}{alias}.id=ANY($1) ORDER BY {order}"
    )))
    .bind(ids)
    .fetch_all(db)
    .await?;
    let values: Vec<Value> = rows.iter().map(|r| r.get("data")).collect();
    let mut seen = HashSet::new();
    Ok(ids
        .iter()
        .filter(|id| seen.insert(**id))
        .filter_map(|id| {
            values
                .iter()
                .find(|v| v["id"].as_str() == Some(id.to_string().as_str()))
                .cloned()
        })
        .collect())
}
fn finish(mut query: QueryBuilder<Postgres>, order: &str, q: &Browse) -> QueryBuilder<Postgres> {
    query
        .push("), page AS (SELECT id FROM filtered ORDER BY ")
        .push(order)
        .push(" LIMIT ")
        .push_bind(q.limit)
        .push(" OFFSET ")
        .push_bind(q.offset)
        .push(
            ") SELECT (SELECT count(*) FROM filtered) AS total, ARRAY(SELECT id FROM page) AS ids",
        );
    query
}
pub async fn catalog(
    pool: &PgPool,
    kind: &str,
    q: &Browse,
    gaps: Option<&JobMatch>,
) -> Result<Value, AppError> {
    let ids = q.validate()?;
    if kind == "provinces" {
        let rows = sqlx::query_scalar::<_, String>(
            "SELECT nama FROM provinces WHERE nama ILIKE $1 ORDER BY nama LIMIT $2 OFFSET $3",
        )
        .bind(like(&q.q))
        .bind(q.limit)
        .bind(q.offset)
        .fetch_all(pool)
        .await?;
        let total: i64 = sqlx::query_scalar("SELECT count(*) FROM provinces WHERE nama ILIKE $1")
            .bind(like(&q.q))
            .fetch_one(pool)
            .await?;
        return Ok(json!({"items":rows,"total":total,"offset":q.offset,"limit":q.limit}));
    }
    let (_, from, alias) = base(kind)?;
    let w: Vec<Uuid> = gaps
        .map(|m| m.missing_wajib.iter().map(|s| s.skill_id).collect())
        .unwrap_or_default();
    let o: Vec<Uuid> = gaps
        .map(|m| m.missing_opsional.iter().map(|s| s.skill_id).collect())
        .unwrap_or_default();
    let mut query = QueryBuilder::new("WITH filtered AS MATERIALIZED (SELECT ");
    query.push(alias).push(".id,").push(alias).push(".nama");
    if kind == "trainings" && q.recommended {
        query.push(",cover.w,cover.o,coalesce(t.biaya=0,false) AS free");
    }
    query.push(" FROM ").push(from);
    if kind == "trainings" && q.recommended {
        query
            .push(" LEFT JOIN LATERAL (SELECT count(*) FILTER (WHERE skill_id=ANY(")
            .push_bind(w)
            .push(")) AS w,count(*) FILTER (WHERE skill_id=ANY(")
            .push_bind(o)
            .push(")) AS o FROM training_skills WHERE training_id=t.id) cover ON true");
    }
    query.push(" WHERE true");
    if kind == "trainings" {
        query.push(" AND t.aktif");
    }
    if !ids.is_empty() {
        query
            .push(" AND ")
            .push(alias)
            .push(".id=ANY(")
            .push_bind(ids)
            .push(")");
    }
    if !q.q.is_empty() {
        query
            .push(" AND (")
            .push(alias)
            .push(".nama ILIKE ")
            .push_bind(like(&q.q));
        if kind == "skills" {
            query.push(" OR EXISTS(SELECT 1 FROM skill_aliases a WHERE a.skill_id=s.id AND a.alias ILIKE ").push_bind(like(&q.q)).push(")");
        }
        if kind == "occupations" {
            query.push(" OR o.bidang ILIKE ").push_bind(like(&q.q));
        }
        query.push(")");
    }
    if !q.category.is_empty() {
        if kind == "skills" {
            query.push(" AND s.kategori=").push_bind(q.category.clone());
        }
        if kind == "occupations" {
            query.push(" AND o.bidang=").push_bind(q.category.clone());
        }
    }
    if !q.province.is_empty() && matches!(kind, "trainings" | "institutions") {
        if kind == "trainings" {
            query
                .push(" AND (t.moda='daring' OR p.nama=")
                .push_bind(q.province.clone())
                .push(")");
        } else {
            query.push(" AND p.nama=").push_bind(q.province.clone());
        }
    }
    if kind == "trainings" {
        match q.cost.as_str() {
            "gratis" => {
                query.push(" AND t.biaya=0");
            }
            "berbayar" => {
                query.push(" AND t.biaya>0");
            }
            "unknown" => {
                query.push(" AND t.biaya IS NULL");
            }
            _ => {}
        }
        match q.mode.as_str() {
            "unknown" => {
                query.push(" AND t.moda IS NULL");
            }
            "daring" | "luring" => {
                query.push(" AND t.moda=").push_bind(q.mode.clone());
            }
            _ => {}
        }
        if q.recommended {
            query.push(" AND (cover.w+cover.o)>0");
        }
        if let Some(id) = q.skill_id {
            query.push(" AND EXISTS(SELECT 1 FROM training_skills x WHERE x.training_id=t.id AND x.skill_id=").push_bind(id).push(")");
        }
    }
    let order = if kind == "trainings" && q.recommended {
        "w DESC,(w+o) DESC,free DESC,nama,id"
    } else {
        "nama,id"
    };
    let mut query = finish(query, order, q);
    let row = query.build().fetch_one(pool).await?;
    let ids: Vec<Uuid> = row.get("ids");
    let items = by_ids(pool, kind, &ids).await?;
    let mut result =
        json!({"items":items,"total":row.get::<i64,_>("total"),"offset":q.offset,"limit":q.limit});
    if kind == "trainings" {
        let institution_ids: Vec<Uuid> = items
            .iter()
            .filter_map(|v| v["institutionId"].as_str()?.parse().ok())
            .collect();
        result["institutions"] = json!(by_ids(pool, "institutions", &institution_ids).await?);
        let skill_ids: Vec<Uuid> = items
            .iter()
            .flat_map(|v| v["skillIds"].as_array().into_iter().flatten())
            .filter_map(|v| v.as_str()?.parse().ok())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        result["skills"] = json!(by_ids(pool, "skills", &skill_ids).await?);
    }
    Ok(result)
}
pub async fn matches(pool: &PgPool, user: Uuid, q: &Browse) -> Result<Value, AppError> {
    let ids = q.validate()?;
    // One user transaction: RLS adds the caller's own occupation to the catalog ones.
    let mut tx = db::active_tx(pool, user).await?;
    let have: Vec<Uuid> = sqlx::query_scalar("SELECT skill_id FROM user_skills WHERE user_id=$1")
        .bind(user)
        .fetch_all(&mut *tx)
        .await?;
    let mut query = QueryBuilder::new(
        "WITH weighted AS (SELECT o.id,o.nama,coalesce(round(100.0*sum(CASE WHEN os.skill_id=ANY(",
    );
    query.push_bind(have.clone()).push(") THEN CASE WHEN os.tipe='wajib' THEN 2 ELSE 1 END ELSE 0 END)/nullif(sum(CASE WHEN os.skill_id IS NULL THEN 0 WHEN os.tipe='wajib' THEN 2 ELSE 1 END),0)),0) AS score FROM occupations o LEFT JOIN occupation_skills os ON os.occupation_id=o.id WHERE true");
    if !ids.is_empty() {
        query.push(" AND o.id=ANY(").push_bind(ids).push(")");
    }
    if !q.q.is_empty() {
        query
            .push(" AND (o.nama ILIKE ")
            .push_bind(like(&q.q))
            .push(" OR o.bidang ILIKE ")
            .push_bind(like(&q.q))
            .push(")");
    }
    if !q.category.is_empty() {
        query.push(" AND o.bidang=").push_bind(q.category.clone());
    }
    query.push(" GROUP BY o.id), filtered AS MATERIALIZED (SELECT * FROM weighted");
    let mut query = finish(query, "score DESC,nama,id", q);
    let row = query.build().fetch_one(&mut *tx).await?;
    let jobs = by_ids(&mut *tx, "occupations", &row.get::<Vec<Uuid>, _>("ids")).await?;
    let fields: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT bidang FROM occupations ORDER BY bidang LIMIT 100")
            .fetch_all(&mut *tx)
            .await?;
    tx.commit().await?;
    let have: HashSet<_> = have.into_iter().collect();
    let matches: Vec<JobMatch> = jobs
        .into_iter()
        .map(|v| {
            serde_json::from_value::<Occupation>(v)
                .map(|o| match_job(&o, &have))
                .map_err(|_| AppError::Internal)
        })
        .collect::<Result<_, _>>()?;
    let skill_ids: Vec<Uuid> = matches
        .iter()
        .flat_map(|m| m.job.skills.iter().map(|s| s.skill_id))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    Ok(
        json!({"items":matches,"skills":by_ids(pool,"skills",&skill_ids).await?,"fields":fields,"total":row.get::<i64,_>("total"),"offset":q.offset,"limit":q.limit}),
    )
}
pub async fn job(pool: &PgPool, user: Uuid, id: Uuid) -> Result<JobMatch, AppError> {
    let value = matches(
        pool,
        user,
        &Browse {
            ids: id.to_string(),
            limit: 1,
            ..Default::default()
        },
    )
    .await?;
    serde_json::from_value(
        value["items"]
            .as_array()
            .and_then(|v| v.first())
            .cloned()
            .ok_or(AppError::NotFound)?,
    )
    .map_err(|_| AppError::Internal)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browsing_is_bounded_and_literal_search_escapes_wildcards() {
        assert_eq!(like("a%_\\b"), "%a\\%\\_\\\\b%");
        for (offset, limit) in [(-1, 20), (0, 0), (0, 101), (100001, 20)] {
            assert!(
                Browse {
                    offset,
                    limit,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            Browse {
                ids: "not-a-uuid".into(),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Browse {
                ids: vec![Uuid::nil().to_string(); 101].join(","),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(Browse::default().validate().is_ok());
    }
}
