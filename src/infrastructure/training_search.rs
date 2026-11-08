//! Local, grounded training retrieval. Semantic matches never become verified skill mappings.
use super::{
    browsing::{self, Browse},
    error::AppError,
};
use crate::{
    application::{EmbeddingKind, EmbeddingProvider},
    domain::{JobMatch, Skill},
};
use pgvector::Vector;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, QueryBuilder, Row};
use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use uuid::Uuid;

pub const MIN_SIMILARITY: f64 = 0.88;
type CachedVector = (Instant, Vec<f32>);
static QUERIES: OnceLock<Mutex<HashMap<String, CachedVector>>> = OnceLock::new();

// Catalog topics/search queries only: hashed keys, bounded memory, no account identifiers.
async fn query_vectors(
    model: &dyn EmbeddingProvider,
    texts: Vec<String>,
) -> Result<Vec<Vec<f32>>, AppError> {
    let cache = QUERIES.get_or_init(|| Mutex::new(HashMap::new()));
    let keys: Vec<String> = texts
        .iter()
        .map(|t| format!("{:x}", Sha256::digest(format!("{}:{t}", model.version()))))
        .collect();
    let mut result = vec![Vec::new(); texts.len()];
    let mut missing = Vec::new();
    {
        let mut cached = cache.lock().map_err(|_| AppError::Internal)?;
        cached.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(900));
        for (i, key) in keys.iter().enumerate() {
            if let Some((_, v)) = cached.get(key) {
                result[i] = v.clone()
            } else {
                missing.push(i)
            }
        }
    }
    for chunk in missing.chunks(100) {
        let vectors = model
            .embed(
                chunk.iter().map(|i| texts[*i].clone()).collect(),
                EmbeddingKind::Query,
            )
            .await?;
        if vectors.len() != chunk.len()
            || vectors
                .iter()
                .any(|v| v.len() != 384 || v.iter().any(|n| !n.is_finite()))
        {
            return Err(AppError::Internal);
        }
        let mut cached = cache.lock().map_err(|_| AppError::Internal)?;
        if cached.len() + vectors.len() > 512 {
            cached.clear();
        }
        for (i, v) in chunk.iter().zip(vectors) {
            cached.insert(keys[*i].clone(), (Instant::now(), v.clone()));
            result[*i] = v;
        }
    }
    Ok(result)
}
fn like(text: &str) -> String {
    format!(
        "%{}%",
        text.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

pub async fn page(
    pool: &PgPool,
    model: &dyn EmbeddingProvider,
    q: &Browse,
    gaps: Option<&JobMatch>,
) -> Result<Value, AppError> {
    let ids = q.validate()?;
    let mut wanted: Vec<(Uuid, i32)> = if q.recommended {
        let gaps = gaps.ok_or(AppError::Invalid("Pilih pekerjaan untuk rekomendasi."))?;
        gaps.missing_wajib
            .iter()
            .map(|s| (s.skill_id, 2))
            .chain(gaps.missing_opsional.iter().map(|s| (s.skill_id, 1)))
            .collect()
    } else {
        q.skill_id.map(|id| vec![(id, 1)]).unwrap_or_default()
    };
    if let Some(id) = q.skill_id {
        if q.recommended && !wanted.iter().any(|(skill, _)| *skill == id) {
            return Err(AppError::Invalid(
                "Skill harus berasal dari kekurangan skill pekerjaan yang dipilih.",
            ));
        }
        wanted.retain(|(skill, _)| *skill == id);
    }
    if wanted.len() > 100 {
        return Err(AppError::Invalid("Maksimal 100 topik skill."));
    }
    let skills: Vec<Skill> = browsing::by_ids(
        pool,
        "skills",
        &wanted.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
    )
    .await?
    .into_iter()
    .map(|v| serde_json::from_value(v).map_err(|_| AppError::Internal))
    .collect::<Result<_, _>>()?;
    if skills.len() != wanted.len() {
        return Err(AppError::Invalid("Skill tidak ditemukan."));
    }
    let topic_ids: Vec<Uuid> = skills.iter().map(|s| s.id).collect();
    let names: Vec<String> = skills.iter().map(|s| s.nama.clone()).collect();
    let weights: Vec<i32> = skills
        .iter()
        .map(|s| {
            wanted
                .iter()
                .find(|(id, _)| *id == s.id)
                .map_or(1, |(_, weight)| *weight)
        })
        .collect();
    let mut texts: Vec<String> = skills
        .iter()
        .map(|s| format!("{} {}", s.nama, s.alias.join(", ")))
        .collect();
    if !q.q.trim().is_empty() {
        texts.push(q.q.trim().to_owned());
    }
    let mut embedding_failed = false;
    let vectors = match query_vectors(model, texts.clone()).await {
        Ok(v) => v
            .into_iter()
            .map(|v| Some(Vector::from(v)))
            .collect::<Vec<_>>(),
        Err(_) => {
            embedding_failed = true;
            tracing::warn!(
                "training query embeddings unavailable; using verified mappings and text retrieval"
            );
            vec![None; texts.len()]
        }
    };
    let topic_vectors: Vec<Option<Vector>> = vectors.iter().take(names.len()).cloned().collect();
    let search_vector = if q.q.trim().is_empty() {
        None
    } else {
        vectors.last().cloned().flatten()
    };
    let mut sql: QueryBuilder<Postgres> =
        QueryBuilder::new("WITH topics AS (SELECT * FROM unnest(");
    sql.push_bind(topic_ids).push("::uuid[],").push_bind(names).push("::text[],").push_bind(weights).push("::int[],").push_bind(topic_vectors).push("::vector[]) AS x(skill_id,name,weight,vector)), eligible AS MATERIALIZED (SELECT t.*,i.nama AS institution_name FROM trainings t JOIN institutions i ON i.id=t.institution_id LEFT JOIN provinces p ON p.id=i.province_id WHERE t.aktif AND NOT t.sample");
    if !ids.is_empty() {
        sql.push(" AND t.id=ANY(").push_bind(ids).push(")");
    }
    if !q.province.is_empty() {
        sql.push(" AND (t.moda='daring' OR p.nama=")
            .push_bind(q.province.clone())
            .push(")");
    }
    match q.cost.as_str() {
        "gratis" => {
            sql.push(" AND t.biaya=0");
        }
        "berbayar" => {
            sql.push(" AND t.biaya>0");
        }
        "unknown" => {
            sql.push(" AND t.biaya IS NULL");
        }
        _ => {}
    }
    match q.mode.as_str() {
        "unknown" => {
            sql.push(" AND t.moda IS NULL");
        }
        "daring" | "luring" => {
            sql.push(" AND t.moda=").push_bind(q.mode.clone());
        }
        _ => {}
    }
    sql.push("), scored AS MATERIALIZED (SELECT e.id,e.nama,e.biaya,e.source,e.source_url,e.imported_at,coalesce(r.mapped_weight,0) mapped_weight,coalesce(r.text_weight,0) text_weight,r.similarity,r.confirmed_ids,r.topic_ids,(e.embedding IS NOT NULL AND e.embedding_hash IS NOT NULL AND e.embedding_model=").push_bind(model.version().to_owned()).push(") indexed, CASE WHEN e.embedding IS NOT NULL AND e.embedding_hash IS NOT NULL AND e.embedding_model=").push_bind(model.version().to_owned()).push(" THEN 1-(e.embedding <=> ").push_bind(search_vector).push("::vector) END search_similarity,");
    let pattern = like(q.q.trim());
    sql.push("(e.nama ILIKE ").push_bind(pattern.clone()).push(" OR e.institution_name ILIKE ").push_bind(pattern.clone()).push(" OR EXISTS(SELECT 1 FROM training_skills ts JOIN skills sk ON sk.id=ts.skill_id WHERE ts.training_id=e.id AND (sk.nama ILIKE ").push_bind(pattern.clone()).push(" OR EXISTS(SELECT 1 FROM skill_aliases sa WHERE sa.skill_id=sk.id AND sa.alias ILIKE ").push_bind(pattern).push(")))) search_text FROM eligible e LEFT JOIN LATERAL (SELECT sum(CASE WHEN confirmed THEN weight ELSE 0 END) mapped_weight,sum(CASE WHEN lexical THEN weight ELSE 0 END) text_weight,max(similarity) similarity,array_agg(skill_id) FILTER(WHERE confirmed) confirmed_ids,array_agg(skill_id) FILTER(WHERE confirmed OR lexical OR similarity>=").push_bind(MIN_SIMILARITY).push(") topic_ids FROM (SELECT topic.*,EXISTS(SELECT 1 FROM training_skills ts WHERE ts.training_id=e.id AND ts.skill_id=topic.skill_id) confirmed,to_tsvector('simple',e.nama) @@ plainto_tsquery('simple',topic.name) lexical,CASE WHEN e.embedding IS NOT NULL AND e.embedding_hash IS NOT NULL AND e.embedding_model=").push_bind(model.version().to_owned()).push(" THEN 1-(e.embedding <=> topic.vector) END similarity FROM topics topic) matches) r ON true), filtered AS MATERIALIZED (SELECT * FROM scored WHERE true");
    if q.recommended || q.skill_id.is_some() {
        sql.push(" AND (mapped_weight>0 OR text_weight>0 OR similarity>=")
            .push_bind(MIN_SIMILARITY)
            .push(")");
    }
    if !q.q.trim().is_empty() {
        sql.push(" AND (search_text OR search_similarity>=")
            .push_bind(MIN_SIMILARITY)
            .push(")");
    }
    let ranked = q.recommended || q.skill_id.is_some() || !q.q.trim().is_empty();
    sql.push("), page AS (SELECT * FROM filtered ORDER BY ");
    if ranked {
        sql.push("mapped_weight DESC,text_weight DESC,search_text DESC,greatest(similarity,search_similarity) DESC NULLS LAST,(biaya=0) DESC NULLS LAST,");
    }
    sql.push("nama,id LIMIT ").push_bind(q.limit).push(" OFFSET ").push_bind(q.offset).push(") SELECT (SELECT count(*) FROM filtered) total,ARRAY(SELECT id FROM page) ids,(SELECT count(*) FROM scored) catalog_count,(SELECT count(*) FROM scored WHERE indexed) indexed_count,coalesce((SELECT jsonb_object_agg(id,jsonb_build_object('method',CASE WHEN mapped_weight>0 THEN 'mapped' WHEN text_weight>0 THEN 'text' WHEN search_text AND ").push_bind(!q.q.trim().is_empty()).push(" THEN 'text' WHEN greatest(similarity,search_similarity) IS NOT NULL THEN 'semantic' ELSE 'catalog' END,'confirmedSkillIds',coalesce(confirmed_ids,'{}'::uuid[]),'relatedSkillIds',coalesce(topic_ids,'{}'::uuid[]),'similarity',greatest(similarity,search_similarity),'source',source,'sourceUrl',source_url,'importedAt',imported_at)) FROM page),'{}'::jsonb) evidence");
    let row = sql.build().fetch_one(pool).await?;
    let page_ids: Vec<Uuid> = row.get("ids");
    let items = browsing::by_ids(pool, "trainings", &page_ids).await?;
    let inst_ids: Vec<Uuid> = items
        .iter()
        .filter_map(|t| t["institutionId"].as_str()?.parse().ok())
        .collect();
    let skill_ids: Vec<Uuid> = items
        .iter()
        .flat_map(|t| t["skillIds"].as_array().into_iter().flatten())
        .filter_map(|s| s.as_str()?.parse().ok())
        .chain(skills.iter().map(|s| s.id))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let catalog_count: i64 = row.get("catalog_count");
    let indexed_count: i64 = row.get("indexed_count");
    let notice = if embedding_failed {
        Some(
            "Pencarian semantik sementara tidak tersedia; hasil memakai pemetaan skill dan pencarian teks.",
        )
    } else if ranked && indexed_count < catalog_count {
        Some(
            "Sebagian program belum memiliki indeks semantik terbaru. Hasil yang tersedia tetap memakai data asli.",
        )
    } else if q.recommended && skills.is_empty() {
        Some(
            "Semua skill pekerjaan ini sudah dimiliki. Pilih semua pelatihan untuk menjelajahi katalog.",
        )
    } else {
        None
    };
    Ok(
        json!({"items":items,"total":row.get::<i64,_>("total"),"offset":q.offset,"limit":q.limit,
        "institutions":browsing::by_ids(pool,"institutions",&inst_ids).await?,"skills":browsing::by_ids(pool,"skills",&skill_ids).await?,"evidence":row.get::<Value,_>("evidence"),
        "retrieval":{"mode":if !ranked{"catalog"}else if embedding_failed{"text"}else{"hybrid"},"model":model.version(),"minimumSimilarity":MIN_SIMILARITY,"catalogCount":catalog_count,"indexedCount":indexed_count,"notice":notice,"sampleDataIncluded":false}}),
    )
}

/// Backfill missing/stale real program vectors in bounded batches. No LLM or personal data.
pub async fn backfill(
    read: &PgPool,
    write: &PgPool,
    model: &dyn EmbeddingProvider,
    max_batches: usize,
) -> Result<usize, AppError> {
    let mut updated = 0;
    for _ in 0..max_batches {
        let rows=sqlx::query("SELECT t.id,t.updated_at,t.nama || ' ' || coalesce((SELECT string_agg(s.nama,' ' ORDER BY ts.skill_id) FROM training_skills ts JOIN skills s ON s.id=ts.skill_id WHERE ts.training_id=t.id),'') passage FROM trainings t WHERE t.aktif AND NOT t.sample AND (t.embedding IS NULL OR t.embedding_model IS DISTINCT FROM $1 OR t.embedding_hash IS NULL) ORDER BY t.id LIMIT 32")
            .bind(model.version()).fetch_all(read).await?;
        if rows.is_empty() {
            break;
        }
        let texts: Vec<String> = rows.iter().map(|r| r.get("passage")).collect();
        let vectors = model.embed(texts.clone(), EmbeddingKind::Passage).await?;
        if vectors.len() != rows.len()
            || vectors
                .iter()
                .any(|v| v.len() != 384 || v.iter().any(|n| !n.is_finite()))
        {
            return Err(AppError::Internal);
        }
        let ids: Vec<Uuid> = rows.iter().map(|r| r.get("id")).collect();
        let versions: Vec<chrono::DateTime<chrono::Utc>> =
            rows.iter().map(|r| r.get("updated_at")).collect();
        let hashes: Vec<String> = texts
            .iter()
            .map(|t| format!("{:x}", Sha256::digest(t.as_bytes())))
            .collect();
        let affected=sqlx::query("UPDATE trainings t SET embedding=x.embedding,embedding_model=$5,embedding_hash=x.hash,embedded_at=now() FROM unnest($1::uuid[],$2::timestamptz[],$3::text[],$4::vector[]) AS x(id,version,hash,embedding) WHERE t.id=x.id AND t.updated_at=x.version")
            .bind(ids).bind(versions).bind(hashes).bind(vectors.into_iter().map(Vector::from).collect::<Vec<_>>()).bind(model.version()).execute(write).await?.rows_affected();
        updated += affected as usize;
        tokio::task::yield_now().await;
    }
    Ok(updated)
}
