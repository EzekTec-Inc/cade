use cade_ai::{CompletionRequest, LlmMessage, LlmProvider};
use std::collections::HashSet;
use std::sync::Arc;

// Constants moved from consolidation.rs to establish locality
const SESSION_SUMMARY_MAX_CHARS: usize = 8_000;
const SESSION_SUMMARY_ARCHIVED_MAX_CHARS: usize = 4_000;
const SESSION_SUMMARY_RING_CAP: usize = 8;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TouchedFiles {
    pub read: Vec<String>,
    pub modified: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RotationPlan {
    /// New blocks to upsert: key (e.g. "session_summary_1") -> value
    pub upserts: Vec<(String, String)>,
    /// Old blocks to delete from SQLite
    pub deletes: Vec<String>,
    /// Content that must be appended to the "session_index" block
    pub append_to_index: Option<String>,
    /// Content that must be archived in "archival_memory"
    pub archive_content: Option<String>,
}

#[derive(Debug, Clone)]
pub enum AccumulationResult {
    Merged(String),
    Rotated(RotationPlan),
}

pub struct SummaryAccumulator {
    llm: Arc<dyn LlmProvider>,
    compaction_model: String,
}

impl SummaryAccumulator {
    pub fn new(llm: Arc<dyn LlmProvider>, compaction_model: String) -> Self {
        Self {
            llm,
            compaction_model,
        }
    }

    /// Accumulates the new summary with the existing summary block.
    ///
    /// If the combined summary fits within `SESSION_SUMMARY_MAX_CHARS`, it combines them or
    /// runs an LLM-based merge. If the budget is exceeded, or if the LLM merge fails, it
    /// plans a FIFO ring rotation across the archived slots.
    pub async fn accumulate(
        &self,
        existing_summary: &str,
        new_summary: &str,
        touched_files: TouchedFiles,
        existing_blocks: &[(String, String)], // Slices of (label, value) from DB
    ) -> AccumulationResult {
        let clean_existing = strip_touched_files_section(existing_summary);
        let clean_new_summary = strip_touched_files_section(new_summary);

        // 1. Process and merge touched files
        let (new_read, new_mod) = (touched_files.read, touched_files.modified);
        let (mut accum_read, mut mut_mod) = parse_existing_touched_files(existing_summary);

        for r in new_read {
            accum_read.insert(r);
        }
        for m in new_mod {
            mut_mod.insert(m);
        }

        let mut accum_read_vec: Vec<String> = accum_read.into_iter().collect();
        let mut mut_mod_vec: Vec<String> = mut_mod.into_iter().collect();
        accum_read_vec.sort();
        mut_mod_vec.sort();

        let files_metadata = format_touched_files_section(&accum_read_vec, &mut_mod_vec);

        // 2. Decide: combine directly, merge with LLM, or rotate
        if clean_existing.is_empty() {
            let final_val = format!("{clean_new_summary}{files_metadata}");
            return AccumulationResult::Merged(final_val);
        }

        let combined = format!("{clean_existing}\n\n---\n\n{clean_new_summary}");
        if combined.chars().count() <= SESSION_SUMMARY_MAX_CHARS {
            let final_val = format!("{combined}{files_metadata}");
            return AccumulationResult::Merged(final_val);
        }

        // Try merging summaries using LLM
        tracing::info!(
            "SummaryAccumulator: merging existing and new summaries ({} chars total)",
            combined.chars().count()
        );

        match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.merge_session_summaries(&clean_existing, &clean_new_summary),
        )
        .await
        {
            Ok(Ok(merged)) => {
                tracing::info!(
                    "SummaryAccumulator: successfully merged summaries ({} chars)",
                    merged.chars().count()
                );
                let final_val = format!("{merged}{files_metadata}");
                AccumulationResult::Merged(final_val)
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    "SummaryAccumulator: LLM summary merge failed: {}. Falling back to ring rotation.",
                    e
                );
                let rotation_plan = self.plan_rotation(
                    existing_summary,
                    &clean_new_summary,
                    files_metadata,
                    existing_blocks,
                );
                AccumulationResult::Rotated(rotation_plan)
            }
            Err(_) => {
                tracing::warn!(
                    "SummaryAccumulator: LLM summary merge timed out. Falling back to ring rotation."
                );
                let rotation_plan = self.plan_rotation(
                    existing_summary,
                    &clean_new_summary,
                    files_metadata,
                    existing_blocks,
                );
                AccumulationResult::Rotated(rotation_plan)
            }
        }
    }

    /// Plan the FIFO ring rotation when size budget is exceeded.
    pub fn plan_rotation(
        &self,
        prev_live: &str,
        clean_new_summary: &str,
        files_metadata: String,
        existing_blocks: &[(String, String)],
    ) -> RotationPlan {
        let mut plan = RotationPlan::default();
        if prev_live.trim().is_empty() {
            // Nothing to rotate, just write slot 1.
            let capped = truncate_head_to(clean_new_summary, SESSION_SUMMARY_ARCHIVED_MAX_CHARS);
            plan.upserts.push((
                "session_summary".to_string(),
                format!("{capped}{files_metadata}"),
            ));
            return plan;
        }

        let label_for = |n: usize| format!("session_summary_{n}");

        // Step 1: evict oldest slot if occupied
        let oldest_label = label_for(SESSION_SUMMARY_RING_CAP);
        if let Some((_, val)) = existing_blocks.iter().find(|(l, _)| l == &oldest_label) {
            if !val.trim().is_empty() {
                plan.archive_content = Some(val.to_string());
                let excerpt = truncate_head_to(val, 500);
                let excerpt = excerpt.trim();
                if !excerpt.is_empty() {
                    plan.append_to_index = Some(excerpt.to_string());
                }
            }
            plan.deletes.push(oldest_label);
        }

        // Step 2: shift N -> N+1, from N=RING_CAP-1 down to 1
        for n in (1..SESSION_SUMMARY_RING_CAP).rev() {
            let src = label_for(n);
            let dst = label_for(n + 1);
            if let Some((_, val)) = existing_blocks.iter().find(|(l, _)| l == &src) {
                plan.upserts.push((dst, val.to_string()));
                plan.deletes.push(src);
            }
        }

        // Step 3: write prev_live into slot 1
        let capped = truncate_head_to(prev_live, SESSION_SUMMARY_ARCHIVED_MAX_CHARS);
        let slot1 = label_for(1);
        plan.upserts.push((slot1, capped));

        // Step 4: write new summary as the fresh session_summary block
        plan.upserts.push((
            "session_summary".to_string(),
            format!("{clean_new_summary}{files_metadata}"),
        ));

        plan
    }

    /// Asynchronously merges the summaries using the LLM provider.
    pub(crate) async fn merge_session_summaries(
        &self,
        old_summary: &str,
        new_summary: &str,
    ) -> Result<String, String> {
        let prompt = format!(
            "You are an expert context consolidation agent. Your task is to merge an older session summary with a newly generated summary of the most recent conversation turns into a single, high-density, cohesive summary.\n\n\
             CRITICAL CONSTRAINTS:\n\
             1. The combined summary must preserve all critical decisions, key file changes, error traces, and architectural goals.\n\
             2. It must be written in a high-density, professional, and concise format.\n\
             3. The final output must be strictly less than 6,500 characters so that it safely fits within CADE's active memory block buffers.\n\
             4. Do not include any intro, outro, preamble, or markdown code block wrappers (like ```markdown). Respond ONLY with the raw merged summary text.\n\n\
             OLDER SESSION SUMMARY:\n{old_summary}\n\n\
             NEW CONVERSATION SUMMARY:\n{new_summary}"
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
            max_tokens: 1500,
            reasoning_effort: None,
        };

        match self.llm.complete(&req).await {
            Ok(resp) => {
                if let Some(content) = resp.content {
                    Ok(content.trim().to_string())
                } else {
                    Err("Empty response from consolidation model".to_string())
                }
            }
            Err(e) => Err(format!("LLM completion failed: {e}")),
        }
    }
}

