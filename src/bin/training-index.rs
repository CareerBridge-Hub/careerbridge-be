//! Explicit bounded maintenance for the real training catalog; never accesses user data.
use careerbridge_be::domain::{Occupation, match_job};
use careerbridge_be::infrastructure::{
    browsing::{self, Browse},
    config,
    db::{self, RuntimeRole},
    embedding::LocalEmbedding,
    training_search,
};
use sqlx::Row;
use uuid::Uuid;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !((args.len() == 1 && matches!(args[0].as_str(), "--status" | "--backfill"))
        || (args.len() == 2 && args[0] == "--preview"))
    {
        return Err("Usage: training-index --status|--backfill|--preview OCCUPATION_UUID".into());
    }
    let url = config::database_url()?;
    let read = db::connect_runtime_pool(&url, RuntimeRole::Api).await?;
    if args[0] != "--status" {
        let model = LocalEmbedding::load(
            std::env::var("MODEL_CACHE_DIR").unwrap_or_else(|_| ".cache/fastembed".into()),
        )
        .await?;
        if args[0] == "--backfill" {
            let write = db::connect_runtime_pool(&url, RuntimeRole::Worker).await?;
            let count = training_search::backfill(&read, &write, model.as_ref(), 400).await?;
            println!("Indexed {count} real training programs");
        } else {
            // Public occupation, all skills treated as missing; no account/profile accessed.
            let id: Uuid = args[1].parse()?;
            let occupation: Occupation = serde_json::from_value(
                browsing::by_ids(&read, "occupations", &[id])
                    .await?
                    .into_iter()
                    .next()
                    .ok_or("Occupation not found")?,
            )?;
            let gaps = match_job(&occupation, &Default::default());
            let page = training_search::page(
                &read,
                model.as_ref(),
                &Browse {
                    recommended: true,
                    limit: 3,
                    ..Default::default()
                },
                Some(&gaps),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&page)?);
        }
    }
    let row=sqlx::query("SELECT count(*) FILTER(WHERE aktif AND NOT sample) real_programs,count(*) FILTER(WHERE aktif AND sample) sample_programs,count(*) FILTER(WHERE aktif AND NOT sample AND embedding IS NOT NULL AND embedding_hash IS NOT NULL) indexed FROM trainings").fetch_one(&read).await?;
    println!(
        "real_programs={} sample_programs={} indexed={}",
        row.get::<i64, _>("real_programs"),
        row.get::<i64, _>("sample_programs"),
        row.get::<i64, _>("indexed")
    );
    Ok(())
}
