//! Strongly-typed interfaces for CADE's compile-time local tools.
//!
//! Provides traits and runtime adapter structures to unify compile-time checked
//! local Rust implementations alongside CADE's dynamic Model Context Protocol (MCP) loops.

use crate::Result;
use serde_json::Value;
use std::sync::Arc;

/// Runtime context that tools can use to interact with the user and
/// the CADE host environment.
///
/// Provides a channel for tools to request mid-execution user permission
/// for sensitive sub-operations (Pillar 3.1).
#[async_trait::async_trait]
pub trait ToolContext: Send + Sync {
    /// Ask the user for permission to perform a sensitive operation.
    ///
    /// `permission` is a short description of the operation (e.g. "bash.exec")
    /// `pattern` is the specific target (e.g. a file path or command).
    ///
    /// Returns `true` if the user granted permission, `false` if denied.
    async fn ask_permission(&self, permission: &str, pattern: &str) -> bool;
}

/// A default no-op implementation that always denies permission.
/// Used in headless / non-interactive contexts.
pub struct DenyAllContext;

#[async_trait::async_trait]
impl ToolContext for DenyAllContext {
    async fn ask_permission(&self, _permission: &str, _pattern: &str) -> bool {
        false
    }
}

/// A context that always grants permission (/yolo mode).
pub struct AllowAllContext;

#[async_trait::async_trait]
impl ToolContext for AllowAllContext {
    async fn ask_permission(&self, _permission: &str, _pattern: &str) -> bool {
        true
    }
}

/// Strongly-typed, compiler-enforced trait for CADE's core built-in tools.
pub trait BuiltInTool: Send + Sync {
    type Args: serde::de::DeserializeOwned + Send;
    type Output: serde::Serialize + Send;

    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn schema(&self) -> Value;

    fn execute(&self, args: Self::Args) -> Result<Self::Output>;
}

/// Dynamic, runtime-compatible wrapper for built-in tools.
pub struct CoreToolAdapter {
    inner: Arc<dyn ErasedBuiltInTool>,
}

impl CoreToolAdapter {
    pub fn new<T>(tool: T) -> Self
    where
        T: BuiltInTool + 'static,
    {
        Self {
            inner: Arc::new(tool),
        }
    }

    pub fn name(&self) -> &'static str {
        self.inner.name()
    }

    pub fn schema(&self) -> Value {
        self.inner.schema()
    }

    pub fn execute_erased(&self, args: Value) -> Result<Value> {
        self.inner.execute_erased(args)
    }
}

/// Type-erased version of [`BuiltInTool`] for dynamic dispatch.
///
/// Tools are registered as `Arc<dyn ErasedBuiltInTool>` and called via
/// `execute_erased` with raw `Value` arguments. The blanket impl converts
/// to the typed `Args` for each concrete [`BuiltInTool`].
pub trait ErasedBuiltInTool: Send + Sync {
    /// Tool name (must match the agent-facing schema name).
    fn name(&self) -> &'static str;
    /// JSON Schema for the tool's arguments.
    fn schema(&self) -> Value;
    /// Execute the tool with pre-deserialized JSON arguments.
    fn execute_erased(&self, args: Value) -> Result<Value>;
}


/// Structured, machine-readable tool execution outcome.
///
/// Encapsulates stdout, stderr, exit status, execution duration,
/// and truncation flags to support automated LLM error-recovery loops.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StructuredToolOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_summary: Option<String>,
}

impl StructuredToolOutput {
    pub fn success(stdout: impl Into<String>, duration_ms: u64) -> Self {
        Self {
            stdout: stdout.into(),
            stderr: String::new(),
            exit_code: 0,
            duration_ms,
            truncated: false,
            error_summary: None,
        }
    }

    pub fn failure(
        stdout: impl Into<String>,
        stderr: impl Into<String>,
        exit_code: i32,
        duration_ms: u64,
    ) -> Self {
        let stderr_str = stderr.into();
        let summary = if !stderr_str.is_empty() {
            Some(stderr_str.lines().next().unwrap_or("").to_string())
        } else {
            Some(format!("Command exited with non-zero status code: {exit_code}"))
        };

        Self {
            stdout: stdout.into(),
            stderr: stderr_str,
            exit_code,
            duration_ms,
            truncated: false,
            error_summary: summary,
        }
    }

    /// Render formatted string suitable for direct injection into LLM context prompts.
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

impl From<cade_core::shell::ShellResult> for StructuredToolOutput {
    fn from(res: cade_core::shell::ShellResult) -> Self {
        let summary = if res.exit_code != 0 {
            if !res.stderr.is_empty() {
                Some(res.stderr.lines().next().unwrap_or("").to_string())
            } else {
                Some(format!("Command exited with code {}", res.exit_code))
            }
        } else {
            None
        };

        Self {
            stdout: res.stdout,
            stderr: res.stderr,
            exit_code: res.exit_code,
            duration_ms: res.duration.as_millis() as u64,
            truncated: res.truncated,
            error_summary: summary,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_structured_tool_output_success() {
        let out = StructuredToolOutput::success("Build completed successfully", 150);
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.truncated, false);
        assert_eq!(out.error_summary, None);
        assert_eq!(out.format_for_llm(), "Build completed successfully");
    }

    #[test]
    fn test_structured_tool_output_failure() {
        let out = StructuredToolOutput::failure("Partial output", "error[E0425]: cannot find function", 1, 200);
        assert_eq!(out.exit_code, 1);
        assert_eq!(out.error_summary, Some("error[E0425]: cannot find function".to_string()));
        let formatted = out.format_for_llm();
        assert!(formatted.contains("Partial output"));
        assert!(formatted.contains("STDERR:\nerror[E0425]"));
        assert!(formatted.contains("(exit code 1)"));
    }
}

impl<T> ErasedBuiltInTool for T
where
    T: BuiltInTool + 'static,
{
    fn name(&self) -> &'static str {
        self.name()
    }

    fn schema(&self) -> Value {
        self.schema()
    }

    fn execute_erased(&self, args: Value) -> Result<Value> {
        let typed_args: T::Args = serde_json::from_value(args).map_err(|e| {
            tracing::warn!("Failed to parse arguments for tool {}: {e}", self.name());
            crate::Error::custom(format!(
                "Invalid arguments provided for tool '{}'. Please verify the tool schema.",
                self.name()
            ))
        })?;
        let result = self.execute(typed_args)?;
        Ok(serde_json::to_value(result)?)
    }
}
