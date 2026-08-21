use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::Stream;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use cade_agent::mcp::McpManager;
use cade_agent::team::{
    LlmCompleter, MemberDef, MemberScope, MemberTools, SubagentRunner, TeamConfig, TeamDef,
    TeamExecutor, TeamMode, TeamResultItem, builtin_members,
};
use cade_agent::tools::manager::ToolResult;
use cade_agent::tools::{RuntimeToolResult, ToolRuntime, all_schemas};
use cade_ai::{
    AiConfig, CompletionRequest, LlmMessage, LlmProvider, LlmRouter,
};
use cade_store::Db;

use crate::embedded::EmbeddedStorageBackend;
use crate::events::CadeStreamEvent;
use crate::{Error, Result};

// region:    --- TeamRunnerAdapters

struct TeamSubagentRunner {
    runtime: Arc<ToolRuntime>,
    provider: Arc<dyn LlmProvider>,
    default_model: String,
    stream_tx: Option<mpsc::Sender<CadeStreamEvent>>,
}

#[async_trait]
impl SubagentRunner for TeamSubagentRunner {
    async fn run_subagent(
        &self,
        task_call_id: &str,
        args: &Value,
    ) -> std::result::Result<ToolResult, String> {
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let member_id = args
            .get("agent_id")
            .or_else(|| args.get("member_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("worker");

        let system_prompt = args
            .get("system_prompt")
            .and_then(|v| v.as_str())
            .map(String::from);

        let model = args
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.default_model)
            .to_string();

        if let Some(tx) = &self.stream_tx {
            let _ = tx
                .send(CadeStreamEvent::ToolExecuting {
                    tool_call_id: task_call_id.to_string(),
                    tool_name: format!("subagent:{member_id}"),
                    arguments: args.clone(),
                })
                .await;
        }

        let mut messages = Vec::new();
        if let Some(sys) = system_prompt {
            messages.push(LlmMessage {
                role: "system".to_string(),
                content: sys,
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            });
        }
        messages.push(LlmMessage {
            role: "user".to_string(),
            content: prompt,
            tool_call_id: None,
            tool_calls: None,
            images: None,
            cache_control: None,
        });

        let tools = all_schemas(false);
        let req = CompletionRequest {
            model,
            messages,
            tools,
            max_tokens: 4096,
            reasoning_effort: None,
        };

        let mut last_output = String::new();
        let mut is_error = false;

        match self.provider.complete(&req).await {
            Ok(resp) => {
                let content = resp.content.unwrap_or_default();
                last_output.clone_from(&content);

                if let Some(tx) = &self.stream_tx
                    && !content.is_empty()
                {
                    let _ = tx.send(CadeStreamEvent::MessageDelta(content.clone())).await;
                }

                for tc in resp.tool_calls {
                    let res = self
                        .runtime
                        .execute(tc.id.clone(), &tc.name, &tc.arguments)
                        .await
                        .unwrap_or_else(|| RuntimeToolResult {
                            tool_call_id: tc.id.clone(),
                            tool_name: tc.name.clone(),
                            output: format!("Tool {} not found", tc.name),
                            is_error: true,
                            ui_resource_uri: None,
                        });
                    last_output = res.output;
                    is_error = res.is_error;
                }
            }
            Err(e) => {
                last_output = format!("Subagent error: {e}");
                is_error = true;
            }
        }

        if let Some(tx) = &self.stream_tx {
            let _ = tx
                .send(CadeStreamEvent::ToolCompleted {
                    tool_call_id: task_call_id.to_string(),
                    tool_name: format!("subagent:{member_id}"),
                    output: last_output.clone(),
                    is_error,
                })
                .await;
        }

        Ok(ToolResult {
            tool_call_id: task_call_id.to_string(),
            tool_name: format!("subagent:{member_id}"),
            output: last_output,
            is_error,
            ui_resource_uri: None,
        })
    }
}

struct TeamLlmCompleter {
    provider: Arc<dyn LlmProvider>,
    stream_tx: Option<mpsc::Sender<CadeStreamEvent>>,
}

#[async_trait]
impl LlmCompleter for TeamLlmCompleter {
    async fn complete(
        &self,
        model: &str,
        system_prompt: Option<&str>,
        prompt: &str,
    ) -> std::result::Result<String, String> {
        let mut messages = Vec::new();
        if let Some(sys) = system_prompt {
            messages.push(LlmMessage {
                role: "system".to_string(),
                content: sys.to_string(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            });
        }
        messages.push(LlmMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
            tool_call_id: None,
            tool_calls: None,
            images: None,
            cache_control: None,
        });

        let req = CompletionRequest {
            model: model.to_string(),
            messages,
            tools: vec![],
            max_tokens: 4096,
            reasoning_effort: None,
        };

        match self.provider.complete(&req).await {
            Ok(resp) => {
                let text = resp.content.unwrap_or_default();
                if let Some(tx) = &self.stream_tx {
                    let _ = tx.send(CadeStreamEvent::MessageDelta(text.clone())).await;
                }
                Ok(text)
            }
            Err(e) => Err(e.to_string()),
        }
    }
}

// endregion: --- TeamRunnerAdapters

// region:    --- TeamSessionBuilder

/// Builder for programmatically configuring multi-agent collaborative teams.
pub struct TeamSessionBuilder {
    team_id: String,
    name: String,
    description: String,
    mode: TeamMode,
    max_iterations: usize,
    leader_model: Option<String>,
    members: Vec<MemberDef>,
    db_path: Option<PathBuf>,
    cwd: PathBuf,
    provider: Option<Arc<dyn LlmProvider>>,
    ai_config: Option<AiConfig>,
}

impl Default for TeamSessionBuilder {
    fn default() -> Self {
        Self {
            team_id: format!("team-{}", uuid::Uuid::new_v4()),
            name: "Multi-Agent Squad".to_string(),
            description: "Programmatic multi-agent squad orchestrated via cade-sdk".to_string(),
            mode: TeamMode::Coordinate,
            max_iterations: 10,
            leader_model: None,
            members: Vec::new(),
            db_path: None,
            cwd: std::env::current_dir().unwrap_or_default(),
            provider: None,
            ai_config: None,
        }
    }
}

impl TeamSessionBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn team_id(mut self, id: impl Into<String>) -> Self {
        self.team_id = id.into();
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    pub fn mode(mut self, mode: TeamMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn max_iterations(mut self, max: usize) -> Self {
        self.max_iterations = max;
        self
    }

    pub fn leader_model(mut self, model: impl Into<String>) -> Self {
        self.leader_model = Some(model.into());
        self
    }

    pub fn in_memory(mut self) -> Self {
        self.db_path = None;
        self
    }

    pub fn db_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.db_path = Some(path.into());
        self
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = cwd.into();
        self
    }

