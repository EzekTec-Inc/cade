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
            return match self.client.delete_memory(&self.agent_id, &label).await {
                Ok(_) => (format!("Memory block '{label}' deleted"), false),
                Err(e) => (format!("Failed to delete memory block: {e}"), true),
            };
        }

        if value.is_empty() && operation != "delete" {
            return ("Error: 'value' is required for set/append operations".to_string(), true);
        }

        let final_value = if operation == "append" {
            let existing = self
                .client
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
            .client
            .upsert_memory(&self.agent_id, &label, &final_value, description.as_deref())
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
                        .client
                        .upsert_memory(&self.agent_id, &label, &trimmed, description.as_deref())
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
            .client
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
                    .client
                    .upsert_memory(&self.agent_id, &label, &new_value, description.as_deref())
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
            .client
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
        let tags: Vec<String> = args["tags"]
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
            .client
            .upsert_typed_memory(
                &self.agent_id,
                &label,
                &value,
                memory_type,
                confidence,
                &tags,
                None,
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
            .client
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

}
