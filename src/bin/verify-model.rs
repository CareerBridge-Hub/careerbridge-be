//! Exercise the exact production loader without database or provider credentials.
use careerbridge_be::{
    application::{EmbeddingKind, EmbeddingProvider},
    infrastructure::embedding::LocalEmbedding,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cache = std::env::var("MODEL_CACHE_DIR").unwrap_or_else(|_| ".cache/fastembed".into());
    let model = LocalEmbedding::load(cache).await?;
    let vectors = model
        .embed(
            vec!["keterampilan administrasi".into()],
            EmbeddingKind::Query,
        )
        .await?;
    if vectors.len() != 1 || vectors[0].len() != 384 {
        return Err("Unexpected embedding output".into());
    }
    println!("Verified inference: {}", model.version());
    Ok(())
}