    pub fn provider(mut self, provider: Arc<dyn LlmProvider>) -> Self {
        self.provider = Some(provider);
        self
    }

    pub fn ai_config(mut self, config: AiConfig) -> Self {
        self.ai_config = Some(config);
        self
    }

    /// Add a pre-configured [`MemberDef`].
    pub fn add_member(mut self, member: MemberDef) -> Self {
        self.members.push(member);
        self
    }

    /// Fluent helper to define and register a team specialist member.
    pub fn with_member(
        mut self,
        id: impl Into<String>,
        name: impl Into<String>,
        role: impl Into<String>,
        system_prompt: impl Into<String>,
        tools: MemberTools,
    ) -> Self {
        let id_str = id.into();
        let name_str = name.into();
        let role_str = role.into();
        self.members.push(MemberDef {
            id: id_str.clone(),
            name: name_str,
            role: Some(role_str.clone()),
            description: format!("Specialist role: {role_str}"),
            model: None,
            tools,
            system_prompt: system_prompt.into(),
            skills: Vec::new(),
            scope: MemberScope::Project,
            path: None,
        });
        self
    }

    pub async fn build(mut self) -> Result<TeamSession> {
        if self.members.is_empty() {
            self.members = builtin_members();
        }

        let db_target = match &self.db_path {
            Some(p) => p.to_string_lossy().to_string(),
            None => ":memory:".to_string(),
        };

        let db = cade_store::sqlite::open(&db_target)
            .map_err(|e| Error::custom(format!("failed to open database at {db_target}: {e}")))?;

        let leader_model = self
            .leader_model
            .clone()
            .unwrap_or_else(|| "anthropic/claude-sonnet-4-5".to_string());

        let provider: Arc<dyn LlmProvider> = if let Some(p) = self.provider {
            p
        } else if let Some(cfg) = self.ai_config {
            Arc::new(LlmRouter::build(&cfg))
        } else {
            let env_config = AiConfig {
                anthropic_api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
                openai_api_key: std::env::var("OPENAI_API_KEY").ok(),
                google_api_key: std::env::var("GEMINI_API_KEY")
                    .ok()
                    .or_else(|| std::env::var("GOOGLE_API_KEY").ok()),
                ollama_base_url: std::env::var("OLLAMA_BASE_URL")
                    .unwrap_or_else(|_| "http://localhost:11434".to_string()),
                llm_provider: "anthropic".to_string(),
            };
            Arc::new(LlmRouter::build(&env_config))
        };

        let team_def = TeamDef {
            id: self.team_id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            mode: self.mode,
            max_iterations: self.max_iterations,
            leader_model: Some(leader_model.clone()),
            members: self.members,
            scope: MemberScope::Project,
            path: None,
        };

        let storage = Arc::new(EmbeddedStorageBackend { db: db.clone() });
        let mcp = Arc::new(McpManager::empty());
        let runtime = Arc::new(ToolRuntime::new(
            storage,
            mcp,
            self.team_id.clone(),
            self.cwd,
        ));

        Ok(TeamSession {
            team_def,
            db,
            provider,
            runtime,
            leader_model,
        })
    }
}

// endregion: --- TeamSessionBuilder

// region:    --- TeamSession

/// Multi-agent team orchestration session.
pub struct TeamSession {
    team_def: TeamDef,
    db: Db,
    provider: Arc<dyn LlmProvider>,
    runtime: Arc<ToolRuntime>,
    leader_model: String,
}

impl TeamSession {
    pub fn builder() -> TeamSessionBuilder {
        TeamSessionBuilder::new()
    }

