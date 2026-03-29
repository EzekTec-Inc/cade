/// Eval harness CLI: run benchmark tasks against a CADE agent.
///
/// Usage:
///   cade eval run <task.json> [--model <model>]
///   cade eval bench <tasks_dir/> [--model <m>] [--concurrency 4]
///   cade eval list
///   cade eval show <run_id>
use std::path::{Path, PathBuf};

use cade_agent::agent::client::CadeClient;
use cade_agent::mcp::McpManager;
use cade_core::permissions::{PermissionManager, PermissionMode};

use crate::Result;

// region:    --- Task format

/// A single eval task loaded from a JSON file.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct EvalTask {
    pub name: String,
    pub description: Option<String>,
    pub prompt: String,
    /// Shell command to run before the agent turn (e.g. "git checkout scenario-1")
    pub setup: Option<String>,
    /// Assertions to check after the run
    #[serde(default)]
    pub assertions: Vec<Assertion>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Assertion {
    CommandPasses { command: String },
    OutputContains { text: String },
    OutputNotContains { text: String },
    FileExists { path: String },
    FileNotExists { path: String },
}

fn default_timeout() -> u64 {
    120
}

// endregion: --- Task format

// region:    --- EvalResult

#[derive(Debug)]
pub struct EvalResult {
    pub task_name: String,
    pub run_id: String,
    pub passed: bool,
    pub score: f64,
    pub output: String,
    pub failures: Vec<String>,
    pub duration_ms: u128,
}

impl EvalResult {
    pub fn print_summary(&self) {
        let icon = if self.passed { "✓" } else { "✗" };
        let pct = (self.score * 100.0) as u32;
        println!(
            "{icon} {name}  score={pct}%  run={}",
            &self.run_id[..12.min(self.run_id.len())],
            name = self.task_name,
        );
        for f in &self.failures {
            println!("    ✗ {f}");
        }
    }
}

// endregion: --- EvalResult

// region:    --- Commands

/// `cade eval list`
pub async fn cmd_list(client: &CadeClient) -> Result<()> {
    let tasks = client
        .list_eval_tasks()
        .await
        .map_err(|e| crate::Error::custom(format!("list_eval_tasks: {e}")))?;
    if tasks.is_empty() {
        println!("No eval tasks found.");
    } else {
        println!("Eval tasks ({}):", tasks.len());
        for t in &tasks {
            println!(
                "  {}  {}",
                t["id"].as_str().unwrap_or("?"),
                t["name"].as_str().unwrap_or("?")
            );
        }
    }
    let runs = client
        .list_eval_runs()
        .await
        .map_err(|e| crate::Error::custom(format!("list_eval_runs: {e}")))?;
    if !runs.is_empty() {
        println!("\nRecent runs ({}):", runs.len().min(10));
        for r in runs.iter().take(10) {
            let id = r["id"].as_str().unwrap_or("?");
            let status = r["status"].as_str().unwrap_or("?");
            let score = r["score"]
                .as_f64()
                .map(|s| format!("{:.0}%", s * 100.0))
                .unwrap_or_else(|| "—".into());
            let model = r["model"].as_str().unwrap_or("?");
            println!("  {id}  {status:<10}  {score:<6}  {model}");
        }
    }
    Ok(())
}

/// `cade eval show <run_id>`
pub async fn cmd_show(client: &CadeClient, run_id: &str) -> Result<()> {
    let run = client
        .get_eval_run(run_id)
        .await
        .map_err(|e| crate::Error::custom(format!("get_eval_run: {e}")))?;
    println!("{}", serde_json::to_string_pretty(&run).unwrap_or_default());
    Ok(())
}

/// `cade eval run <task_file>`
pub async fn cmd_run(
    client: &CadeClient,
    task_file: &Path,
    model_opt: Option<&str>,
    cwd: &Path,
) -> Result<EvalResult> {
    let content = std::fs::read_to_string(task_file)
        .map_err(|e| crate::Error::custom(format!("read {}: {e}", task_file.display())))?;
    let task: EvalTask = serde_json::from_str(&content)
        .map_err(|e| crate::Error::custom(format!("parse task: {e}")))?;
    run_task(client, &task, model_opt, cwd).await
}

