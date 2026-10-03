use careerbridge_be::{infrastructure::config::Config, presentation::http};
use actix_web::{App, HttpServer};

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let config = Config::from_env()?;

    HttpServer::new(|| App::new().configure(http::configure))
        .bind((config.host.as_str(), config.port))?
        .run()
        .await
}
