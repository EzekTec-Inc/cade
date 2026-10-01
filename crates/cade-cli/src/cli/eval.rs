/// Eval harness CLI: run benchmark tasks against a CADE agent.
///
/// Usage:
///   cade eval run <task.json> [--model <model>]
///   cade eval bench <tasks_dir/> [--model <m>] [--concurrency 4]
///   cade eval list
///   cade eval show <run_id>
use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use cade_agent::agent::client::{
    CadeMessage, CreateAgentRequest, CreateToolRequest, HttpTransport,
};
use cade_agent::tools::IsolatedWorkspace;
use cade_core::permissions::PermissionMode;
use serde_json::{Value, json};

use crate::Result;

// region:    --- Task format

/// A single eval task loaded from a JSON file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvalTask {
    pub name: String,
    pub description: Option<String>,
    pub prompt: String,
    /// Shell command to prepare the copied task workspace before the agent turn.
    pub setup: Option<String>,
    /// Assertions to check after the run
    #[serde(default)]
    pub assertions: Vec<Assertion>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, serde::Serialize)]
pub struct EvalResult {
    pub task_name: String,
    pub run_id: String,
    pub passed: bool,
    pub score: f64,
    pub output: String,
    pub failures: Vec<String>,
    /// Attempt wall time through cleanup, before writing the completion record.
    pub duration_ms: u128,
    /// Only known after the completion request returns; null in stored payload.
    pub persistence_ms: Option<u128>,
    pub status: String,
    pub failed_phase: Option<String>,
    pub model: Option<String>,
    pub agent_id: Option<String>,
    pub task_id: Option<String>,
    pub runtime_run_id: Option<String>,
    /// Local wall-clock phases, including setup, verification and cleanup.
    pub phase_ms: BTreeMap<String, u128>,
    pub stats: EvalStats,
}

impl EvalResult {
    fn new(task_name: String) -> Self {
        Self {
            task_name,
            run_id: String::new(),
            passed: false,
            score: 0.0,
            output: String::new(),
            failures: Vec::new(),
            duration_ms: 0,
            persistence_ms: None,
            status: "error".into(),
            failed_phase: None,
            model: None,
            agent_id: None,
            task_id: None,
            runtime_run_id: None,
            phase_ms: BTreeMap::new(),
            stats: EvalStats::default(),
        }
    }

    fn fail(&mut self, phase: &str, error: impl std::fmt::Display) {
        self.passed = false;
        self.score = 0.0;
        if self.status != "timed_out" {
            self.status = "error".into();
        }
        self.failed_phase.get_or_insert_with(|| phase.to_owned());
        self.failures.push(format!("{phase}: {error}"));
    }

    pub fn print_summary(&self) {
        let icon = if self.passed { "✓" } else { "✗" };
        let pct = (self.score * 100.0) as u32;
        println!(
            "{icon} {name}  score={pct}%  status={}  run={}",
            self.status,
            if self.run_id.is_empty() {
                "unrecorded"
            } else {
                &self.run_id
            },
            name = self.task_name,
        );
        for f in &self.failures {
            println!("    ✗ {f}");
        }
        println!(
            "    duration={}ms  tokens(in/out)={}/{}  cost_usd={}  usage={}",
            self.duration_ms,
            self.stats
                .input_tokens
                .map(|n| n.to_string())
                .unwrap_or_else(|| "unknown".into()),
            self.stats
                .output_tokens
                .map(|n| n.to_string())
                .unwrap_or_else(|| "unknown".into()),
            self.stats
                .cost_usd
                .map(|n| format!("{n:.6}"))
                .unwrap_or_else(|| "unknown".into()),
            if self.stats.complete {
                "terminal"
            } else {
                "partial/absent"
            },
        );
    }
}

/// Preserve server usage verbatim. Missing measurements are unknown, not zero;
/// the runtime may itself estimate tokens, so no provider-native claim is made.
#[derive(Debug, Default, serde::Serialize)]
pub struct EvalStats {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub tool_calls: u64,
    pub usage_events: Vec<Value>,
    pub terminal_event: Option<Value>,
    pub complete: bool,
    pub usage_source: Option<&'static str>,
}