/// `cade eval bench <dir/>`
pub async fn cmd_bench(
    client: &CadeClient,
    tasks_dir: &Path,
    model_opt: Option<&str>,
    concurrency: usize,
    cwd: &Path,
) -> Result<Vec<EvalResult>> {
    let mut task_files: Vec<PathBuf> = std::fs::read_dir(tasks_dir)
        .map_err(|e| crate::Error::custom(format!("read dir: {e}")))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    task_files.sort();
    if task_files.is_empty() {
        println!("No *.json task files in {}", tasks_dir.display());
        return Ok(vec![]);
    }
    println!(
        "Running {} eval task(s) (concurrency={concurrency})…",
        task_files.len()
    );

    let mut all_results = Vec::new();
    for chunk in task_files.chunks(concurrency) {
        let futs: Vec<_> = chunk
            .iter()
            .map(|f| {
                let c = client.clone();
                let f = f.clone();
                let m = model_opt.map(String::from);
                let cwd = cwd.to_path_buf();
                async move { cmd_run(&c, &f, m.as_deref(), &cwd).await }
            })
            .collect();
        let results = futures::future::join_all(futs).await;
        for r in results {
            match r {
                Ok(res) => {
                    res.print_summary();
                    all_results.push(res);
                }
                Err(e) => eprintln!("✗ task error: {e}"),
            }
        }
    }

    let passed = all_results.iter().filter(|r| r.passed).count();
    let total = all_results.len();
    let avg = if total > 0 {
        all_results.iter().map(|r| r.score).sum::<f64>() / total as f64
    } else {
        0.0
    };
    println!("\n── Benchmark summary ──");
    println!("{passed}/{total} passed  avg_score={:.0}%", avg * 100.0);
    Ok(all_results)
}

// endregion: --- Commands

// region:    --- Task runner

async fn run_task(
    client: &CadeClient,
    task: &EvalTask,
    model_opt: Option<&str>,
    cwd: &Path,
) -> Result<EvalResult> {
    let t0 = std::time::Instant::now();
    println!("  Running: {}", task.name);

    // Optional setup step
    if let Some(setup_cmd) = &task.setup {
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(setup_cmd)
            .current_dir(cwd)
            .status()
            .map_err(|e| crate::Error::custom(format!("setup: {e}")))?;
        if !status.success() {
            return Err(crate::Error::custom(format!("setup '{setup_cmd}' failed")));
        }
    }

    // Create ephemeral agent
    let model = model_opt.unwrap_or("anthropic/claude-sonnet-4-5-20250929");
    let req = cade_agent::agent::client::CreateAgentRequest {
        name: Some(format!("eval-{}", task.name)),
        model: model.to_string(),
        description: Some(format!("eval: {}", task.name)),
        system_prompt: None,
        memory_blocks: vec![],
        tool_ids: vec![],
    };
    let agent = client
        .create_agent(req)
        .await
        .map_err(|e| crate::Error::custom(format!("create_agent: {e}")))?;

    // Register tools with the eval agent
    cade_agent::tools::register_meta_tools(client).await;
    let toolset = cade_core::toolsets::Toolset::default();
    if let Ok(tools) = cade_agent::agent::tools::register_cade_tools(client, toolset).await {
        let ids: Vec<String> = tools.iter().map(|t| t.id.clone()).collect();
        if !ids.is_empty() {
            let _ = client.attach_agent_tools(&agent.id, &ids).await;
        }
    }

    // Create eval run record
    let run_id = client
        .create_eval_run(&agent.id, Some(&agent.id), Some(model))
        .await
        .unwrap_or_else(|_| format!("local-{}", uuid::Uuid::new_v4()));

    // Run the prompt with timeout
    let permissions = PermissionManager::new(PermissionMode::BypassPermissions);
    let mcp = std::sync::Arc::new(McpManager::empty());
    let hooks = cade_core::hooks::HookEngine::new(Default::default(), cwd.to_path_buf());

    let run_fut = crate::cli::headless::run_headless(
        client,
        &agent.id,
        &task.prompt,
        &permissions,
        &mcp,
        &hooks,
        None,
    );
    let (final_output, _stats) =
        tokio::time::timeout(std::time::Duration::from_secs(task.timeout_secs), run_fut)
            .await
            .map_err(|_| crate::Error::custom(format!("timed out after {}s", task.timeout_secs)))?
            .map_err(|e| crate::Error::custom(format!("headless run: {e}")))?;

    // Delete the ephemeral agent
    let _ = client.delete_agent(&agent.id).await;

    // Check assertions
    let mut failures = Vec::new();
    for a in &task.assertions {
        match a {
            Assertion::OutputContains { text } => {
                if !final_output.contains(text.as_str()) {
                    failures.push(format!("output_contains: expected '{text}'"));
                }
            }
            Assertion::OutputNotContains { text } => {
                if final_output.contains(text.as_str()) {
                    failures.push(format!("output_not_contains: found '{text}'"));
                }
            }
            Assertion::FileExists { path } => {
                if !cwd.join(path).exists() {
                    failures.push(format!("file_exists: '{path}' not found"));
                }
            }
            Assertion::FileNotExists { path } => {
                if cwd.join(path).exists() {
                    failures.push(format!("file_not_exists: '{path}' present"));
                }
            }
            Assertion::CommandPasses { command } => {
                let ok = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(command)
                    .current_dir(cwd)
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
                if !ok {
                    failures.push(format!("command_passes: '{command}' failed"));
                }
            }
        }
    }

    let total = task.assertions.len().max(1);
    let pass_n = total - failures.len().min(total);
    let score = pass_n as f64 / total as f64;

    Ok(EvalResult {
        task_name: task.name.clone(),
        run_id,
        passed: failures.is_empty(),
        score,
        output: final_output,
        failures,
        duration_ms: t0.elapsed().as_millis(),
    })
}

// endregion: --- Task runner
