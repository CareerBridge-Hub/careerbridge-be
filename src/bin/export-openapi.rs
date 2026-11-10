//! Export the same Rust-generated contract served by the API.
use careerbridge_be::presentation::api_doc::ApiDoc;
use utoipa::OpenApi;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", ApiDoc::openapi().to_pretty_json()?);
    Ok(())
}