#[derive(Default)]
struct RunCapture {
    run_id: Option<String>,
    output: String,
    status: Option<String>,
    errors: Vec<String>,
    seen: HashSet<(String, i64)>,
    stats: EvalStats,
}

impl RunCapture {
    fn observe(&mut self, event: &CadeMessage) {
        if let Some(id) = event.run_id() {
            self.run_id = Some(id.to_owned());
            if let Some(seq) = event.seq_id()
                && !self.seen.insert((id.to_owned(), seq))
            {
                return;
            }
        }
        if let Some(text) = event.assistant_text() {
            self.output.push_str(text);
        }
        match event.msg_type() {
            "usage_statistics" => {
                self.stats.usage_source =
                    Some("server_reported; token estimation provenance may be unspecified");
                self.stats.usage_events.push(event.data.clone());
            }
            "tool_call_message" => self.stats.tool_calls += 1,
            "error" => self.errors.push(
                event.data["error"]
                    .as_str()
                    .unwrap_or("runtime reported an unspecified error")
                    .to_owned(),
            ),
            "run_done" => {
                self.status = event.data["status"].as_str().map(str::to_owned);
                self.stats.terminal_event = Some(event.data.clone());
                self.stats.complete = true;
            }
            _ => {}
        }
    }

    fn finish(mut self, result: &mut EvalResult) {
        let sum = |key: &str| -> Option<u64> {
            if self.stats.usage_events.is_empty() {
                return None;
            }
            self.stats
                .usage_events
                .iter()
                .try_fold(0_u64, |total, event| {
                    total.checked_add(event[key].as_u64()?)
                })
        };
        self.stats.input_tokens = sum("input_tokens");
        self.stats.output_tokens = sum("output_tokens");
        self.stats.cache_read_tokens = sum("cache_read_tokens");
        self.stats.cache_write_tokens = sum("cache_write_tokens");
        if !self.stats.usage_events.is_empty() {
            self.stats.cost_usd = self
                .stats
                .usage_events
                .iter()
                .try_fold(0.0, |total, event| {
                    let cost = event["cost_usd"].as_f64()?;
                    let sum = total + cost;
                    (cost >= 0.0 && sum.is_finite()).then_some(sum)
                });
        }
        result.runtime_run_id = self.run_id;
        result.output = self.output;
        result.stats = self.stats;
    }
}

struct PhaseClock {
    phase: &'static str,
    started: Instant,
}

impl PhaseClock {
    fn enter(&mut self, phase: &'static str, result: &mut EvalResult) {
        *result.phase_ms.entry(self.phase.to_owned()).or_default() +=
            self.started.elapsed().as_millis();
        self.phase = phase;
        self.started = Instant::now();
    }
}

// endregion: --- EvalResult

// region:    --- Commands