// ── UTILITY HELPERS ──────────────────────────────────────────────────────────

/// Parse any existing touched files from the existing summary block.
pub fn parse_existing_touched_files(summary: &str) -> (HashSet<String>, HashSet<String>) {
    let mut read = HashSet::new();
    let mut modified = HashSet::new();

    for line in summary.lines() {
        if line.starts_with("* Read: [") && line.ends_with(']') {
            let content = &line["* Read: [".len()..line.len() - 1];
            for p in content.split(',') {
                let cleaned = p.trim().to_string();
                if !cleaned.is_empty() {
                    read.insert(cleaned);
                }
            }
        } else if line.starts_with("* Modified: [") && line.ends_with(']') {
            let content = &line["* Modified: [".len()..line.len() - 1];
            for p in content.split(',') {
                let cleaned = p.trim().to_string();
                if !cleaned.is_empty() {
                    modified.insert(cleaned);
                }
            }
        }
    }

    (read, modified)
}

/// Format the touched files section to append to the summary.
pub fn format_touched_files_section(read: &[String], modified: &[String]) -> String {
    if read.is_empty() && modified.is_empty() {
        return String::new();
    }

    let mut section = String::new();
    section.push_str("\n\n### Files Checked in this Session:\n");
    if !read.is_empty() {
        section.push_str(&format!("* Read: [{}]\n", read.join(", ")));
    }
    if !modified.is_empty() {
        section.push_str(&format!("* Modified: [{}]\n", modified.join(", ")));
    }
    section
}

/// Strip the touched files section from a summary block to keep it pure for synthesis.
pub fn strip_touched_files_section(summary: &str) -> String {
    if let Some(pos) = summary.find("### Files Checked in this Session:") {
        summary[..pos].trim().to_string()
    } else {
        summary.to_string()
    }
}

/// Truncate `s` from the head so the result has at most `max_chars` chars.
pub fn truncate_head_to(s: &str, max_chars: usize) -> String {
    let total = s.chars().count();
    if total <= max_chars {
        return s.to_string();
    }
    let skip = total - max_chars;
    s.chars().skip(skip).collect()
}

/// Sanitize a line for inclusion in `session_index`: strip newlines,
/// collapse internal whitespace, cap at 200 chars.
pub fn sanitize_index_line(s: &str) -> String {
    let collapsed: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(200).collect()
}
