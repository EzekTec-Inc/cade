use cade_ai::{CompletionRequest, LlmMessage, LlmProvider};
use std::collections::HashSet;
use std::sync::Arc;

pub use cade_store::sqlite::consolidation::SummaryPlan as RotationPlan;
use cade_store::sqlite::consolidation::{ARCHIVED_CAP, LIVE_CAP, RING_CAP};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TouchedFiles {
    pub read: Vec<String>,
    pub modified: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum AccumulationResult {
    Merged(String),
    Rotated(RotationPlan),
}

impl AccumulationResult {
    pub fn into_plan(self) -> (RotationPlan, bool) {
        match self {
            Self::Merged(value) => (
                RotationPlan {
                    upserts: vec![("session_summary".to_string(), value)],
                    ..Default::default()
                },
                false,
            ),
            Self::Rotated(plan) => (plan, true),
        }
    }
}

/// Pure rotation planning and LLM synthesis; persistence belongs to the store's
/// captured-snapshot commit, so there is only one production rotation writer.
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

    pub async fn accumulate(
        &self,
        existing_summary: &str,
        new_summary: &str,
        touched_files: TouchedFiles,
        existing_blocks: &[(String, String)],
    ) -> AccumulationResult {
        let clean_existing = strip_touched_files_section(existing_summary);
        let clean_new = strip_touched_files_section(new_summary);
        let (mut read, mut modified) = parse_existing_touched_files(existing_summary);
        read.extend(touched_files.read);
        modified.extend(touched_files.modified);
        let mut read: Vec<_> = read.into_iter().collect();
        let mut modified: Vec<_> = modified.into_iter().collect();
        read.sort();
        modified.sort();
        let metadata = format_touched_files_section(&read, &modified);
        if clean_existing.trim().is_empty() {
            return AccumulationResult::Merged(fit_live(&clean_new, &metadata));
        }
        let combined = format!("{clean_existing}\n\n---\n\n{clean_new}");
        if combined.chars().count() + metadata.chars().count() <= LIVE_CAP {
            return AccumulationResult::Merged(format!("{combined}{metadata}"));
        }
        match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.merge_session_summaries(&clean_existing, &clean_new),
        )
        .await
        {
            Ok(Ok(merged))
                if !merged.trim().is_empty()
                    && merged.chars().count() + metadata.chars().count() <= LIVE_CAP =>
            {
                AccumulationResult::Merged(format!("{merged}{metadata}"))
            }
            // An empty or oversized 'successful' merge is a failed synthesis,
            // just like a timeout. Preserve the previous summary in the ring.
            _ => AccumulationResult::Rotated(self.plan_rotation(
                existing_summary,
                &clean_new,
                metadata,
                existing_blocks,
            )),
        }
    }

    pub fn plan_rotation(
        &self,
        prev_live: &str,
        clean_new_summary: &str,
        files_metadata: String,
        existing_blocks: &[(String, String)],
    ) -> RotationPlan {
        let mut plan = RotationPlan::default();
        if !prev_live.trim().is_empty() {
            let oldest = format!("session_summary_{RING_CAP}");
            if let Some((_, value)) = existing_blocks.iter().find(|(l, _)| l == &oldest) {
                if !value.trim().is_empty() {
                    plan.archive_content = Some(value.clone());
                    plan.append_to_index = Some(truncate_head_to(value, 500).trim().to_string());
                }
                plan.deletes.push(oldest);
            }
            for n in (1..RING_CAP).rev() {
                let src = format!("session_summary_{n}");
                if let Some((_, value)) = existing_blocks.iter().find(|(l, _)| l == &src) {
                    plan.upserts.push((
                        format!("session_summary_{}", n + 1),
                        truncate_head_to(value, ARCHIVED_CAP),
                    ));
                    plan.deletes.push(src);
                }
            }
            plan.upserts.push((
                "session_summary_1".to_string(),
                truncate_head_to(prev_live, ARCHIVED_CAP),
            ));
        }
        plan.upserts.push((
            "session_summary".to_string(),
            fit_live(clean_new_summary, &files_metadata),
        ));
        plan
    }

    pub(crate) async fn merge_session_summaries(
        &self,
        old_summary: &str,
        new_summary: &str,
    ) -> Result<String, String> {
        let req = CompletionRequest {
            model: self.compaction_model.clone(),
            messages: vec![LlmMessage {
                role: "user".to_string(),
                content: format!(
                    "Merge the older and new coding session summaries into a single high-density note. \
                     Preserve critical decisions, exact file paths, errors, goals and rejected alternatives. \
                     Use strictly fewer than 6,500 characters. Return only the raw summary, no preamble or code fences.\n\n\
                     OLDER SESSION SUMMARY:\n{old_summary}\n\nNEW CONVERSATION SUMMARY:\n{new_summary}"
                ),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            }],
            tools: vec![],
            max_tokens: 1500,
            reasoning_effort: None,
        };
        self.llm
            .complete(&req)
            .await
            .map_err(|e| e.to_string())?
            .content
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string())
            .ok_or_else(|| "Empty response from consolidation model".to_string())
    }
}

fn fit_live(summary: &str, metadata: &str) -> String {
    // Reserve for deterministic paths, but do not let an unbounded path list
    // erase the synthesized note. The full source remains in archival/history.
    let metadata: String = metadata.chars().take(LIVE_CAP / 2).collect();
    let summary: String = summary
        .chars()
        .take(LIVE_CAP - metadata.chars().count())
        .collect();
    format!("{summary}{metadata}")
}

pub fn parse_existing_touched_files(summary: &str) -> (HashSet<String>, HashSet<String>) {
    let mut read = HashSet::new();
    let mut modified = HashSet::new();
    for line in summary.lines() {
        let (prefix, target) = if line.starts_with("* Read: [") {
            ("* Read: [", &mut read)
        } else if line.starts_with("* Modified: [") {
            ("* Modified: [", &mut modified)
        } else {
            continue;
        };
        if let Some(content) = line.strip_prefix(prefix).and_then(|s| s.strip_suffix(']')) {
            target.extend(
                content
                    .split(',')
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(str::to_string),
            );
        }
    }
    (read, modified)
}

pub fn format_touched_files_section(read: &[String], modified: &[String]) -> String {
    if read.is_empty() && modified.is_empty() {
        return String::new();
    }
    let mut section = "\n\n### Files Checked in this Session:\n".to_string();
    if !read.is_empty() {
        section.push_str(&format!("* Read: [{}]\n", read.join(", ")));
    }
    if !modified.is_empty() {
        section.push_str(&format!("* Modified: [{}]\n", modified.join(", ")));
    }
    section
}

pub fn strip_touched_files_section(summary: &str) -> String {
    summary
        .find("### Files Checked in this Session:")
        .map_or_else(|| summary.to_string(), |p| summary[..p].trim().to_string())
}

pub fn truncate_head_to(s: &str, max_chars: usize) -> String {
    s.chars()
        .skip(s.chars().count().saturating_sub(max_chars))
        .collect()
}

pub fn sanitize_index_line(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(200)
        .collect()
}
