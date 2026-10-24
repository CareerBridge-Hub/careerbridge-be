//! Application ports keep LLM and local embedding implementations independent.
pub mod error;
use async_trait::async_trait;
use error::AppError;
use serde_json::Value;

#[derive(Clone, Copy)]
pub enum EmbeddingKind {
    Query,
    Passage,
}
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(
        &self,
        texts: Vec<String>,
        kind: EmbeddingKind,
    ) -> Result<Vec<Vec<f32>>, AppError>;
    fn version(&self) -> &str;
}
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn json(&self, instruction: &str, data: Value) -> Result<Value, AppError>;
    fn model(&self) -> &str;
}

#[async_trait]
pub trait CvTextExtractor: Send + Sync {
    async fn extract(&self, bytes: &[u8]) -> Result<zeroize::Zeroizing<String>, AppError>;
}