    pub fn team_id(&self) -> &str {
        &self.team_def.id
    }

    pub fn team_name(&self) -> &str {
        &self.team_def.name
    }

    pub fn mode(&self) -> TeamMode {
        self.team_def.mode
    }

    pub fn members(&self) -> &[MemberDef] {
        &self.team_def.members
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    /// Dispatch and execute a team mission synchronously across squad members.
    pub async fn run(&self, prompt: &str) -> Result<Vec<TeamResultItem>> {
        let config = TeamConfig {
            task: prompt.to_string(),
            team_id: self.team_def.id.clone(),
            mode_override: Some(self.team_def.mode),
            background: false,
            model_override: None,
            custom_system_prompt: None,
            description: None,
            test_command: None,
            human_review: false,
            silent_stream: false,
            max_iterations: None,
            depth: 0,
            max_tokens_budget: None,
            isolation: false,
        };

        let runner = TeamSubagentRunner {
            runtime: self.runtime.clone(),
            provider: self.provider.clone(),
            default_model: self.leader_model.clone(),
            stream_tx: None,
        };

        let llm = TeamLlmCompleter {
            provider: self.provider.clone(),
            stream_tx: None,
        };

        let executor = TeamExecutor::new();
        let tool_call_id = format!("team-call-{}", uuid::Uuid::new_v4());

        let results = executor
            .run_team(
                &self.team_def,
                &config,
                &self.leader_model,
                &tool_call_id,
                &runner,
                &llm,
            )
            .await
            .map_err(|e| Error::custom(format!("Team execution failed: {e}")))?;

        Ok(results)
    }

    /// Stream typed [`CadeStreamEvent`] telemetry in real-time during team execution.
    pub async fn stream_run(
        &self,
        prompt: &str,
    ) -> Result<Pin<Box<dyn Stream<Item = CadeStreamEvent> + Send>>> {
        let (tx, rx) = mpsc::channel(64);

        let team_def = self.team_def.clone();
        let leader_model = self.leader_model.clone();
        let runtime = self.runtime.clone();
        let provider = self.provider.clone();
        let prompt_str = prompt.to_string();

        tokio::spawn(async move {
            let config = TeamConfig {
                task: prompt_str,
                team_id: team_def.id.clone(),
                mode_override: Some(team_def.mode),
                background: false,
                model_override: None,
                custom_system_prompt: None,
                description: None,
                test_command: None,
                human_review: false,
                silent_stream: false,
                max_iterations: None,
                depth: 0,
                max_tokens_budget: None,
                isolation: false,
            };

            let runner = TeamSubagentRunner {
                runtime,
                provider: provider.clone(),
                default_model: leader_model.clone(),
                stream_tx: Some(tx.clone()),
            };

            let llm = TeamLlmCompleter {
                provider,
                stream_tx: Some(tx.clone()),
            };

            let executor = TeamExecutor::new();
            let tool_call_id = format!("team-stream-{}", uuid::Uuid::new_v4());

            let _ = tx
                .send(CadeStreamEvent::Thought(format!(
                    "Starting team execution for team '{}' with mode {:?}",
                    team_def.name, team_def.mode
                )))
                .await;

            match executor
                .run_team(
                    &team_def,
                    &config,
                    &leader_model,
                    &tool_call_id,
                    &runner,
                    &llm,
                )
                .await
            {
                Ok(items) => {
                    for item in items {
                        let _ = tx
                            .send(CadeStreamEvent::MessageDelta(format!(
                                "\n[Member output]: {}\n",
                                item.output
                            )))
                            .await;
                    }
                    let _ = tx
                        .send(CadeStreamEvent::Finished {
                            outcome: "team_completed".to_string(),
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx.send(CadeStreamEvent::Error(e)).await;
                }
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

// endregion: --- TeamSession

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cade_ai::{CompletionRequest, CompletionResponse, StreamChunk};
    use futures::StreamExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TeamMockLlmProvider {
        count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl LlmProvider for TeamMockLlmProvider {
        async fn complete(&self, req: &CompletionRequest) -> cade_ai::Result<CompletionResponse> {
            let n = self.count.fetch_add(1, Ordering::SeqCst);
            let prompt = req.messages.last().map(|m| m.content.as_str()).unwrap_or("");
            Ok(CompletionResponse {
                content: Some(format!("Squad member executed successfully (step {n}). Input: {prompt}")),
                tool_calls: vec![],
                finish_reason: "stop".to_string(),
            })
        }

        async fn stream(
            &self,
            req: &CompletionRequest,
        ) -> cade_ai::Result<Pin<Box<dyn Stream<Item = cade_ai::Result<StreamChunk>> + Send>>> {
            let resp = self.complete(req).await?;
            let chunks = vec![
                Ok(StreamChunk::Text(resp.content.unwrap_or_default())),
                Ok(StreamChunk::Done),
            ];
            Ok(Box::pin(futures::stream::iter(chunks)))
        }
    }

    #[tokio::test]
    async fn test_team_session_builder_and_execution() {
        let mock_llm = Arc::new(TeamMockLlmProvider {
            count: Arc::new(AtomicUsize::new(0)),
        });

        let team = TeamSession::builder()
            .team_id("security-squad")
            .name("Security Review Squad")
            .mode(TeamMode::Coordinate)
            .with_member(
                "architect",
                "Lead Architect",
                "System architecture & threat modeling",
                "You are an expert system architect.",
                MemberTools::Readonly,
            )
            .with_member(
                "security_oracle",
                "Security Oracle",
                "Vulnerability detection & remediation",
                "You are an elite security auditor.",
                MemberTools::Readonly,
            )
            .provider(mock_llm)
            .build()
            .await
            .expect("team creation should succeed");

        assert_eq!(team.team_id(), "security-squad");
        assert_eq!(team.members().len(), 2);

        let results = team
            .run("Audit the authentication flow")
            .await
            .expect("team execution should succeed");

        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn test_team_session_streaming() {
        let mock_llm = Arc::new(TeamMockLlmProvider {
            count: Arc::new(AtomicUsize::new(0)),
        });

        let team = TeamSession::builder()
            .team_id("review-squad")
            .name("Code Review Squad")
            .mode(TeamMode::Coordinate)
            .with_member(
                "reviewer",
                "Code Reviewer",
                "Code review and linting",
                "You are a code reviewer.",
                MemberTools::Readonly,
            )
            .provider(mock_llm)
            .build()
            .await
            .expect("team creation should succeed");

        let mut stream = team
            .stream_run("Perform full codebase review")
            .await
            .expect("stream run should succeed");

        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event);
        }

        assert!(!events.is_empty());
        let has_thought = events.iter().any(|e| matches!(e, CadeStreamEvent::Thought(_)));
        let has_finish = events.iter().any(|e| matches!(e, CadeStreamEvent::Finished { .. }));

        assert!(has_thought, "Should emit initial thought");
        assert!(has_finish, "Should emit finished event");
    }
}
