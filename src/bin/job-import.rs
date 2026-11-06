//! Use --preview, --once [--force], or --daemon. Never needs Supabase Auth/LLM secrets.
use careerbridge_be::infrastructure::jobs::{self, Collector, Settings};
use std::io::Write;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .json()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let settings = Settings::from_env()?;
    match args.first().map(String::as_str) {
        Some("--preview") if args.len() == 1 => {
            let collector = Collector::new()?;
            let mut jobs = vec![];
            for source in &settings.sources {
                jobs.extend(collector.collect(*source).await?);
            }
            std::fs::create_dir_all(".cache/software-jobs")?;
            let path = ".cache/software-jobs/indonesia-expanded.csv";
            let mut file = std::fs::File::create(path)?;
            file.write_all(&[0xef, 0xbb, 0xbf])?;
            writeln!(
                file,
                "source,source_id,company,title,country,city,region,workplace,apply_url,source_url,posted_at,description,requirements"
            )?;
            for job in &jobs {
                let fields = [
                    job.source.clone(),
                    job.source_id.clone(),
                    job.company.clone(),
                    job.title.clone(),
                    job.country.clone(),
                    job.city.clone(),
                    job.region.clone(),
                    job.workplace.clone(),
                    job.apply_url.clone(),
                    job.source_url.clone(),
                    job.posted_at.map(|d| d.to_rfc3339()).unwrap_or_default(),
                    job.description.clone(),
                    job.requirements.clone(),
                ];
                writeln!(
                    file,
                    "{}",
                    fields
                        .into_iter()
                        .map(|v| {
                            let safe = if v.trim_start().starts_with(['=', '+', '-', '@']) {
                                format!("'{v}")
                            } else {
                                v
                            };
                            format!("\"{}\"", safe.replace('"', "\"\""))
                        })
                        .collect::<Vec<_>>()
                        .join(",")
                )?;
            }
            println!(
                "{}",
                serde_json::json!({"path":path,"count":jobs.len(),"databaseWrites":0})
            );
        }
        Some("--status") if args.len() == 1 => {
            let pool = jobs::importer_pool().await?;
            let status:serde_json::Value=sqlx::query_scalar("SELECT jsonb_agg(j ORDER BY key) FROM (SELECT s.key,s.last_success_at,s.last_count,s.last_error,s.next_run_at,s.lease_until,(SELECT count(*) FROM job_vacancies v WHERE v.source=s.key AND status='active') active_count FROM job_sources s) j").fetch_one(&pool).await?;
            println!("{}", serde_json::to_string_pretty(&status)?);
        }
        Some("--once") if args.len() == 1 || (args.len() == 2 && args[1] == "--force") => {
            let pool = jobs::importer_pool().await?;
            if !jobs::run_pass(&pool, &settings, args.len() == 2).await {
                return Err("One or more vacancy sources failed; see sanitized logs".into());
            }
            println!("Vacancy import pass completed.");
        }
        Some("--daemon") if args.len() == 1 => {
            jobs::schedule(jobs::importer_pool().await?, settings).await
        }
        _ => {
            return Err(
                "Usage: job-import --preview | --once [--force] | --daemon | --status".into(),
            );
        }
    }
    Ok(())
}
