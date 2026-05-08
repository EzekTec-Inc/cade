//! /init command handler.

use crate::Result;
use super::Repl;

impl Repl {
    pub(crate) async fn cmd_init(
        &mut self,
        stdout: &mut std::io::Stdout,
    ) -> Result<bool> {
            self.tui_dim(format!("  Analysing project at {}…", self.cwd.display()));
            let explore_prompt = format!(
                "Analyse the project at '{}'. \
                 Read: README.md, Cargo.toml / package.json / pyproject.toml / go.mod (whichever exist), \
                 src/ or lib/ directory structure (top-level only), .env.example if present. \
                 Return a concise report covering: \
                 (1) Project name and purpose (2 sentences), \
                 (2) Language + framework / stack, \
                 (3) Key source directories and their purpose, \
                 (4) Build / test commands, \
                 (5) Any important conventions or notes from README. \
                 Be specific and factual. Maximum 400 words.",
                self.cwd.display()
            );
            let agent_id = self.agent_id();
            let client = self.client.clone();
            let cwd = self.cwd.clone();
            let all_defs = cade_agent::subagents::discover_all_subagents(&cwd);
            let explore_def =
                cade_agent::subagents::find_subagent("explore", &all_defs).cloned();
            let main_model = self.model();
            let hooks = self.hooks.clone();
            // Run explore subagent synchronously
            let summary = {
                use crate::cli::headless::run_headless;
                use cade_core::permissions::PermissionManager;
                let _system_prompt =
                    explore_def.map(|d| d.system_prompt).unwrap_or_else(|| {
                        "You are an expert code explorer. Be concise and precise."
                            .to_string()
                    });
                let req = cade_agent::agent::client::CreateAgentRequest {
                    name: Some("init-explore".to_string()),
                    model: main_model,
                    description: Some("Ephemeral init analysis".to_string()),
                    system_prompt: Some(
                        "You are an expert code explorer. Be concise and precise."
                            .to_string(),
                    ),
                    memory_blocks: vec![],
                    tool_ids: vec![],
                };
                match client.create_agent(req).await {
                    Ok(sub) => {
                        let perm = PermissionManager::default();
                        let mcp_empty =
                            std::sync::Arc::new(cade_agent::mcp::McpManager::empty());
                        let result = run_headless(
                            &client,
                            &sub.id,
                            &explore_prompt,
                            &perm,
                            &mcp_empty,
                            &hooks,
                            None,
                            None,
                        )
                        .await;
                        let _ = client.delete_agent(&sub.id).await;
                        result
                            .map(|(s, _)| s)
                            .unwrap_or_else(|e| format!("Analysis failed: {e}"))
                    }
                    Err(e) => format!("Could not spawn explore agent: {e}"),
                }
            };
            // Write summary into project memory block
            let _ = self
                .client
                .upsert_memory(&agent_id, "project", &summary, None)
                .await;
            // Tell the main agent what was discovered
            let init_prompt = format!(
                "[/init completed] Project analysis summary:\n\n{summary}\n\n\
                 I've stored this in your 'project' memory block. \
                 Acknowledge and summarise what you learned in 2-3 sentences."
            );
            self.agent_turn(stdout, &init_prompt).await?;
            let _ = self.app.lock().commit_streaming();
        Ok(false)
    }
}
