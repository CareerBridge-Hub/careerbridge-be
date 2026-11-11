//! Manual native LPK import using the same restricted role and locks as the scheduler.
use careerbridge_be::infrastructure::{config, db, extract::skillhub};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .json()
        .with_writer(std::io::stderr)
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["--test-api"] {
        println!(
            "{}",
            serde_json::to_string_pretty(&skillhub::probe().await?)?
        );
        return Ok(());
    }
    if args.first().map(String::as_str) != Some("--once")
        || args.iter().skip(1).any(|a| a != "--dry-run")
        || args.len() > 2
    {
        return Err("Usage: skillhub-import --once [--dry-run]".into());
    }
    let pool = db::connect_runtime_pool(&config::database_url()?, db::RuntimeRole::CatalogImporter)
        .await?;
    let report = skillhub::once(&pool, true, args.len() == 2).await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
