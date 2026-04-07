use super::*;

impl HttpTransport {
    // -- Memory

    /// Fetch all memory blocks for an agent.
    pub async fn get_memory(&self, agent_id: &str) -> Result<Vec<MemoryBlock>> {
        let resp = self
            .client
            .get(self.url(&format!("/agents/{agent_id}/memory")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "get_memory failed {}",
                resp.status()
            )));
        }
        let body: Value = resp.json().await?;
        let blocks = body["blocks"].as_array().cloned().unwrap_or_default();
        Ok(blocks
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect())
    }

    /// Delete a single memory block.
    pub async fn delete_memory(&self, agent_id: &str, label: &str) -> Result<()> {
        let resp = self
            .client
            .delete(self.url(&format!("/agents/{agent_id}/memory/{label}")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            return Err(crate::Error::custom(format!(
                "delete_memory failed {}",
                resp.status()
            )));
        }
        Ok(())
    }

    /// Search memory blocks by label or value text.
    /// Returns Vec of (label, value, snippet).
    pub async fn search_memory(&self, agent_id: &str, query: &str) -> Result<Vec<Value>> {
        let resp = self
            .client
            .get(self.url(&format!("/agents/{agent_id}/memory")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .query(&[("q", query)])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "search_memory failed {}",
                resp.status()
            )));
        }
        let body: Value = resp.json().await?;
        Ok(body["blocks"].as_array().cloned().unwrap_or_default())
    }

    /// Upsert a single memory block.
    pub async fn upsert_memory(
        &self,
        agent_id: &str,
        label: &str,
        value: &str,
        description: Option<&str>,
    ) -> Result<()> {
        self.upsert_memory_with_limit(agent_id, label, value, description, None)
            .await
    }

    pub async fn upsert_memory_with_limit(
        &self,
        agent_id: &str,
        label: &str,
        value: &str,
        description: Option<&str>,
        max_chars: Option<usize>,
    ) -> Result<()> {
        self.upsert_memory_with_options(agent_id, label, value, description, max_chars, "set")
            .await
    }

    pub async fn append_memory_with_limit(
        &self,
        agent_id: &str,
        label: &str,
        value: &str,
        description: Option<&str>,
        max_chars: Option<usize>,
    ) -> Result<()> {
        self.upsert_memory_with_options(agent_id, label, value, description, max_chars, "append")
            .await
    }

    pub async fn upsert_memory_with_options(
        &self,
        agent_id: &str,
        label: &str,
        value: &str,
        description: Option<&str>,
        max_chars: Option<usize>,
        operation: &str,
    ) -> Result<()> {
        let mut body = json!({ "value": value, "operation": operation });
        if let Some(desc) = description {
            body["description"] = json!(desc);
        }
        if let Some(n) = max_chars {
            body["max_chars"] = json!(n);
        }
        let resp = self
            .client
            .put(self.url(&format!("/agents/{agent_id}/memory/{label}")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "upsert_memory failed {}",
                resp.status()
            )));
        }
        Ok(())
    }

    /// List the last `limit` revisions of a memory block.
    pub async fn list_memory_history(
        &self,
        agent_id: &str,
        label: &str,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>> {
        let resp = self
            .client
            .get(self.url(&format!(
                "/agents/{agent_id}/memory/{label}/history?limit={limit}"
            )))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "list_memory_history failed {}",
                resp.status()
            )));
        }
        let body: serde_json::Value = resp.json().await?;
        Ok(body.as_array().cloned().unwrap_or_default())
    }

    /// Insert into Archival Memory.
    pub async fn insert_archival_memory(
        &self,
        agent_id: &str,
        content: &str,
        tags: &[String],
    ) -> Result<String> {
        let body = json!({ "content": content, "tags": tags });
        let resp = self
            .client
            .post(self.url(&format!("/agents/{agent_id}/archival")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "insert_archival_memory failed {}",
                resp.status()
            )));
        }
        let data: Value = resp.json().await?;
        Ok(data["id"].as_str().unwrap_or_default().to_string())
    }

    /// Search Archival Memory.
    pub async fn search_archival_memory(
        &self,
        agent_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<Value>> {
        let resp = self
            .client
            .get(self.url(&format!("/agents/{agent_id}/archival/search")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .query(&[("q", query), ("limit", &limit.to_string())])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "search_archival_memory failed {}",
                resp.status()
            )));
        }
        let body: Value = resp.json().await?;
        Ok(body["results"].as_array().cloned().unwrap_or_default())
    }

    /// Restore a memory block to a specific history revision.
    pub async fn restore_memory(&self, agent_id: &str, label: &str, rev_id: &str) -> Result<()> {
        let resp = self
            .client
            .put(self.url(&format!(
                "/agents/{agent_id}/memory/{label}/restore/{rev_id}"
            )))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::Value::Null)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "restore_memory failed {}",
                resp.status()
            )));
        }
        Ok(())
    }

    // -- Memory tier management

    /// Set the tier of a memory block ('short' | 'long' | 'pinned').
    pub async fn set_memory_tier(&self, agent_id: &str, label: &str, tier: &str) -> Result<()> {
        let resp = self
            .client
            .put(self.url(&format!("/agents/{agent_id}/memory/{label}/tier")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&json!({ "tier": tier }))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "set_memory_tier failed {}",
                resp.status()
            )));
        }
        Ok(())
    }

    /// Pin a memory block (always injected, never aged out).
    pub async fn pin_memory(&self, agent_id: &str, label: &str) -> Result<()> {
        self.set_memory_tier(agent_id, label, "pinned").await
    }

    /// Demote a memory block to long-term (archived, index-only injection).
    pub async fn demote_memory(&self, agent_id: &str, label: &str) -> Result<()> {
        self.set_memory_tier(agent_id, label, "long").await
    }

    /// Promote an archived long-term block back to short-term (reactivate).
    pub async fn promote_memory(&self, agent_id: &str, label: &str) -> Result<()> {
        self.set_memory_tier(agent_id, label, "short").await
    }

    // -- Context management

    /// Fetch real server-side context-window stats for an agent.
    ///
    /// Returns the same budget arithmetic used by `build_context`: turns
    /// included / omitted, chars used, budget, memory size, and whether a
    /// Sleeptime consolidation is pending.
    pub async fn get_context_stats(
        &self,
        agent_id: &str,
        conversation_id: Option<&str>,
    ) -> Result<Value> {
        let mut req = self
            .client
            .get(self.url(&format!("/agents/{agent_id}/context")))
            .header("Authorization", format!("Bearer {}", self.api_key));
        if let Some(conv) = conversation_id {
            req = req.query(&[("conversation_id", conv)]);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "get_context_stats failed {}",
                resp.status()
            )));
        }
        Ok(resp.json().await?)
    }

    /// Delete all messages for an agent (clear context window).
    pub async fn clear_messages(&self, agent_id: &str) -> Result<usize> {
        let resp = self
            .client
            .delete(self.url(&format!("/agents/{agent_id}/messages")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "clear_messages failed {}",
                resp.status()
            )));
        }
        let body: Value = resp.json().await?;
        Ok(body["deleted"].as_u64().unwrap_or(0) as usize)
    }

    /// Search message history for an agent.
    pub async fn search_messages(&self, agent_id: &str, query: &str) -> Result<Vec<Value>> {
        let resp = self
            .client
            .get(self.url(&format!("/agents/{agent_id}/messages")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .query(&[("q", query)])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "search_messages failed {}",
                resp.status()
            )));
        }
        let body: Value = resp.json().await?;
        Ok(body["messages"].as_array().cloned().unwrap_or_default())
    }
}
