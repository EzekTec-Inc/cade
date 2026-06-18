//! Strongly-typed interfaces for CADE's compile-time local tools.
//!
//! Provides traits and runtime adapter structures to unify compile-time checked
//! local Rust implementations alongside CADE's dynamic Model Context Protocol (MCP) loops.

use crate::Result;
use serde_json::Value;
use std::sync::Arc;

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
