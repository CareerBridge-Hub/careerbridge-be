//! Run only with the schema-owner connection, never the runtime role.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let url = std::env::var("MIGRATION_DATABASE_URL")?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    println!("Migrations applied.");
    Ok(())
}