/// `cade eval list`
pub async fn cmd_list(client: &HttpTransport) -> Result<()> {
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
pub async fn cmd_show(client: &HttpTransport, run_id: &str) -> Result<()> {
    let mut run = client
        .get_eval_run(run_id)
        .await
        .map_err(|e| crate::Error::custom(format!("get_eval_run: {e}")))?;
    if let Some(record) = run["result"].as_str()
        && let Ok(record) = serde_json::from_str::<Value>(record)
    {
        run["result"] = record;
    }
    println!("{}", serde_json::to_string_pretty(&run).unwrap_or_default());
    Ok(())
}

/// `cade eval run <task_file>`
pub async fn cmd_run(
    client: &HttpTransport,
    task_file: &Path,
    model_opt: Option<&str>,
    cwd: &Path,
) -> Result<EvalResult> {
    let started = Instant::now();
    let loaded = std::fs::read_to_string(task_file)
        .map_err(|e| format!("read {}: {e}", task_file.display()))
        .and_then(|content| {
            serde_json::from_str::<EvalTask>(&content)
                .map_err(|e| format!("parse {}: {e}", task_file.display()))
        });
    let task = match loaded {
        Ok(task) => task,
        Err(error) => {
            let mut result = EvalResult::new(task_file.display().to_string());
            result.fail("load", error);
            result.duration_ms = started.elapsed().as_millis();
            return Ok(result);
        }
    };
    run_task(client, &task, model_opt, cwd).await
}

/// `cade eval bench <dir/>`
pub async fn cmd_bench(
    client: &HttpTransport,
    tasks_dir: &Path,
    model_opt: Option<&str>,
    concurrency: usize,
    cwd: &Path,
) -> Result<Vec<EvalResult>> {
    if concurrency == 0 {
        return Err(crate::Error::custom(
            "eval concurrency must be greater than zero",
        ));
    }
    let mut task_files: Vec<PathBuf> = std::fs::read_dir(tasks_dir)
        .map_err(|e| crate::Error::custom(format!("read dir: {e}")))?
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|e| crate::Error::custom(format!("read task entry: {e}")))?
        .into_iter()
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
        for (file, result) in chunk.iter().zip(results) {
            let res = result.unwrap_or_else(|error| {
                let mut result = EvalResult::new(file.display().to_string());
                result.fail("task", error);
                result
            });
            res.print_summary();
            all_results.push(res);
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
    client: &HttpTransport,
    task: &EvalTask,
    model_opt: Option<&str>,
    cwd: &Path,
) -> Result<EvalResult> {
    let started = Instant::now();
    println!("  Running: {}", task.name);
    let mut result = EvalResult::new(task.name.clone());
    let mut clock = PhaseClock {
        phase: "validation",
        started: Instant::now(),
    };
    let mut workspace = None;
    let capture = parking_lot::Mutex::new(RunCapture::default());
    let ephemeral_name = format!("eval-{}", uuid::Uuid::new_v4());
    let mut creating_agent = false;
    let mut starting_run = false;

    // A single budget covers setup, transport, model work and verification.
    // State lives outside this future so dropping it cannot lose cleanup IDs or
    // already observed usage/output.
    let execution = async {
        validate_task(task)?;
        clock.enter("isolation", &mut result);
        let source = cwd.to_path_buf();
        workspace = Some(
            tokio::task::spawn_blocking(move || IsolatedWorkspace::clone_from(&source))
                .await
                .map_err(|e| crate::Error::custom(format!("copy task workspace: {e}")))??,
        );
        let task_cwd = workspace
            .as_ref()
            .expect("workspace was just created")
            .path();

        clock.enter("setup", &mut result);
        if let Some(command) = &task.setup {
            run_shell(command, task_cwd).await?;
        }

        clock.enter("model", &mut result);
        let env_model = std::env::var("CADE_DEFAULT_MODEL").ok();
        let model = match model_opt.or(env_model.as_deref()) {
            Some(model) if !model.trim().is_empty() => model.trim().to_owned(),
            Some(_) => return Err(crate::Error::custom("model must not be empty")),
            None => server_model(client).await?,
        };
        result.model = Some(model.clone());

        clock.enter("agent", &mut result);
        creating_agent = true;
        let agent = client
            .create_agent(CreateAgentRequest {
                name: Some(ephemeral_name.clone()),
                model: model.clone(),
                description: Some(format!("eval: {}", task.name)),
                system_prompt: None,
                memory_blocks: vec![],
                tool_ids: vec![],
            })
            .await?;
        result.agent_id = Some(agent.id.clone());

        clock.enter("storage", &mut result);
        create_record(client, task, &mut result).await?;

        clock.enter("tools", &mut result);
        attach_eval_tools(client, &agent.id, &model).await?;

        clock.enter("runtime", &mut result);
        starting_run = true;
        let options = json!({
            "cwd": task_cwd,
            "allowed_paths": [task_cwd],
            "permission_mode": PermissionMode::BypassPermissions.to_string(),
            "execution": { "backend": "local" },
        });
        client
            .start_run_cancellable_with_options(
                &agent.id,
                &task.prompt,
                None,
                &options,
                |event| capture.lock().observe(event),
                None,
            )
            .await?;
        {
            let observed = capture.lock();
            if !observed.errors.is_empty() {
                return Err(crate::Error::custom(observed.errors.join("; ")));
            }
            if observed.status.as_deref() != Some("done") {
                return Err(crate::Error::custom(format!(
                    "runtime did not complete successfully (status={})",
                    observed.status.as_deref().unwrap_or("unknown"),
                )));
            }
        }

        clock.enter("verification", &mut result);
        let output = capture.lock().output.clone();
        result.failures = verify_assertions(&task.assertions, &output, task_cwd).await;
        result.score =
            (task.assertions.len() - result.failures.len()) as f64 / task.assertions.len() as f64;
        result.passed = result.failures.is_empty();
        result.status = if result.passed { "passed" } else { "failed" }.into();
        if !result.passed {
            result.failed_phase = Some("verification".into());
        }
        Ok::<(), crate::Error>(())
    };
    match tokio::time::timeout(Duration::from_secs(task.timeout_secs.max(1)), execution).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => result.fail(clock.phase, error),
        Err(_) => {
            result.status = "timed_out".into();
            result.fail(
                clock.phase,
                format!("timed out after {}s", task.timeout_secs),
            );
        }
    }
    capture.into_inner().finish(&mut result);

    clock.enter("cleanup", &mut result);
    // If creation was accepted but its response was lost, the nonce identifies
    // this task's ephemeral agent without touching any user's persistent agent.
    if creating_agent && result.agent_id.is_none() {
        match tokio::time::timeout(Duration::from_secs(10), client.list_agents()).await {
            Ok(Ok(agents)) => {
                result.agent_id = agents
                    .into_iter()
                    .find(|agent| agent.name == ephemeral_name)
                    .map(|agent| agent.id)
            }
            Ok(Err(error)) => result.fail("cleanup", error),
            Err(_) => result.fail("cleanup", "agent discovery timed out"),
        }
    }
    if let Some(agent_id) = result.agent_id.clone() {
        if starting_run && !result.stats.complete {
            match tokio::time::timeout(
                Duration::from_secs(10),
                stop_eval_run(client, &agent_id, &mut result.runtime_run_id),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => result.fail("cleanup", error),
                Err(_) => result.fail("cleanup", "durable run cancellation timed out"),
            }
        }
        match tokio::time::timeout(Duration::from_secs(10), client.delete_agent(&agent_id)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => result.fail("cleanup", error),
            Err(_) => result.fail("cleanup", "ephemeral agent deletion timed out"),
        }
    }
    // Plain copy isolation is deliberately discarded, never merged into cwd.
    if let Some(workspace) = workspace.take()
        && let Err(error) = workspace.close()
    {
        result.fail("cleanup", error);
    }
    clock.enter("persistence", &mut result);
    result.duration_ms = started.elapsed().as_millis();
    let persistence_started = Instant::now();
    let persistence = async {
        // Also store setup/validation failures, which never created an agent.
        create_record(client, task, &mut result).await?;
        complete_record(client, &result).await
    };
    match tokio::time::timeout(Duration::from_secs(15), persistence).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => result.fail("persistence", error),
        Err(_) => result.fail("persistence", "result storage timed out"),
    }
    result.persistence_ms = Some(persistence_started.elapsed().as_millis());
    clock.enter("finished", &mut result);
    Ok(result)
}

