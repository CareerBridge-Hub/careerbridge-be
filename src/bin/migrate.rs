//! Use SUPABASE_DB_URL with schema-owner credentials for migration work.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let url = careerbridge_be::infrastructure::config::database_url()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(careerbridge_be::infrastructure::config::database_options(
            &url,
        )?)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    println!("Migrations applied.");
    Ok(())
}
