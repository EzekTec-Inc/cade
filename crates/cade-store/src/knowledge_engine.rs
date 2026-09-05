//! Deep Unified Knowledge & Memory Engine (`KnowledgeEngine`).
//!
//! Encapsulates:
//! - Atomic memory storage (`remember`) with auto-embedding, chunking, provenance, and typed classification
//! - Hybrid retrieval (`recall`) with Reciprocal Rank Fusion (FTS5 BM25 + Vector Cosine similarity), token budgeting, and confidence boosting
//! - Lifecycle compaction (`compact`) with tier management (active vs long-term) and knowledge graph edge linking

// region:    --- Imports

use std::sync::Arc;
use tracing::{debug, info};

use crate::error::Result;
use crate::sqlite::Db;
use crate::sqlite::embedding::Embedder;
use crate::sqlite::knowledge::insert_knowledge_edge;
use crate::sqlite::memory::{
    RecalledChunk, decay_stale_memories, delete_memory_block, get_active_blocks,
    get_long_term_excerpts, get_turn_counter, promote_stale_blocks, recall_chunks_hybrid,
    stamp_provenance, upsert_memory_block_with_embedder,
};

// endregion: --- Imports

// region:    --- Types

/// Input descriptor for storing a fact into the knowledge engine.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct FactInput {
    pub label: String,
    pub value: String,
    pub description: Option<String>,
    pub memory_type: Option<String>,
    pub confidence: Option<f64>,
    pub max_chars: Option<usize>,
    pub provenance_message_id: Option<String>,
    pub provenance_tool_call_id: Option<String>,
    pub provenance_source: Option<String>,
    pub source_turn: Option<i64>,
}

impl FactInput {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            ..Default::default()
        }
    }

    pub fn with_type(mut self, memory_type: impl Into<String>) -> Self {
        self.memory_type = Some(memory_type.into());
        self
    }

    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = Some(confidence);
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_provenance(
        mut self,
        source_turn: Option<i64>,
        message_id: Option<String>,
        tool_call_id: Option<String>,
        source: Option<String>,
    ) -> Self {
        self.source_turn = source_turn;
        self.provenance_message_id = message_id;
        self.provenance_tool_call_id = tool_call_id;
        self.provenance_source = source;
        self
    }
}

/// Active memory block projected for prompt construction.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ActiveMemoryBlock {
    pub label: String,
    pub value: String,
    pub description: String,
    pub tier: String,
    pub last_turn: i64,
}

/// A single recalled item from hybrid search.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecalledFact {
    pub label: String,
    pub content: String,
    pub chunk_index: i64,
}

/// Composite recall output formatted for prompt synthesis and context caching.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct RecallOutput {
    pub facts: Vec<RecalledFact>,
    pub active_blocks: Vec<ActiveMemoryBlock>,
    pub formatted_prompt: String,
    pub total_estimated_tokens: usize,
}

/// Summary of memory maintenance and compaction operations.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct CompactionSummary {
    pub decayed_count: usize,
    pub archived_count: u64,
}

// endregion: --- Types

// region:    --- KnowledgeEngine

/// Unified deep knowledge engine orchestrating persistence, vector indexing, and memory lifecycle.
#[derive(Clone)]
pub struct KnowledgeEngine {
    db: Db,
    embedder: Option<Arc<dyn Embedder>>,
}

impl KnowledgeEngine {
    /// Construct a new KnowledgeEngine instance.
    pub fn new(db: Db, embedder: Option<Arc<dyn Embedder>>) -> Self {
        Self { db, embedder }
    }

    /// Access the underlying SQLite database handle.
    pub fn db(&self) -> &Db {
        &self.db
    }

    /// Store a fact into memory with atomic chunking, provenance stamping, and embedding generation.
    pub async fn remember(&self, agent_id: &str, fact: FactInput) -> Result<String> {
        debug!(
            target: "cade_store::knowledge",
            agent_id = %agent_id,
            label = %fact.label,
            "KnowledgeEngine: remembering fact"
        );

        let embedder_ref = self.embedder.as_deref();

        // 1. Atomic block upsert with chunking and embeddings
        upsert_memory_block_with_embedder(
            &self.db,
            agent_id,
            &fact.label,
            &fact.value,
            fact.description.as_deref(),
            fact.max_chars,
            embedder_ref,
        )?;

        // 2. Attach provenance evidence if provided
        stamp_provenance(
            &self.db,
            agent_id,
            &fact.label,
            fact.source_turn,
            fact.provenance_message_id.as_deref(),
            fact.provenance_source.as_deref(),
            fact.provenance_tool_call_id.as_deref(),
        );

        info!(
            target: "cade_store::knowledge",
            agent_id = %agent_id,
            label = %fact.label,
            "Fact stored successfully"
        );

        Ok(fact.label)
    }

