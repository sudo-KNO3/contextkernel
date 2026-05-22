//! Embedder trait — lets `ctxk-store` and `ctxk-retrieval` accept any
//! embedding backend (fastembed-rs by default, but swappable for OpenAI,
//! Ollama, sentence-transformers via a sidecar, etc.) without depending
//! on a concrete implementation crate.

use crate::Result;

pub trait EmbedderProvider: Send + Sync {
    /// Embed N texts in one batch. Output has the same length as input,
    /// each inner vector has [`dim`] dimensions.
    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;

    /// Human-readable model identifier (persisted in schema_meta so an
    /// upgrade triggers a re-embed).
    fn model_name(&self) -> &str;

    /// Vector dimensionality.
    fn dim(&self) -> usize;
}