/// Resolve a configured or discovered server model without a catalogue literal
/// silently turning an unavailable provider into an apparently runnable model.
pub async fn server_model(client: &HttpTransport) -> Result<String> {
    if let Ok(config) = client.raw_get("/config").await
        && let Some(model) = configured_model(&config)
    {
        return Ok(model);
    }
    let models = client.list_models().await?;
    models["dynamic"].as_array().into_iter().flatten()
        .filter_map(|entry| entry["id"].as_str())
        .find(|id| !id.trim().is_empty()).map(str::to_owned)
        .ok_or_else(|| crate::Error::custom("No configured/discovered model on the CADE server; use --model or CADE_DEFAULT_MODEL"))
}

fn configured_model(config: &Value) -> Option<String> {
    let model = config["default_model"].as_str()?.trim();
    if model.is_empty() {
        return None;
    }
    let provider = match model.split_once('/') {
        Some((provider, _)) => provider,
        None => config["provider"].as_str()?.trim(),
    };
    if provider.is_empty() {
        return None;
    }
    if let Some(available) = config["available_providers"].as_array()
        && !available.iter().any(|name| name.as_str() == Some(provider))
    {
        return None;
    }
    if model.contains('/') {
        return Some(model.to_owned());
    }
    Some(format!("{provider}/{model}"))
}

