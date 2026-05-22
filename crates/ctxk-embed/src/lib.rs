//! Process-wide embedding singleton.
//!
//! Wraps `fastembed-rs` so the rest of ContextKernel can ignore ONNX
//! Runtime, model download, and tokenisation. The model is loaded lazily
//! on first call to [`Embedder::global`] and cached for the process
//! lifetime. Default: BAAI/bge-small-en-v1.5 (384-dim, ~33 MB).
//!
//! The model files cache to `$HF_HOME` (or fastembed's default), so the
//! first run downloads once and subsequent invocations are offline.

use ctxk_core::{EmbedderProvider, Result};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use once_cell::sync::OnceCell;
use std::sync::Mutex;

/// Number of dimensions produced by the default model.
pub const DEFAULT_DIM: usize = 384;
/// Human-readable name of the default model (matches what gets persisted
/// in `schema_meta` so we can detect mismatches and re-embed on upgrade).
pub const DEFAULT_MODEL: &str = "bge-small-en-v1.5";

static GLOBAL: OnceCell<Mutex<TextEmbedding>> = OnceCell::new();

/// Encapsulates the embedder so callers depend on a small surface, not on
/// `fastembed` directly. The model itself is loaded once and shared.
pub struct Embedder;

impl Embedder {
    /// Get a handle to the global embedder. Loads the ONNX model on first
    /// call (~2-3 s first time, instant after).
    fn global() -> &'static Mutex<TextEmbedding> {
        GLOBAL.get_or_init(|| {
            tracing::info!("loading embedding model: {}", DEFAULT_MODEL);
            let model = TextEmbedding::try_new(
                InitOptions::new(EmbeddingModel::BGESmallENV15)
                    .with_show_download_progress(true),
            )
            .expect("fastembed: failed to initialise BGE-small-en-v1.5");
            tracing::info!("embedding model ready ({}-dim)", DEFAULT_DIM);
            Mutex::new(model)
        })
    }

    /// Embed a batch of strings. Returns a `Vec<Vec<f32>>` aligned with
    /// the input order, each inner vector is `DEFAULT_DIM` floats.
    pub fn embed_batch(texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let model = Self::global().lock().unwrap();
        let docs: Vec<&str> = texts.iter().map(String::as_str).collect();
        model
            .embed(docs, None)
            .map_err(|e| ctxk_core::Error::Other(format!("embed_batch: {e}")))
    }

    /// Embed a single string.
    pub fn embed(text: &str) -> Result<Vec<f32>> {
        let mut out = Self::embed_batch(&[text.to_string()])?;
        out.pop()
            .ok_or_else(|| ctxk_core::Error::Other("embed: no vector returned".into()))
    }

    pub fn model_name() -> &'static str {
        DEFAULT_MODEL
    }
    pub fn dim() -> usize {
        DEFAULT_DIM
    }
}

impl EmbedderProvider for Embedder {
    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Self::embed_batch(texts)
    }

    fn model_name(&self) -> &str {
        DEFAULT_MODEL
    }

    fn dim(&self) -> usize {
        DEFAULT_DIM
    }
}

/// Serialise a vector to little-endian f32 bytes for SQLite BLOB storage.
pub fn vec_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(v.len() * 4);
    for x in v {
        buf.extend_from_slice(&x.to_le_bytes());
    }
    buf
}

/// Inverse of [`vec_to_bytes`].
pub fn bytes_to_vec(b: &[u8]) -> Vec<f32> {
    let mut out = Vec::with_capacity(b.len() / 4);
    for chunk in b.chunks_exact(4) {
        out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    out
}

/// Normalise a vector in place to unit length. Cosine similarity on
/// pre-normalised vectors collapses to a plain dot product.
pub fn normalise(v: &mut [f32]) {
    let mag = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag > 0.0 {
        for x in v.iter_mut() {
            *x /= mag;
        }
    }
}

/// Dot product of two same-length slices. Assumes pre-normalised inputs
/// for cosine similarity.
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}
