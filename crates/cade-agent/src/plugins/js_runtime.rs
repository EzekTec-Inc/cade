//! JavaScript/TypeScript plugin runtime powered by `rquickjs`.
//!
//! Initializes a sandboxed QuickJS context and exposes CADE's built-in
//! tools (filesystem, prompt, ask_user) as native JS functions so that
//! third-party plugins can be authored in JavaScript or TypeScript.

use crate::Result;
use rquickjs::{Context, Function, Object, Runtime, Value};

/// A sandboxed JavaScript runtime for executing CADE plugins.
pub struct JsRuntime {
    runtime: Runtime,
    ctx: Context,
}

impl JsRuntime {
    /// Create a new sandboxed JS runtime with CADE's built-in tools exposed.
    pub fn new() -> Result<Self> {
        let runtime = Runtime::new()
            .map_err(|e| crate::Error::custom(format!("Failed to create QuickJS runtime: {e}")))?;

        // Limit resource usage for safety
        runtime.set_memory_limit(32 * 1024 * 1024); // 32 MB
        runtime.set_max_stack_size(256 * 1024); // 256 KB
        runtime.set_gc_threshold(4096);

        let ctx = Context::full(&runtime)
            .map_err(|e| crate::Error::custom(format!("Failed to create QuickJS context: {e}")))?;

        let mut instance = Self { runtime, ctx };

        // Expose built-in CADE tools to the JS environment
        instance.expose_tools()?;

        Ok(instance)
    }

    /// Evaluate JavaScript source code in the sandboxed context.
    ///
    /// Returns the result of the last expression as a string.
    pub fn eval(&self, source: &str) -> Result<String> {
        self.ctx.with(|ctx| -> Result<String> {
            let value: Value = ctx
                .eval(source)
                .map_err(|e| crate::Error::custom(format!("JS eval error: {e}")))?;
            // Convert the result to a string representation
            if value.is_undefined() || value.is_null() {
                Ok(String::new())
            } else if let Some(s) = rquickjs::Coerced::<String>::from_value(value.clone()).ok() {
                Ok(s.0)
            } else {
                Ok(format!("{value:?}"))
            }
        })
    }

    /// Call a named function exported by the evaluated script.
    pub fn call(&self, name: &str, args: &[&dyn JsArg]) -> Result<String> {
        self.ctx.with(|ctx| -> Result<String> {
            let global = ctx.globals();
            let func: Function = global.get(name).map_err(|e| {
                crate::Error::custom(format!("JS function '{name}' not found: {e}"))
            })?;

            let js_args: Vec<Value> = args
                .iter()
                .map(|a| a.to_js_value(&ctx))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| crate::Error::custom(format!("JS arg conversion error: {e}")))?;

            let result: Value = func
                .call::<_, Value>(&js_args)
                .map_err(|e| crate::Error::custom(format!("JS call '{name}' error: {e}")))?;

            if result.is_undefined() || result.is_null() {
                Ok(String::new())
            } else if let Some(s) = rquickjs::Coerced::<String>::from_value(result).ok() {
                Ok(s.0)
            } else {
                Ok(format!("{result:?}"))
            }
        })
    }

    /// Expose CADE's built-in tools as JS functions on the global object.
    fn expose_tools(&self) -> Result<()> {
        self.ctx.with(|ctx| -> Result<()> {
            let global = ctx.globals();

            // -- read_file(path: string) → string
            let read_file_fn = Function::new(ctx.clone(), |path: String| -> Result<String> {
                std::fs::read_to_string(&path)
                    .map_err(|e| rquickjs::Error::new_from_js(&e.to_string()))
            })?;
            global.set("read_file", read_file_fn)?;

            // -- write_file(path: string, content: string) → string
            let write_file_fn = Function::new(
                ctx.clone(),
                |path: String, content: String| -> Result<String> {
                    std::fs::write(&path, &content)
                        .map(|_| format!("Wrote {} bytes to {}", content.len(), path))
                        .map_err(|e| rquickjs::Error::new_from_js(&e.to_string()))
                },
            )?;
            global.set("write_file", write_file_fn)?;

            // -- glob(pattern: string) → string (JSON array)
            let glob_fn = Function::new(ctx.clone(), |pattern: String| -> Result<String> {
                let entries: Vec<String> = glob::glob(&pattern)
                    .map_err(|e| rquickjs::Error::new_from_js(&e.to_string()))?
                    .filter_map(|e| e.ok().map(|p| p.to_string_lossy().to_string()))
                    .collect();
                Ok(serde_json::to_string(&entries).unwrap_or_default())
            })?;
            global.set("glob", glob_fn)?;

            // -- prompt(message: string) → string
            let prompt_fn = Function::new(ctx.clone(), |_msg: String| -> Result<String> {
                // Delegated to CADE's ask_user — returns empty for headless
                Ok(String::new())
            })?;
            global.set("prompt", prompt_fn)?;

            Ok(())
        })
    }
}

/// Trait for converting Rust values into JavaScript arguments.
pub trait JsArg {
    fn to_js_value(&self, ctx: &Context) -> std::result::Result<Value, rquickjs::Error>;
}

impl JsArg for str {
    fn to_js_value(&self, ctx: &Context) -> std::result::Result<Value, rquickjs::Error> {
        Ok(Value::new_string(ctx, self)?.into_value())
    }
}

impl JsArg for String {
    fn to_js_value(&self, ctx: &Context) -> std::result::Result<Value, rquickjs::Error> {
        (self as &str).to_js_value(ctx)
    }
}

impl JsArg for i32 {
    fn to_js_value(&self, ctx: &Context) -> std::result::Result<Value, rquickjs::Error> {
        Ok(Value::new_int(ctx, *self))
    }
}

impl JsArg for f64 {
    fn to_js_value(&self, ctx: &Context) -> std::result::Result<Value, rquickjs::Error> {
        Ok(Value::new_float(ctx, *self))
    }
}

impl JsArg for bool {
    fn to_js_value(&self, ctx: &Context) -> std::result::Result<Value, rquickjs::Error> {
        Ok(Value::new_bool(ctx, *self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_runtime_creation() {
        let rt = JsRuntime::new();
        assert!(rt.is_ok(), "JS runtime should create successfully");
    }

    #[test]
    fn js_simple_eval() {
        let rt = JsRuntime::new().unwrap();
        let result = rt.eval("1 + 2").unwrap();
        assert_eq!(result, "3");
    }

    #[test]
    fn js_function_call() {
        let rt = JsRuntime::new().unwrap();

        // Define a simple function and call it
        rt.eval("function add(a, b) { return a + b; }").unwrap();

        let result = rt.call("add", &[&1i32, &2i32]).unwrap();
        assert_eq!(result, "3");
    }

    #[test]
    fn js_glob_tool_exposed() {
        let rt = JsRuntime::new().unwrap();
        // The glob function should be on the global object
        let result = rt.eval("typeof glob").unwrap();
        assert_eq!(result, "function");
    }

    #[test]
    fn js_read_file_tool_exposed() {
        let rt = JsRuntime::new().unwrap();
        let result = rt.eval("typeof read_file").unwrap();
        assert_eq!(result, "function");
    }
}
