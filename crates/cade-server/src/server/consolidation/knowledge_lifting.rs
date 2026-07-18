use cade_ai::{CompletionRequest, LlmMessage, LlmProvider};
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ExtractedFact {
    pub label: String,
    pub memory_type: String,
    pub value: String,
    pub confidence: f64,
}

pub struct KnowledgeLiftingEngine {
    llm: Arc<dyn LlmProvider>,
    compaction_model: String,
}

impl KnowledgeLiftingEngine {
    pub fn new(llm: Arc<dyn LlmProvider>, compaction_model: String) -> Self {
        Self {
            llm,
            compaction_model,
        }
    }

    /// Extract durable facts from raw text using the compaction model.
    ///
    /// This is completely decoupled from any database state, allowing pure,
    /// in-memory unit testing.
    pub async fn extract_from_text(&self, text: &str) -> Result<Vec<ExtractedFact>, String> {
        if text.trim().is_empty() {
            return Ok(vec![]);
        }

        let prompt = format!(
            "You are a memory extraction sub-agent. Based on this consolidation summary, \
             extract durable facts, decisions, and constraints into a JSON array.\n\
             \n\
             Each object in the array must have:\n\
             - \"label\": a short snake_case identifier (e.g. \"project_convention_auth\", \"decision_db_sqlite\")\n\
             - \"memory_type\": exactly one of [\"decision\", \"convention\", \"project_fact\", \"constraint\"]\n\
             - \"value\": a concise, factual description\n\
             - \"confidence\": a number between 0.0 and 1.0 (default to 1.0)\n\
             \n\
             Only extract durable knowledge that will be useful across sessions. Do NOT extract transient state.\n\
             If there are no new durable facts, return exactly: []\n\
             \n\
             SUMMARY:\n\
             {text}"
        );

        let req = CompletionRequest {
            model: self.compaction_model.clone(),
            messages: vec![LlmMessage {
                role: "user".to_string(),
                content: prompt,
                tool_call_id: None,
                tool_calls: None,
                images: None, cache_control: None,
            }],
            tools: vec![],
            max_tokens: 1000,
            reasoning_effort: None,
        };

        let response_text = match self.llm.complete(&req).await {
            Ok(resp) => resp.content.unwrap_or_default().trim().to_string(),
            Err(e) => return Err(format!("LLM complete failed: {e}")),
        };

        let clean_json = response_text
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        if clean_json.is_empty() || clean_json == "[]" {
            return Ok(vec![]);
        }

        let raw_facts: Vec<Value> = serde_json::from_str(clean_json)
            .map_err(|e| format!("Failed to parse LLM JSON: {e}"))?;

        let mut validated_facts = Vec::new();
        for fact in raw_facts {
            let label = fact["label"].as_str().unwrap_or("").to_string();
            let memory_type = fact["memory_type"]
                .as_str()
                .unwrap_or("generic")
                .to_string();
            let value = fact["value"].as_str().unwrap_or("").to_string();
            let confidence = fact["confidence"].as_f64().unwrap_or(1.0);

            if label.is_empty() || value.is_empty() {
                continue;
            }

            validated_facts.push(ExtractedFact {
                label,
                memory_type,
                value,
                confidence,
            });
        }

        Ok(validated_facts)
    }
}
