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
        let lock = self.interactions.lock().unwrap();
        lock.iter()
            .find(|i| i.url == url && i.method == method && i.request_body == body)
            .cloned()
    }

    pub fn record_interaction(&self, interaction: HttpInteraction) -> Result<()> {
        if self.mode != VcrMode::Record {
            return Ok(());
        }
        let mut lock = self.interactions.lock().unwrap();
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
