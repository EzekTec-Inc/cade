use super::*;

impl HttpTransport {
    // -- Tools

    pub async fn create_tool(&self, req: CreateToolRequest) -> Result<ToolDef> {
        let resp = self
            .client
            .post(self.url("/tools"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&req)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(crate::Error::custom(format!(
                "create_tool failed {status}: {body}"
            )));
        }
        Ok(resp.json::<ToolDef>().await?)
    }

    // -- Raw HTTP helpers for extension tools

    /// GET /v1{path} and return parsed JSON.
    pub async fn raw_get(&self, path: &str) -> Result<serde_json::Value> {
        let resp = self
            .client
            .get(self.url(path))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "GET {path} failed {}",
                resp.status()
            )));
        }
        Ok(resp.json().await?)
    }

    /// POST /v1{path} with JSON body and return parsed JSON.
    pub async fn raw_post(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let resp = self
            .client
            .post(self.url(path))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(body)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "POST {path} failed {}",
                resp.status()
            )));
        }
        Ok(resp.json().await?)
    }

    pub async fn list_tools(&self) -> Result<Vec<ToolDef>> {
        let resp = self
            .client
            .get(self.url("/tools"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "list_tools failed {}",
                resp.status()
            )));
        }
        Ok(resp.json::<Vec<ToolDef>>().await?)
    }

    // -- Checkpoints

    /// Create a checkpoint for the agent, optionally with a label and git info.
    pub async fn create_checkpoint(
        &self,
        agent_id: &str,
        label: Option<&str>,
        description: Option<&str>,
        conversation_id: Option<&str>,
        git_commit_hash: Option<&str>,
    ) -> Result<String> {
        let mut body = serde_json::json!({});
        if let Some(l) = label {
            body["label"] = l.into();
        }
        if let Some(d) = description {
            body["description"] = d.into();
        }
        if let Some(c) = conversation_id {
            body["conversation_id"] = c.into();
        }
        if let Some(h) = git_commit_hash {
            body["git_commit_hash"] = h.into();
        }

        let resp = self
            .client
            .post(self.url(&format!("/agents/{agent_id}/checkpoints")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(crate::Error::custom(format!(
                "create_checkpoint failed: {txt}"
            )));
        }
        let v: serde_json::Value = resp.json().await?;
        Ok(v["id"].as_str().unwrap_or("").to_string())
    }

    /// List checkpoints for an agent.
    pub async fn list_checkpoints(&self, agent_id: &str) -> Result<Vec<serde_json::Value>> {
        let resp = self
            .client
            .get(self.url(&format!("/agents/{agent_id}/checkpoints")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "list_checkpoints failed {}",
                resp.status()
            )));
        }
        Ok(resp.json().await?)
    }

    /// Get a specific checkpoint.
    pub async fn get_checkpoint(&self, agent_id: &str, cp_id: &str) -> Result<serde_json::Value> {
        let resp = self
            .client
            .get(self.url(&format!("/agents/{agent_id}/checkpoints/{cp_id}")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "get_checkpoint failed {}",
                resp.status()
            )));
        }
        Ok(resp.json().await?)
    }

    /// Restore to a checkpoint (server-side marker only; git restore handled client-side).
    pub async fn restore_checkpoint(&self, agent_id: &str, cp_id: &str) -> Result<()> {
        let resp = self
            .client
            .post(self.url(&format!("/agents/{agent_id}/checkpoints/{cp_id}/restore")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({}))
            .send()
            .await?;
        if !resp.status().is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(crate::Error::custom(format!(
                "restore_checkpoint failed: {txt}"
            )));
        }
        Ok(())
    }

    /// Delete a checkpoint.
    pub async fn delete_checkpoint(&self, agent_id: &str, cp_id: &str) -> Result<()> {
        let resp = self
            .client
            .delete(self.url(&format!("/agents/{agent_id}/checkpoints/{cp_id}")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(crate::Error::custom(format!(
                "delete_checkpoint failed: {txt}"
            )));
        }
        Ok(())
    }

    // -- Artifacts

    /// Store an artifact (screenshot, diff, log, test report, etc.).
    pub async fn store_artifact(
        &self,
        agent_id: &str,
        kind: &str,
        content_type: &str,
        data_text: Option<&str>,
        run_id: Option<&str>,
        tool_call_id: Option<&str>,
    ) -> Result<String> {
        let mut body = serde_json::json!({ "kind": kind, "content_type": content_type });
        if let Some(d) = data_text {
            body["data_text"] = d.into();
        }
        if let Some(r) = run_id {
            body["run_id"] = r.into();
        }
        if let Some(t) = tool_call_id {
            body["tool_call_id"] = t.into();
        }

        let resp = self
            .client
            .post(self.url(&format!("/agents/{agent_id}/artifacts")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(crate::Error::custom(format!(
                "store_artifact failed: {txt}"
            )));
        }
        let v: serde_json::Value = resp.json().await?;
        Ok(v["id"].as_str().unwrap_or("").to_string())
    }

    /// List artifacts for an agent.
    pub async fn list_artifacts(&self, agent_id: &str) -> Result<Vec<serde_json::Value>> {
        let resp = self
            .client
            .get(self.url(&format!("/agents/{agent_id}/artifacts")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "list_artifacts failed {}",
                resp.status()
            )));
        }
        Ok(resp.json().await?)
    }

    // -- Evals

    /// Create an eval task.
    pub async fn create_eval_task(
        &self,
        name: &str,
        prompt: &str,
        description: Option<&str>,
    ) -> Result<String> {
        let mut body = serde_json::json!({ "name": name, "prompt": prompt });
        if let Some(d) = description {
            body["description"] = d.into();
        }
        let resp = self
            .client
            .post(self.url("/evals/tasks"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(crate::Error::custom(format!(
                "create_eval_task failed: {txt}"
            )));
        }
        let v: serde_json::Value = resp.json().await?;
        Ok(v["id"].as_str().unwrap_or("").to_string())
    }

    /// List eval tasks.
    pub async fn list_eval_tasks(&self) -> Result<Vec<serde_json::Value>> {
        let resp = self
            .client
            .get(self.url("/evals/tasks"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "list_eval_tasks failed {}",
                resp.status()
            )));
        }
        Ok(resp.json().await?)
    }

    /// Create an eval run (queue a task for execution).
    pub async fn create_eval_run(
        &self,
        task_id: &str,
        agent_id: Option<&str>,
        model: Option<&str>,
    ) -> Result<String> {
        let mut body = serde_json::json!({ "task_id": task_id });
        if let Some(a) = agent_id {
            body["agent_id"] = a.into();
        }
        if let Some(m) = model {
            body["model"] = m.into();
        }
        let resp = self
            .client
            .post(self.url("/evals/runs"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(crate::Error::custom(format!(
                "create_eval_run failed: {txt}"
            )));
        }
        let v: serde_json::Value = resp.json().await?;
        Ok(v["id"].as_str().unwrap_or("").to_string())
    }

    /// List eval runs.
    pub async fn list_eval_runs(&self) -> Result<Vec<serde_json::Value>> {
        let resp = self
            .client
            .get(self.url("/evals/runs"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "list_eval_runs failed {}",
                resp.status()
            )));
        }
        Ok(resp.json().await?)
    }

    /// Get an eval run by ID.
    pub async fn get_eval_run(&self, run_id: &str) -> Result<serde_json::Value> {
        let resp = self
            .client
            .get(self.url(&format!("/evals/runs/{run_id}")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "get_eval_run failed {}",
                resp.status()
            )));
        }
        Ok(resp.json().await?)
    }

    // -- Tool execution logging

    /// Log a single tool execution.  Fire-and-forget — errors are silently discarded.
    pub fn log_tool_execution_spawn(
        &self,
        agent_id: String,
        tool_name: String,
        arguments_json: String,
        output: String,
        is_error: bool,
        duration_ms: u64,
    ) {
        let client = self.clone();
        tokio::spawn(async move {
            let body = serde_json::json!({
                "tool_name":     tool_name,
                "arguments_json": arguments_json,
                "output":        output,
                "is_error":      is_error,
                "duration_ms":   duration_ms,
            });
            let _ = client
                .client
                .post(client.url(&format!("/agents/{agent_id}/tool_executions")))
                .header("Authorization", format!("Bearer {}", client.api_key))
                .json(&body)
                .send()
                .await;
        });
    }

    // -- Typed memory / provenance

    /// Update a memory block with a specific type and optional confidence score.
    pub async fn upsert_typed_memory(
        &self,
        agent_id: &str,
        label: &str,
        value: &str,
        memory_type: &str,
        confidence: f64,
        tags: &[String],
        description: Option<&str>,
    ) -> Result<()> {
        let tags_json = serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_string());
        let mut body = serde_json::json!({
            "value":        value,
            "memory_type":  memory_type,
            "confidence":   confidence,
            "tags_json":    tags_json,
        });
        if let Some(d) = description {
            body["description"] = d.into();
        }

        let resp = self
            .client
            .put(self.url(&format!("/agents/{agent_id}/memory/{label}")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(crate::Error::custom(format!(
                "upsert_typed_memory failed: {txt}"
            )));
        }
        Ok(())
    }

    /// Add an evidence entry to a memory block.
    pub async fn add_memory_evidence(
        &self,
        agent_id: &str,
        label: &str,
        kind: &str,
        reference: &str,
        excerpt: Option<&str>,
    ) -> Result<()> {
        let mut body = serde_json::json!({ "kind": kind, "reference": reference });
        if let Some(e) = excerpt {
            body["excerpt"] = e.into();
        }

        let resp = self
            .client
            .post(self.url(&format!("/agents/{agent_id}/memory/{label}/evidence")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(crate::Error::custom(format!(
                "add_memory_evidence failed: {txt}"
            )));
        }
        Ok(())
    }

    /// Get evidence entries for a memory block.
    pub async fn get_memory_evidence(
        &self,
        agent_id: &str,
        label: &str,
    ) -> Result<Vec<serde_json::Value>> {
        let resp = self
            .client
            .get(self.url(&format!("/agents/{agent_id}/memory/{label}/evidence")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "get_memory_evidence failed {}",
                resp.status()
            )));
        }
        Ok(resp.json().await?)
    }

    /// Get a human-readable provenance summary ("why") for a memory block.
    pub async fn get_memory_why(&self, agent_id: &str, label: &str) -> Result<String> {
        let resp = self
            .client
            .get(self.url(&format!("/agents/{agent_id}/memory/{label}/why")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "get_memory_why failed {}",
                resp.status()
            )));
        }
        let v: serde_json::Value = resp.json().await?;
        Ok(v["summary"]
            .as_str()
            .unwrap_or("No provenance available.")
            .to_string())
    }

    // -- Reflection

    /// Trigger a manual `/compact` consolidation pass.  Synchronously
    /// invokes the same `consolidate_agent` flow used by the Sleeptime
    /// background task and the P1-3 recovery loop.
    ///
    /// Returns the size (chars) of the resulting `session_summary` block,
    /// suitable for surfacing in a toast to the user.
    pub async fn compact(&self, agent_id: &str, conversation_id: Option<&str>) -> Result<usize> {
        let mut url = self.url(&format!("/agents/{agent_id}/compact"));
        if let Some(c) = conversation_id {
            url.push_str(&format!("?conversation_id={c}"));
        }
        let resp = self
            .client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({}))
            .send()
            .await?;
        if !resp.status().is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(crate::Error::custom(format!("compact failed: {txt}")));
        }
        let v: serde_json::Value = resp.json().await?;
        Ok(v["session_summary_chars"].as_u64().unwrap_or(0) as usize)
    }

    /// Trigger a reflection pass over recent conversation history.
    pub async fn trigger_reflect(&self, agent_id: &str, focus: Option<&str>) -> Result<String> {
        let mut body = serde_json::json!({ "trigger": "manual" });
        if let Some(f) = focus {
            body["focus"] = f.into();
        }

        let resp = self
            .client
            .post(self.url(&format!("/agents/{agent_id}/reflect")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(crate::Error::custom(format!("reflect failed: {txt}")));
        }
        let v: serde_json::Value = resp.json().await?;
        Ok(v["summary"]
            .as_str()
            .unwrap_or("Reflection complete.")
            .to_string())
    }

    /// Get the reflection log for an agent.
    pub async fn list_reflection_log(&self, agent_id: &str) -> Result<Vec<serde_json::Value>> {
        let resp = self
            .client
            .get(self.url(&format!("/agents/{agent_id}/reflection")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "list_reflection_log failed {}",
                resp.status()
            )));
        }
        Ok(resp.json().await?)
    }
}
