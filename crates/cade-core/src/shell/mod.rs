//! Deep, cross-platform shell execution engine for CADE.
//!
//! Provides a unified [`ShellExecutionEngine`] hiding OS-specific shell binary
//! resolution, environment merging (`agent_env`, `askpass`), process lifecycle,
//! timeout governance, and 25%/75% head-tail middle-truncation.

use crate::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const DEFAULT_TIMEOUT_SECS: u64 = 120;
pub const MAX_OUTPUT_CHARS: usize = 16_384;

/// Execution request passed across the shell seam.
#[derive(Debug, Clone)]
pub struct ShellRequest<'a> {
    pub command: &'a str,
    pub working_dir: Option<&'a Path>,
    pub timeout: Duration,
    pub env: HashMap<String, String>,
}

impl<'a> ShellRequest<'a> {
    pub fn new(command: &'a str) -> Self {
        Self {
            command,
            working_dir: None,
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            env: HashMap::new(),
        }
    }

    pub fn with_working_dir(mut self, dir: &'a Path) -> Self {
        self.working_dir = Some(dir);
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.env.insert(key.into(), val.into());
        self
    }
}

/// Structured outcome returned by the [`ShellExecutionEngine`].
#[derive(Debug, Clone)]
pub struct ShellResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration: Duration,
    pub truncated: bool,
}

impl ShellResult {
    /// Render formatted output string for LLM context window.
    pub fn format_for_llm(&self) -> String {
        let mut out = String::new();
        if !self.stdout.is_empty() {
            out.push_str(&self.stdout);
        }
        if !self.stderr.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str("STDERR:\n");
            out.push_str(&self.stderr);
        }
        if self.exit_code != 0 {
            if out.is_empty() {
                out = format!("(exit code {})", self.exit_code);
            } else {
                out.push_str(&format!("\n(exit code {})", self.exit_code));
            }
        }
        out
    }
}

/// Seam interface for platform-specific process spawning adapters.
pub trait ShellAdapter: Send + Sync {
    fn build_command(&self, command: &str) -> tokio::process::Command;
    fn build_command_sync(&self, command: &str) -> std::process::Command;
}

// ── POSIX Adapter (Linux & macOS) ─────────────────────────────────────────────

pub struct PosixShellAdapter;

impl ShellAdapter for PosixShellAdapter {
    fn build_command(&self, command: &str) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("-c").arg(command);
        cmd
    }

    fn build_command_sync(&self, command: &str) -> std::process::Command {
        let mut cmd = std::process::Command::new("bash");
        cmd.arg("-c").arg(command);
        cmd
    }
}

// ── Windows Portable Shell Adapter ───────────────────────────────────────────

pub struct WindowsPortableShellAdapter {
    resolved_binary: String,
    is_wsl: bool,
}

impl WindowsPortableShellAdapter {
    pub fn auto_detect() -> Self {
        if let Some(git_bash) = Self::find_git_bash() {
            return Self {
                resolved_binary: git_bash,
                is_wsl: false,
            };
        }
        if Self::has_wsl() {
            return Self {
                resolved_binary: "wsl.exe".to_string(),
                is_wsl: true,
            };
        }
        if let Some(msys2) = Self::find_msys2() {
            return Self {
                resolved_binary: msys2,
                is_wsl: false,
            };
        }
        Self {
            resolved_binary: "cmd.exe".to_string(),
            is_wsl: false,
        }
    }

    fn find_git_bash() -> Option<String> {
        let default_path = PathBuf::from(r"C:\Program Files\Git\bin\bash.exe");
        if default_path.exists() {
            return Some(default_path.to_string_lossy().to_string());
        }
        if let Ok(path) = which::which("bash") {
            return Some(path.to_string_lossy().to_string());
        }
        None
    }

    fn has_wsl() -> bool {
        which::which("wsl.exe").is_ok()
    }

    fn find_msys2() -> Option<String> {
        let default_path = PathBuf::from(r"C:\msys64\usr\bin\bash.exe");
        if default_path.exists() {
            return Some(default_path.to_string_lossy().to_string());
        }
        None
    }
}

impl ShellAdapter for WindowsPortableShellAdapter {
    fn build_command(&self, command: &str) -> tokio::process::Command {
        if self.is_wsl {
            let mut cmd = tokio::process::Command::new(&self.resolved_binary);
            cmd.args(["-e", "bash", "-c"]).arg(command);
            cmd
        } else if self.resolved_binary.ends_with("cmd.exe") {
            let mut cmd = tokio::process::Command::new("cmd.exe");
            cmd.arg("/C").arg(command);
            cmd
        } else {
            let mut cmd = tokio::process::Command::new(&self.resolved_binary);
            cmd.arg("-c").arg(command);
            cmd
        }
    }