fn validate_task(task: &EvalTask) -> Result<()> {
    if task.name.trim().is_empty() || task.prompt.trim().is_empty() {
        return Err(crate::Error::custom(
            "task name and prompt must not be empty",
        ));
    }
    if task.timeout_secs == 0 {
        return Err(crate::Error::custom(
            "task timeout_secs must be greater than zero",
        ));
    }
    if task.assertions.is_empty() {
        return Err(crate::Error::custom(
            "task requires at least one verifier assertion",
        ));
    }
    for assertion in &task.assertions {
        match assertion {
            Assertion::CommandPasses { command } if command.trim().is_empty() => {
                return Err(crate::Error::custom("command verifier must not be empty"));
            }
            Assertion::OutputContains { text } | Assertion::OutputNotContains { text }
                if text.is_empty() =>
            {
                return Err(crate::Error::custom(
                    "output verifier text must not be empty",
                ));
            }
            Assertion::FileExists { path } | Assertion::FileNotExists { path } => {
                validate_relative_path(path)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<()> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || !path.components().any(|c| matches!(c, Component::Normal(_)))
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(crate::Error::custom(
            "file verifier path must be relative to the isolated workspace",
        ));
    }
    Ok(())
}

fn verifier_file_exists(cwd: &Path, path: &str) -> Result<bool> {
    validate_relative_path(path)?;
    let root = cwd.canonicalize()?;
    let mut target = root.clone();
    for component in Path::new(path).components() {
        target.push(component.as_os_str());
        match std::fs::symlink_metadata(&target) {
            Ok(_) => {
                if !target.canonicalize()?.starts_with(&root) {
                    return Err(crate::Error::custom(
                        "file verifier resolves outside the isolated workspace",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(target.try_exists()?)
}

async fn run_shell(command: &str, cwd: &Path) -> Result<()> {
    #[cfg(windows)]
    let mut child = {
        let mut child = tokio::process::Command::new("cmd");
        child.args(["/D", "/S", "/C", command]);
        child
    };
    #[cfg(not(windows))]
    let mut child = {
        let mut child = tokio::process::Command::new("sh");
        child.args(["-c", command]);
        child
    };
    let output = child
        .current_dir(cwd)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .output()
        .await?;
    if !output.status.success() {
        return Err(crate::Error::custom(format!(
            "command '{command}' exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim(),
        )));
    }
    Ok(())
}

async fn verify_assertions(assertions: &[Assertion], output: &str, cwd: &Path) -> Vec<String> {
    if assertions.is_empty() {
        return vec!["task requires at least one verifier assertion".into()];
    }
    let mut failures = Vec::new();
    for a in assertions {
        match a {
            Assertion::OutputContains { text } => {
                if text.is_empty() || !output.contains(text.as_str()) {
                    failures.push(format!("output_contains: expected '{text}'"));
                }
            }
            Assertion::OutputNotContains { text } => {
                if text.is_empty() || output.contains(text.as_str()) {
                    failures.push(format!("output_not_contains: found '{text}'"));
                }
            }
            Assertion::FileExists { path } => match verifier_file_exists(cwd, path) {
                Ok(true) => {}
                Ok(false) => failures.push(format!("file_exists: '{path}' not found")),
                Err(error) => failures.push(format!("file_exists: '{path}': {error}")),
            },
            Assertion::FileNotExists { path } => match verifier_file_exists(cwd, path) {
                Ok(false) => {}
                Ok(true) => failures.push(format!("file_not_exists: '{path}' present")),
                Err(error) => failures.push(format!("file_not_exists: '{path}': {error}")),
            },
            Assertion::CommandPasses { command } => {
                if command.trim().is_empty() {
                    failures.push("command_passes: verifier command must not be empty".into());
                } else if let Err(error) = run_shell(command, cwd).await {
                    failures.push(format!("command_passes: {error}"));
                }
            }
        }
    }

    failures
}

async fn attach_eval_tools(client: &HttpTransport, agent_id: &str, model: &str) -> Result<()> {
    let native = cade_agent::tools::schemas_for_toolset(
        cade_core::toolsets::Toolset::for_model(model),
        false,
    );
    let mut ids = Vec::new();
    for (schemas, meta) in [
        (cade_agent::tools::all_meta_schemas(), true),
        (native, false),
    ] {
        for schema in schemas {
            let source_code = if meta {
                String::new()
            } else {
                cade_agent::agent::tools::build_python_stub_from_schema(
                    schema["name"].as_str().unwrap_or(""),
                    schema["description"].as_str().unwrap_or(""),
                    &schema["parameters"],
                )
            };
            let tool = client
                .create_tool(CreateToolRequest {
                    source_code,
                    source_type: if meta { "json" } else { "python" }.into(),
                    json_schema: Some(schema),
                    tags: if meta {
                        vec!["cade".into(), "meta".into()]
                    } else {
                        vec!["cade".into()]
                    },
                })
                .await?;
            ids.push(tool.id);
        }
    }
    // Unlike the legacy attachment helper, propagate non-success HTTP status.
    client
        .raw_post(
            &format!("/agents/{agent_id}/tools"),
            &json!({ "tool_ids": ids }),
        )
        .await?;
    Ok(())
}

async fn create_record(
    client: &HttpTransport,
    task: &EvalTask,
    result: &mut EvalResult,
) -> Result<()> {
    if !result.run_id.is_empty() {
        return Ok(());
    }
    if result.task_id.is_none() {
        let record = client
            .raw_post(
                "/evals/tasks",
                &json!({
                    "name": task.name, "prompt": task.prompt, "description": task.description,
                    "expected_json": task,
                }),
            )
            .await?;
        result.task_id = Some(
            record["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| crate::Error::custom("eval task storage returned no ID"))?
                .to_owned(),
        );
    }
    result.run_id = client
        .create_eval_run(
            result.task_id.as_deref().expect("task record exists"),
            result.agent_id.as_deref(),
            result.model.as_deref(),
        )
        .await?;
    if result.run_id.is_empty() {
        return Err(crate::Error::custom("eval run storage returned no ID"));
    }
    Ok(())
}

fn completion_payload(result: &EvalResult) -> Result<Value> {
    // Store the observed execution duration. The completion PATCH itself is not
    // yet measurable inside its own payload; returned phase_ms includes it.
    Ok(json!({
        "status": result.status, "score": result.score,
        "duration_ms": result.duration_ms.min(i64::MAX as u128) as i64,
        "completed_at": chrono::Utc::now().timestamp(),
        "result_json": serde_json::to_string(result)?,
    }))
}

async fn complete_record(client: &HttpTransport, result: &EvalResult) -> Result<()> {
    let response = reqwest::Client::new()
        .patch(format!(
            "{}/v1/evals/runs/{}",
            client.base_url().trim_end_matches('/'),
            result.run_id
        ))
        .bearer_auth(client.api_key())
        .json(&completion_payload(result)?)
        .send()
        .await
        .map_err(crate::Error::custom_from_err)?
        .error_for_status()
        .map_err(crate::Error::custom_from_err)?;
    let body: Value = response
        .json()
        .await
        .map_err(crate::Error::custom_from_err)?;
    if body["updated"].as_bool() != Some(true) {
        return Err(crate::Error::custom("eval completion was not stored"));
    }
    Ok(())
}

async fn stop_eval_run(
    client: &HttpTransport,
    agent_id: &str,
    run_id: &mut Option<String>,
) -> Result<()> {
    if run_id.is_none() {
        // An SSE handshake may be lost after acceptance. This agent is unique
        // to one task, so its runs cannot belong to an unrelated session.
        let runs = client.raw_get(&format!("/agents/{agent_id}/runs")).await?;
        *run_id = runs["runs"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|run| run["id"].as_str().map(str::to_owned));
    }
    if let Some(id) = run_id.as_deref() {
        client.cancel_run(id).await?;
        loop {
            let run = client.get_run(id).await?;
            if matches!(run["status"].as_str(), Some("done" | "error" | "cancelled")) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    Ok(())
}

// endregion: --- Task runner

#[cfg(test)]
#[path = "eval/tests.rs"]
mod tests;