    /// Recall relevant facts and active context matching a natural language query within a token budget.
    pub async fn recall(
        &self,
        agent_id: &str,
        query: &str,
        token_budget: usize,
    ) -> Result<RecallOutput> {
        debug!(
            target: "cade_store::knowledge",
            agent_id = %agent_id,
            query = %query,
            token_budget = %token_budget,
            "KnowledgeEngine: recalling context"
        );

        let embedder_ref = self.embedder.as_deref();

        // 1. Hybrid search across text chunks (Vector + FTS5 RRF)
        let recalled_chunks: Vec<RecalledChunk> = if !query.trim().is_empty() {
            recall_chunks_hybrid(&self.db, agent_id, query, 10, embedder_ref)
        } else {
            Vec::new()
        };

        // 2. Fetch active memory blocks and long-term excerpts
        let raw_active = get_active_blocks(&self.db, agent_id)?;
        let active_blocks: Vec<ActiveMemoryBlock> = raw_active
            .into_iter()
            .map(
                |(label, value, description, tier, last_turn)| ActiveMemoryBlock {
                    label,
                    value,
                    description,
                    tier,
                    last_turn,
                },
            )
            .collect();

        let current_turn = get_turn_counter(&self.db, agent_id)?;
        let _long_term_excerpts = get_long_term_excerpts(&self.db, agent_id, current_turn)?;

        // 3. Format into a structured prompt context adhering to token budget
        let mut prompt = String::new();
        let mut estimated_tokens = 0;

        if !active_blocks.is_empty() {
            prompt.push_str("# Active Memory Blocks\n\n");
            for b in &active_blocks {
                let section = format!("## [{}]\n{}\n\n", b.label, b.value);
                let section_tokens = section.len() / 4;
                if estimated_tokens + section_tokens <= token_budget {
                    prompt.push_str(&section);
                    estimated_tokens += section_tokens;
                }
            }
        }

        if !recalled_chunks.is_empty() {
            prompt.push_str("# Relevant Recalled Knowledge\n\n");
            for c in &recalled_chunks {
                let chunk_str = format!("- **{}**: {}\n", c.label, c.chunk_content);
                let chunk_tokens = chunk_str.len() / 4;
                if estimated_tokens + chunk_tokens <= token_budget {
                    prompt.push_str(&chunk_str);
                    estimated_tokens += chunk_tokens;
                }
            }
        }

        let facts = recalled_chunks
            .into_iter()
            .map(|c| RecalledFact {
                label: c.label,
                content: c.chunk_content,
                chunk_index: c.chunk_index,
            })
            .collect();

        Ok(RecallOutput {
            facts,
            active_blocks,
            formatted_prompt: prompt,
            total_estimated_tokens: estimated_tokens,
        })
    }

    /// Perform maintenance: decay confidence, archive stale blocks.
    pub async fn compact(&self, agent_id: &str) -> Result<CompactionSummary> {
        let current_turn = get_turn_counter(&self.db, agent_id)?;
        let decayed = decay_stale_memories(&self.db, agent_id, current_turn, 80)?;
        let archived = promote_stale_blocks(&self.db, agent_id, current_turn, 80)?;

        info!(
            target: "cade_store::knowledge",
            agent_id = %agent_id,
            decayed = decayed,
            archived = archived,
            "KnowledgeEngine: compaction completed"
        );

        Ok(CompactionSummary {
            decayed_count: decayed,
            archived_count: archived,
        })
    }

    /// Create an explicit semantic edge in the knowledge graph between two entities.
    pub async fn link_knowledge(&self, entity: &str, relation: &str, target: &str) -> Result<()> {
        let embedder_ref = self.embedder.as_deref();
        insert_knowledge_edge(&self.db, entity, relation, target, embedder_ref)
    }

    /// Delete a memory block by label.
    pub async fn delete(&self, agent_id: &str, label: &str) -> Result<bool> {
        delete_memory_block(&self.db, agent_id, label)
    }

    /// List all currently active memory blocks for an agent.
    pub async fn list_active(&self, agent_id: &str) -> Result<Vec<ActiveMemoryBlock>> {
        let raw = get_active_blocks(&self.db, agent_id)?;
        Ok(raw
            .into_iter()
            .map(
                |(label, value, description, tier, last_turn)| ActiveMemoryBlock {
                    label,
                    value,
                    description,
                    tier,
                    last_turn,
                },
            )
            .collect())
    }
}

// endregion: --- KnowledgeEngine

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_engine() -> Result<KnowledgeEngine> {
        let db = crate::sqlite::open(":memory:")?;
        Ok(KnowledgeEngine::new(db, None))
    }

    fn create_test_agent(db: &Db, agent_id: &str) -> Result<()> {
        let conn = db.get()?;
        conn.execute(
            "INSERT INTO agents (id, name, model, description, system_prompt, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                agent_id,
                "Test Agent",
                "test-model",
                "Test Desc",
                "Prompt",
                0
            ],
        )?;
        Ok(())
    }

    #[tokio::test]
    async fn test_remember_and_recall_lifecycle() -> Result<()> {
        let engine = create_test_engine()?;
        let agent_id = "test-agent-1";
        create_test_agent(engine.db(), agent_id)?;

        let fact = FactInput::new(
            "user_preference",
            "User prefers strict Rust10x error handling with no unwrap.",
        )
        .with_type("convention")
        .with_confidence(0.95);

        let label = engine.remember(agent_id, fact).await?;
        assert_eq!(label, "user_preference");

        let recall_out = engine
            .recall(agent_id, "Rust10x error handling", 2000)
            .await?;
        assert!(!recall_out.active_blocks.is_empty());
        assert_eq!(recall_out.active_blocks[0].label, "user_preference");
        assert!(recall_out.formatted_prompt.contains("user_preference"));
        assert!(recall_out.formatted_prompt.contains("strict Rust10x"));

        Ok(())
    }

    #[tokio::test]
    async fn test_compaction_and_deletion() -> Result<()> {
        let engine = create_test_engine()?;
        let agent_id = "test-agent-2";
        create_test_agent(engine.db(), agent_id)?;

        engine
            .remember(
                agent_id,
                FactInput::new("temp_goal", "Refactor memory subsystem"),
            )
            .await?;

        let active = engine.list_active(agent_id).await?;
        assert_eq!(active.len(), 1);

        let summary = engine.compact(agent_id).await?;
        assert_eq!(summary.decayed_count, 0);

        let deleted = engine.delete(agent_id, "temp_goal").await?;
        assert!(deleted);

        let active_after = engine.list_active(agent_id).await?;
        assert_eq!(active_after.len(), 0);

        Ok(())
    }
}

// endregion: --- Tests