    fn build_command_sync(&self, command: &str) -> std::process::Command {
        if self.is_wsl {
            let mut cmd = std::process::Command::new(&self.resolved_binary);
            cmd.args(["-e", "bash", "-c"]).arg(command);
            cmd
        } else if self.resolved_binary.ends_with("cmd.exe") {
            let mut cmd = std::process::Command::new("cmd.exe");
            cmd.arg("/C").arg(command);
            cmd
        } else {
            let mut cmd = std::process::Command::new(&self.resolved_binary);
            cmd.arg("-c").arg(command);
            cmd
        }
    }
}

// ── Primary Engine ───────────────────────────────────────────────────────────

pub struct ShellExecutionEngine {
    adapter: Box<dyn ShellAdapter>,
}

impl Default for ShellExecutionEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellExecutionEngine {
    pub fn new() -> Self {
        #[cfg(unix)]
        let adapter = Box::new(PosixShellAdapter);

        #[cfg(windows)]
        let adapter = Box::new(WindowsPortableShellAdapter::auto_detect());

        #[cfg(not(any(unix, windows)))]
        let adapter = Box::new(PosixShellAdapter);

        Self { adapter }
    }

    pub fn with_adapter(adapter: Box<dyn ShellAdapter>) -> Self {
        Self { adapter }
    }

    /// Execute a command to completion synchronously with timeouts & truncation.
    pub async fn execute(&self, req: ShellRequest<'_>) -> Result<ShellResult> {
        let start_time = std::time::Instant::now();

        if crate::permissions::bash_command_is_suspicious(req.command) {
            tracing::warn!(
                "shell_engine: executing suspicious command: {:?}",
                req.command.chars().take(120).collect::<String>()
            );
        }

        let mut cmd = self.adapter.build_command(req.command);
        if let Some(dir) = req.working_dir {
            cmd.current_dir(dir);
        }
        for (k, v) in &req.env {
            cmd.env(k, v);
        }

        crate::agent_env::apply_agent_env(&mut cmd);
        crate::askpass::apply_askpass_env(&mut cmd);

        let output = tokio::time::timeout(req.timeout, cmd.output())
            .await
            .map_err(|_| crate::Error::custom(format!("Command timed out after {}s", req.timeout.as_secs())))?
            .map_err(|e| crate::Error::custom(format!("Failed to spawn shell: {e}")))?;

        let duration = start_time.elapsed();
        let raw_stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let raw_stderr = String::from_utf8_lossy(&output.stderr).to_string();

        let (stdout, truncated_out) = truncate_head_tail(&raw_stdout, MAX_OUTPUT_CHARS);
        let (stderr, truncated_err) = truncate_head_tail(&raw_stderr, MAX_OUTPUT_CHARS);

        Ok(ShellResult {
            stdout,
            stderr,
            exit_code: output.status.code().unwrap_or(-1),
            duration,
            truncated: truncated_out || truncated_err,
        })
    }
}

/// 25% / 75% Head-Tail Middle-Truncation helper.
pub fn truncate_head_tail(text: &str, max_chars: usize) -> (String, bool) {
    let count = text.chars().count();
    if count <= max_chars {
        return (text.to_string(), false);
    }

    let head_chars = (max_chars as f64 * 0.25).round() as usize;
    let tail_chars = max_chars.saturating_sub(head_chars);
    let omitted = count.saturating_sub(max_chars);

    let head: String = text.chars().take(head_chars).collect();
    let tail: String = text.chars().skip(count.saturating_sub(tail_chars)).collect();

    let formatted = format!(
        "{head}\n\n[... Output truncated: {omitted} characters omitted from middle. Use head/tail/grep to narrow output. ...]\n\n{tail}"
    );

    (formatted, true)
}

// ── Backward Compatibility API ───────────────────────────────────────────────

pub fn shell_command(command: &str) -> tokio::process::Command {
    ShellExecutionEngine::new().adapter.build_command(command)
}

pub fn shell_command_sync(command: &str) -> std::process::Command {
    ShellExecutionEngine::new().adapter.build_command_sync(command)
}

pub fn open_browser(url: &str) -> std::io::Result<std::process::Child> {
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", url])
            .spawn()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Unsupported target platform",
        ))
    }
}

#[cfg(test)]
mod tests;
