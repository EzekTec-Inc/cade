//! Local cross-encoder inference using ONNX Runtime.
//!
//! Loads `ms-marco-MiniLM-L-6-v2` (or a user-supplied model) and runs
//! sequence-classification inference on `(query, document)` pairs.

#![cfg(feature = "local")]

use crate::reranker::ToolDocument;
use crate::{Error, Result};
use ndarray::Array2;
use ort::session::Session;
use std::path::{Path, PathBuf};
use tokenizers::Tokenizer;

const DEFAULT_HF_REPO: &str = "cross-encoder/ms-marco-MiniLM-L-6-v2";
const ONNX_FILE: &str = "onnx/model.onnx";
const TOKENIZER_FILE: &str = "tokenizer.json";

/// A loaded local cross-encoder model ready for inference.
pub struct LocalModel {
    session: std::sync::Mutex<Session>,
    tokenizer: Tokenizer,
}

impl LocalModel {
    /// Load the model from a custom path, or download the default from
    /// HuggingFace Hub into `~/.cache/cade/models/reranker/`.
    pub async fn load(model_dir: Option<&Path>) -> Result<Self> {
        let dir = match model_dir {
            Some(p) => p.to_path_buf(),
            None => default_cache_dir()?,
        };

        let model_path = dir.join("model.onnx");
        let tokenizer_path = dir.join("tokenizer.json");

        // Download if missing.
        if !model_path.exists() || !tokenizer_path.exists() {
            download_model(&dir).await?;
        }

        tracing::info!("[reranker] loading ONNX model from {}", model_path.display());

        let session = Session::builder()
            .map_err(|e| Error::custom(format!("ort session builder: {e}")))?
            .commit_from_file(&model_path)
            .map_err(|e| Error::custom(format!("ort load model: {e}")))?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| Error::custom(format!("tokenizer load: {e}")))?;

        tracing::info!("[reranker] model loaded successfully");

        Ok(Self {
            session: std::sync::Mutex::new(session),
            tokenizer,
        })
    }

    /// Score `(query, document)` pairs and return the top-N documents.
    pub fn rerank(
        &self,
        query: &str,
        docs: &[ToolDocument],
        top_n: usize,
    ) -> Result<Vec<ToolDocument>> {
        if docs.is_empty() {
            return Ok(vec![]);
        }

        // Tokenize all (query, document) pairs.
        let pairs: Vec<tokenizers::EncodeInput> = docs
            .iter()
            .map(|d| tokenizers::EncodeInput::Dual(
                tokenizers::InputSequence::from(query),
                tokenizers::InputSequence::from(d.text.as_str()),
            ))
            .collect();

        let encodings = self
            .tokenizer
            .encode_batch(pairs, true)
            .map_err(|e| Error::custom(format!("tokenize: {e}")))?;

        // Find max length for padding.
        let max_len = encodings.iter().map(|e| e.get_ids().len()).max().unwrap_or(0);
        let batch_size = encodings.len();

        // Build padded input tensors: input_ids, attention_mask, token_type_ids.
        let mut input_ids = Array2::<i64>::zeros((batch_size, max_len));
        let mut attention_mask = Array2::<i64>::zeros((batch_size, max_len));
        let mut token_type_ids = Array2::<i64>::zeros((batch_size, max_len));

        for (i, enc) in encodings.iter().enumerate() {
            for (j, &id) in enc.get_ids().iter().enumerate() {
                input_ids[[i, j]] = id as i64;
            }
            for (j, &mask) in enc.get_attention_mask().iter().enumerate() {
                attention_mask[[i, j]] = mask as i64;
            }
            for (j, &type_id) in enc.get_type_ids().iter().enumerate() {
                token_type_ids[[i, j]] = type_id as i64;
            }
        }

        // Run inference.
        let mut session = self
            .session
            .lock()
            .map_err(|e| Error::custom(format!("session lock: {e}")))?;
        let outputs = session
            .run(ort::inputs![
                "input_ids" => ort::value::TensorRef::from_array_view(&input_ids)
                    .map_err(|e| Error::custom(format!("ort tensor input_ids: {e}")))?,
                "attention_mask" => ort::value::TensorRef::from_array_view(&attention_mask)
                    .map_err(|e| Error::custom(format!("ort tensor attention_mask: {e}")))?,
                "token_type_ids" => ort::value::TensorRef::from_array_view(&token_type_ids)
                    .map_err(|e| Error::custom(format!("ort tensor token_type_ids: {e}")))?,
            ])
            .map_err(|e| Error::custom(format!("ort run: {e}")))?;

        // Extract logits (shape: [batch_size, 1] for cross-encoder).
        let (shape, logits_data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| Error::custom(format!("ort extract: {e}")))?;

        // Pair each document with its score and sort descending.
        // For a cross-encoder the logits shape is [batch, 1]; we take the first
        // (and only) column value for each row.
        let cols = if shape.len() > 1 { shape[1] as usize } else { 1 };
        let mut scored: Vec<(usize, f32)> = (0..batch_size)
            .map(|i| (i, logits_data[i * cols]))
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Take top-N.
        let selected: Vec<ToolDocument> = scored
            .into_iter()
            .take(top_n)
            .map(|(idx, _score)| docs[idx].clone())
            .collect();

        Ok(selected)
    }
}

// -- Model download

fn default_cache_dir() -> Result<PathBuf> {
    let base = dirs::cache_dir()
        .or_else(dirs::home_dir)
        .ok_or_else(|| Error::custom("cannot determine cache directory"))?;
    Ok(base.join("cade").join("models").join("reranker"))
}

async fn download_model(dest: &Path) -> Result<()> {
    tracing::info!(
        "[reranker] downloading model '{}' to {}",
        DEFAULT_HF_REPO,
        dest.display()
    );

    std::fs::create_dir_all(dest)?;

    let base_url = format!(
        "https://huggingface.co/{}/resolve/main",
        DEFAULT_HF_REPO
    );

    // Download ONNX model.
    download_file(
        &format!("{base_url}/{ONNX_FILE}"),
        &dest.join("model.onnx"),
    )
    .await?;

    // Download tokenizer.
    download_file(
        &format!("{base_url}/{TOKENIZER_FILE}"),
        &dest.join("tokenizer.json"),
    )
    .await?;

    tracing::info!("[reranker] model download complete");
    Ok(())
}

async fn download_file(url: &str, dest: &Path) -> Result<()> {
    tracing::debug!("[reranker] downloading {url}");

    let resp = reqwest::get(url).await?;
    if !resp.status().is_success() {
        return Err(Error::custom(format!(
            "download failed: {} → {}",
            url,
            resp.status()
        )));
    }

    let bytes = resp.bytes().await?;
    std::fs::write(dest, &bytes)?;

    tracing::debug!(
        "[reranker] wrote {} bytes to {}",
        bytes.len(),
        dest.display()
    );
    Ok(())
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cache_dir_exists() {
        // Just check it doesn't panic.
        let dir = default_cache_dir().unwrap();
        assert!(dir.to_string_lossy().contains("cade"));
    }
}

// endregion: --- Tests
