use super::*;
use serde_json::Value;

impl ToolRuntime {
    pub(crate) async fn handle_update_memory(&self, args: &Value) -> (String, bool) {
        let label = args["label"].as_str().unwrap_or("").trim().to_string();
        let value = args["value"].as_str().unwrap_or("").to_string();
        let operation = args["operation"].as_str().unwrap_or("set");
        let description = args["description"].as_str().map(String::from);

        if label.is_empty() {
            return ("Error: 'label' is required".to_string(), true);
        }

        if operation == "delete" {
            return match self.storage.delete_memory(&self.agent_id, &label).await {
                Ok(_) => (format!("Memory block '{label}' deleted"), false),
                Err(e) => (format!("Failed to delete memory block: {e}"), true),
            };
        }

        if value.is_empty() && operation != "delete" {
            return (
                "Error: 'value' is required for set/append operations".to_string(),
                true,
            );
        }

        let final_value = if operation == "append" {
            let existing = self
                .storage
                .get_memory(&self.agent_id)
                .await
                .unwrap_or_default()
                .into_iter()
                .find(|b| b.label == label)
                .map(|b| b.value)
                .unwrap_or_default();
            if existing.is_empty() {
                value
            } else {
                format!("{existing}\n{value}")
            }
        } else {
            value
        };

        match self
            .storage
            .upsert_memory_with_limit(
                &self.agent_id,
                &label,
                &final_value,
                description.as_deref(),
                None,
            )
            .await
        {
            Ok(_) => (format!("Memory block '{label}' updated"), false),
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("exceeds character limit") {
                    let limit = parse_limit_from_error(&err_str).unwrap_or(2_000);
                    let trimmed = auto_trim_to_limit(&final_value, limit);
                    let orig = final_value.chars().count();
                    let kept = trimmed.chars().count();
                    match self
                        .storage
                        .upsert_memory_with_limit(
                            &self.agent_id,
                            &label,
                            &trimmed,
                            description.as_deref(),
                            None,
                        )
                        .await
                    {
                        Ok(_) => (
                            format!(
                                "Memory block '{label}' updated (auto-trimmed from {orig} to {kept} chars to fit the {limit}-char limit)."
                            ),
                            false,
                        ),
                        Err(e2) => (format!("Failed after auto-trim: {e2}"), true),
                    }
                } else {
                    (format!("Failed: {err_str}"), true)
                }
            }
        }
    }

    pub(crate) async fn handle_memory_apply_patch(&self, args: &Value) -> (String, bool) {
        let label = args["label"].as_str().unwrap_or("").trim().to_string();
        let patch = args["patch"].as_str().unwrap_or("").to_string();
        let description = args["description"].as_str().map(String::from);

        if label.is_empty() || patch.is_empty() {
            return ("Error: 'label' and 'patch' are required".to_string(), true);
        }

        // Get current value
        let current = self
            .storage
            .get_memory(&self.agent_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .find(|b| b.label == label)
            .map(|b| b.value)
            .unwrap_or_default();

        // Apply unified diff patch
        match apply_unified_diff(&current, &patch) {
            Ok(new_value) => {
                match self
                    .storage
                    .upsert_memory_with_limit(
                        &self.agent_id,
                        &label,
                        &new_value,
                        description.as_deref(),
                        None,
                    )
                    .await
                {
                    Ok(_) => (
                        format!("Memory block '{label}' patched successfully"),
                        false,
                    ),
                    Err(e) => (format!("Failed to save patched memory: {e}"), true),
                }
            }
            Err(e) => (format!("Patch failed: {e}"), true),
        }
    }

    pub(crate) async fn handle_store_artifact(&self, args: &Value) -> (String, bool) {
        let kind = args["kind"].as_str().unwrap_or("other");
        let content = args["content"].as_str().unwrap_or("");
        let label = args["label"].as_str().unwrap_or("");

        if content.is_empty() {
            return ("Error: 'content' is required".to_string(), true);
        }

        match self
            .storage
            .store_artifact(
                &self.agent_id,
                kind,
                "text/plain",
                Some(content),
                None,
                None,
            )
            .await
        {
            Ok(art_id) => {
                let label_str = if label.is_empty() {
                    String::new()
                } else {
                    format!(" '{label}'")
                };
                (format!("Artifact{label_str} stored. ID: {art_id}"), false)
            }
            Err(e) => (format!("Failed to store artifact: {e}"), true),
        }
    }

    pub(crate) async fn handle_update_memory_typed(&self, args: &Value) -> (String, bool) {
        let label = args["label"].as_str().unwrap_or("").trim().to_string();
        let value = args["value"].as_str().unwrap_or("").to_string();
        let memory_type = args["memory_type"].as_str().unwrap_or("generic");
        let confidence = args["confidence"].as_f64().unwrap_or(1.0).clamp(0.0, 1.0);
        let _tags: Vec<String> = args["tags"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if label.is_empty() || value.is_empty() {
            return ("Error: 'label' and 'value' are required".to_string(), true);
        }

        match self
            .storage
            .upsert_memory_with_options(
                &self.agent_id,
                &label,
                &value,
                Some("Auto-updated typed memory"),
                None,
                Some(memory_type),
                Some(confidence),
            )
            .await
        {
            Ok(_) => (
                format!(
                    "Memory block '{label}' stored as [{memory_type}] (confidence: {:.0}%)",
                    confidence * 100.0
                ),
                false,
            ),
            Err(e) => (format!("Failed to store typed memory: {e}"), true),
        }
    }

    pub(crate) async fn handle_update_memory_field(&self, args: &Value) -> (String, bool) {
        let label = args["label"].as_str().unwrap_or("").trim().to_string();
        let pointer = args["path"].as_str().unwrap_or("").to_string();
        let op_str = args["op"].as_str().unwrap_or("");
        let value = args.get("value").cloned();

        if label.is_empty() || pointer.is_empty() {
            return ("Error: 'label' and 'path' are required".to_string(), true);
        }

        let op = match cade_core::structured_patch::PatchOp::from_str_loose(op_str) {
            Some(o) => o,
            None => {
                return (
                    format!("Error: invalid op '{op_str}' — must be set, append, or remove"),
                    true,
                );
            }
        };

        // Fetch existing block
        let current = self
            .storage
            .get_memory(&self.agent_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .find(|b| b.label == label)
            .map(|b| b.value)
            .unwrap_or_default();

        if current.is_empty() {
            return (
                format!(
                    "Error: memory block '{label}' is empty or does not exist. \
                     Use update_memory(set,...) to seed it with JSON first."
                ),
                true,
            );
        }

        // Parse as JSON
        let mut root = match cade_core::structured_patch::parse_block(&current) {
            Ok(v) => v,
            Err(e) => {
                return (
                    format!("Error: {e}. Use update_memory(set,...) to seed JSON."),
                    true,
                );
            }
        };

        // Apply the patch
        if let Err(e) =
            cade_core::structured_patch::apply_pointer_patch(&mut root, &pointer, op, value)
        {
            return (format!("Patch error: {e}"), true);
        }

        // Serialize and persist
        let new_body = cade_core::structured_patch::serialize_back(&root);
        match self
            .storage
            .upsert_memory_with_limit(&self.agent_id, &label, &new_body, None, None)
            .await
        {
            Ok(_) => (
                format!("Memory block '{label}' field '{pointer}' updated ({op_str})"),
                false,
            ),
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("exceeds character limit") {
                    (format!("Error: patched block too large — {err_str}"), true)
                } else {
                    (format!("Failed to save: {err_str}"), true)
                }
            }
        }
    }

    pub(crate) async fn handle_link_memory_evidence(&self, args: &Value) -> (String, bool) {
        let label = args["label"].as_str().unwrap_or("").trim().to_string();
        let kind = args["kind"].as_str().unwrap_or("user_assertion");
        let reference = args["reference"].as_str().unwrap_or("").trim().to_string();
        let excerpt = args["excerpt"].as_str().map(String::from);

        if label.is_empty() || reference.is_empty() {
            return (
                "Error: 'label' and 'reference' are required".to_string(),
                true,
            );
        }

        match self
            .storage
            .add_memory_evidence(&self.agent_id, &label, kind, &reference, excerpt.as_deref())
            .await
        {
            Ok(_) => (
                format!("Evidence linked to '{label}': [{kind}] {reference}"),
                false,
            ),
            Err(e) => (format!("Failed to link evidence: {e}"), true),
        }
    }

    pub(crate) async fn handle_recall(&self, args: &Value) -> (String, bool) {
        let query = args["query"].as_str().unwrap_or("").trim().to_string();
        let limit = args["limit"].as_u64().unwrap_or(10) as usize;

        if query.is_empty() {
            return ("Error: 'query' is required".to_string(), true);
        }

        match self
            .storage
            .recall(&self.agent_id, &query, Some(limit))
            .await
        {
            Ok(results) => {
                if results.is_empty() {
                    return (
                        "No results found across any memory source.".to_string(),
                        false,
                    );
                }
                let mut out = format!(
                    "Found {} result(s) across all memory sources:\n\n",
                    results.len()
                );
                for (i, item) in results.iter().enumerate() {
                    let source = item["source"].as_str().unwrap_or("?");
                    let label = item["label"].as_str().unwrap_or("");
                    let snippet = item["snippet"].as_str().unwrap_or("");
                    let preview: String = snippet.chars().take(300).collect();
                    out.push_str(&format!("{}. [{}] {}: {}\n", i + 1, source, label, preview));
                }
                (out, false)
            }
            Err(e) => (format!("Recall failed: {e}"), true),
        }
    }

    pub(crate) async fn handle_answer(&self, args: &Value) -> (String, bool) {
        let question = args["question"].as_str().unwrap_or("").trim().to_string();
        let memory_type = args["memory_type"].as_str();
        let max_sources = args["max_sources"].as_u64().unwrap_or(5) as usize;

        if question.is_empty() {
            return ("Error: 'question' is required".to_string(), true);
        }

        // Fetch broader results, then filter and rank
        let limit = max_sources * 2;
        let results = match self
            .storage
            .recall(&self.agent_id, &question, Some(limit))
            .await
        {
            Ok(r) => r,
            Err(e) => return (format!("Recall failed: {e}"), true),
        };

        let filtered: Vec<&Value> = if let Some(mt) = memory_type {
            results
                .iter()
                .filter(|v| v.get("memory_type").and_then(|m| m.as_str()) == Some(mt))
                .take(max_sources)
                .collect()
        } else {
            results.iter().take(max_sources).collect()
        };

        if filtered.is_empty() {
            return (
                "No relevant memories found to answer your question.".to_string(),
                false,
            );
        }

        let mut answer =
            format!("Based on my memory, here's what I know about \"{question}\":\n\n");
        for (i, src) in filtered.iter().enumerate() {
            let source = src["source"].as_str().unwrap_or("memory");
            let label = src["label"].as_str().unwrap_or("");
            let snippet = src["snippet"].as_str().unwrap_or("");
            let preview: String = snippet.chars().take(500).collect();
            answer.push_str(&format!("{}. [{}] {}: {}\n", i + 1, source, label, preview));
        }

        (answer, false)
    }
}
