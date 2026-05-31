//! VCR Cassette mock testing system.
//!
//! Provides file-backed recording and replaying of HTTP interactions
//! to enable deterministic, offline, and cost-effective unit/integration tests.

use crate::Result;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HttpInteraction {
    pub url: String,
    pub method: String,
    pub request_body: String,
    pub response_status: u16,
    pub response_body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcrMode {
    Record,
    Replay,
}

pub struct VcrCassette {
    path: PathBuf,
    mode: VcrMode,
    interactions: Arc<Mutex<Vec<HttpInteraction>>>,
}

/// Redact standard API keys, bearer tokens, and secrets from JSON or raw text.
pub fn redact_secrets(text: &str) -> String {
    let mut redacted = text.to_string();

    // 1. Redact Bearer tokens safely by tracking search position
    let mut search_pos = 0;
    while let Some(pos) = redacted[search_pos..].to_lowercase().find("bearer ") {
        let actual_pos = search_pos + pos;
        let start = actual_pos + "bearer ".len();
        let end_offset = redacted[start..]
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\\' || c == '\'' || c == '}')
            .unwrap_or(redacted[start..].len());
        let end = start + end_offset;
        redacted.replace_range(start..end, "[REDACTED_BEARER_TOKEN]");
        search_pos = start + "[REDACTED_BEARER_TOKEN]".len();
    }

    // 2. Redact Anthropic keys safely
    let mut search_pos = 0;
    while let Some(pos) = redacted[search_pos..].find("sk-ant-") {
        let actual_pos = search_pos + pos;
        let start = actual_pos;
        let end_offset = redacted[start..]
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\\' || c == '\'' || c == '}' || c == '&')
            .unwrap_or(redacted[start..].len());
        let end = start + end_offset;
        redacted.replace_range(start..end, "[REDACTED_ANTHROPIC_KEY]");
        search_pos = start + "[REDACTED_ANTHROPIC_KEY]".len();
    }

    // 3. Redact OpenAI keys safely
    let mut search_pos = 0;
    while let Some(pos) = redacted[search_pos..].find("sk-") {
        let actual_pos = search_pos + pos;
        if redacted[actual_pos..].starts_with("sk-ant-") {
            search_pos = actual_pos + "sk-ant-".len();
            continue;
        }
        let start = actual_pos;
        let end_offset = redacted[start..]
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\\' || c == '\'' || c == '}' || c == '&')
            .unwrap_or(redacted[start..].len());
        let end = start + end_offset;
        redacted.replace_range(start..end, "[REDACTED_OPENAI_KEY]");
        search_pos = start + "[REDACTED_OPENAI_KEY]".len();
    }

    // 4. Redact Google Gemini keys safely
    let mut search_pos = 0;
    while let Some(pos) = redacted[search_pos..].find("AIzaSy") {
        let actual_pos = search_pos + pos;
        let start = actual_pos;
        let end_offset = redacted[start..]
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\\' || c == '\'' || c == '}' || c == '&')
            .unwrap_or(redacted[start..].len());
        let end = start + end_offset;
        redacted.replace_range(start..end, "[REDACTED_GEMINI_KEY]");
        search_pos = start + "[REDACTED_GEMINI_KEY]".len();
    }

    redacted
}

impl VcrCassette {
    pub fn new(path: PathBuf, mode: VcrMode) -> Result<Self> {
        let interactions = if mode == VcrMode::Replay {
            let mut file = File::open(&path)
                .map_err(|e| crate::Error::custom(format!("Failed to open cassette file: {:?}", e)))?;
            let mut content = String::new();
            file.read_to_string(&mut content)
                .map_err(|e| crate::Error::custom(format!("Failed to read cassette file: {:?}", e)))?;
            serde_json::from_str(&content)
                .map_err(|e| crate::Error::custom(format!("Failed to deserialize cassette interactions: {:?}", e)))?
        } else {
            Vec::new()
        };

        Ok(Self {
            path,
            mode,
            interactions: Arc::new(Mutex::new(interactions)),
        })
    }

    pub fn match_response(&self, url: &str, method: &str, body: &str) -> Option<HttpInteraction> {
        let lock = self.interactions.lock().unwrap_or_else(|e| e.into_inner());
        lock.iter()
            .find(|i| i.url == url && i.method == method && i.request_body == body)
            .cloned()
    }

    pub fn record_interaction(&self, mut interaction: HttpInteraction) -> Result<()> {
        if self.mode != VcrMode::Record {
            return Ok(());
        }

        // Sanitize and redact secrets before persisting
        interaction.request_body = redact_secrets(&interaction.request_body);
        interaction.response_body = redact_secrets(&interaction.response_body);
        interaction.url = redact_secrets(&interaction.url);

        let mut lock = self.interactions.lock().unwrap_or_else(|e| e.into_inner());
        lock.push(interaction);

        let content = serde_json::to_string_pretty(&*lock)
            .map_err(|e| crate::Error::custom(format!("Failed to serialize interactions: {:?}", e)))?;
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.path)
            .map_err(|e| crate::Error::custom(format!("Failed to open cassette for writing: {:?}", e)))?;
        file.write_all(content.as_bytes())
            .map_err(|e| crate::Error::custom(format!("Failed to write cassette data: {:?}", e)))?;
        Ok(())
    }
}
