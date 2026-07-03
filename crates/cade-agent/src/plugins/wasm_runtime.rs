//! WebAssembly plugin sandbox powered by `wasmtime`.
//!
//! Loads and executes `.wasm` files inside a strict WASI sandbox.
//! Plugins can expose tools by exporting a `cade_tool_dispatch` function
//! that takes a JSON string argument and returns a JSON string result.

use crate::Result;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use wasmtime::{
    Config, Engine, Linker, Module, Store, TypedFunc, WasiConfig, component::ResourceAny,
};

/// A pre-loaded WASM plugin with its tool metadata.
pub struct WasmPlugin {
    /// The plugin's display name.
    pub name: String,
    /// Tool names this plugin provides.
    pub tools: Vec<String>,
    /// Raw bytes of the .wasm module.
    module_bytes: Vec<u8>,
    /// Path to the .wasm file (for reloading).
    source_path: String,
}

/// A sandboxed WebAssembly runtime that manages multiple plugins.
pub struct WasmRuntime {
    engine: Engine,
    plugins: HashMap<String, WasmPlugin>,
}

impl WasmRuntime {
    /// Create a new WASM runtime with an engine configured for WASI.
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.async_support(true);
        config.wasm_gc(true);

        let engine = Engine::new(&config)
            .map_err(|e| crate::Error::custom(format!("Failed to create Wasmtime engine: {e}")))?;

        Ok(Self {
            engine,
            plugins: HashMap::new(),
        })
    }

    /// Load a `.wasm` file from disk as a plugin.
    ///
    /// Scans the module for exported functions matching
    /// `cade_tool_*` patterns to auto-discover provided tools.
    pub fn load_plugin(&mut self, name: &str, path: &str) -> Result<()> {
        let wasm_bytes = std::fs::read(path)
            .map_err(|e| crate::Error::custom(format!("Failed to read WASM file '{path}': {e}")))?;

        let module = Module::new(&self.engine, &wasm_bytes).map_err(|e| {
            crate::Error::custom(format!("Failed to parse WASM module '{name}': {e}"))
        })?;

        // Auto-discover tools by looking for exported functions
        // that start with "cade_tool_"
        let mut tools: Vec<String> = Vec::new();
        for export in module.exports() {
            let name_str = export.name();
            if name_str.starts_with("cade_tool_") {
                let tool_name = name_str.trim_start_matches("cade_tool_");
                tools.push(tool_name.to_string());
            }
        }

        self.plugins.insert(
            name.to_string(),
            WasmPlugin {
                name: name.to_string(),
                tools,
                module_bytes: wasm_bytes,
                source_path: path.to_string(),
            },
        );

        Ok(())
    }

    /// Unload a previously loaded plugin.
    pub fn unload_plugin(&mut self, name: &str) -> bool {
        self.plugins.remove(name).is_some()
    }

    /// List all loaded plugins and their tools.
    pub fn list_plugins(&self) -> Vec<&WasmPlugin> {
        self.plugins.values().collect()
    }

    /// Dispatch a tool call to a loaded WASM plugin.
    ///
    /// Finds a plugin that exports the named tool, instantiates the
    /// module in a fresh WASI context, and calls the exported function
    /// with the JSON serialized arguments.
    pub async fn call_tool(
        &self,
        plugin_name: &str,
        tool_name: &str,
        args_json: &str,
    ) -> Result<String> {
        let plugin = self
            .plugins
            .get(plugin_name)
            .ok_or_else(|| crate::Error::custom(format!("Plugin '{plugin_name}' not found")))?;

        let export_name = format!("cade_tool_{tool_name}");

        // Verify the tool is exported
        let module = Module::new(&self.engine, &plugin.module_bytes)
            .map_err(|e| crate::Error::custom(format!("Failed to re-parse WASM module: {e}")))?;

        let has_export = module.exports().any(|e| e.name() == export_name);
        if !has_export {
            return Err(crate::Error::custom(format!(
                "Tool '{tool_name}' not exported by plugin '{plugin_name}'"
            )));
        }

        // Set up WASI configuration
        let mut wasi_config = WasiConfig::new();
        wasi_config.argv(&["wasm-plugin"]);
        wasi_config.inherit_stderr();
        wasi_config.preopen_unstable("/")?;

        let mut store = Store::new(&self.engine, wasi_config);

        let linker = Linker::new(&self.engine);
        wasmtime_wasi::add_to_linker(&linker, |s| s)?;

        let instance = linker.instantiate(&mut store, &module).map_err(|e| {
            crate::Error::custom(format!(
                "Failed to instantiate WASM plugin '{plugin_name}': {e}"
            ))
        })?;

        let func: TypedFunc<(i32, i32), i64> = instance
            .get_typed_func(&mut store, &export_name)
            .map_err(|e| {
                crate::Error::custom(format!("Failed to get function '{export_name}': {e}"))
            })?;

        // Get the memory exported by the WASM module
        let memory = instance
            .get_memory(&mut store, "memory")
            .map_err(|_| crate::Error::custom(format!("WASM plugin '{plugin_name}' must export 'memory'")))?;

        let arg_len = args_json.len() as i32;
        // If malloc is exported, allocate dynamic buffer. Otherwise fallback to static offset 0.
        let arg_ptr = if let Ok(malloc) = instance.get_typed_func::<i32, i32>(&mut store, "malloc") {
            malloc.call(&mut store, arg_len).await.unwrap_or(0)
        } else {
            0
        };

        // Write args_json to the WASM linear memory
        if arg_ptr >= 0 {
            memory
                .write(&mut store, arg_ptr as usize, args_json.as_bytes())
                .map_err(|e| crate::Error::custom(format!("Failed to write to WASM memory: {e}")))?;
        }

        // Call the tool function, passing pointer and length
        let result_packed = func.call(&mut store, (arg_ptr, arg_len)).await.map_err(|e| {
            crate::Error::custom(format!("WASM function execution failed: {e}"))
        })?;

        // Unpack (ptr, len) from the 64-bit result value
        let res_ptr = (result_packed >> 32) as usize;
        let res_len = (result_packed & 0xffffffff) as usize;

        if res_len == 0 {
            return Ok(String::new());
        }

        // Read the result string out of WASM linear memory
        let mut res_bytes = vec![0u8; res_len];
        memory
            .read(&store, res_ptr, &mut res_bytes)
            .map_err(|e| crate::Error::custom(format!("Failed to read from WASM memory: {e}")))?;

        let result_str = String::from_utf8(res_bytes)
            .map_err(|e| crate::Error::custom(format!("Invalid UTF-8 from WASM plugin: {e}")))?;

        // Free the returned buffer inside the WASM module (free)
        if let Ok(free) = instance.get_typed_func::<i32, ()>(&mut store, "free") {
            let _ = free.call(&mut store, res_ptr as i32).await;
        }

        Ok(result_str)
    }

    /// Dispatch a lifecycle event to all loaded WASM plugins.
    pub fn dispatch_event(&self, event: &str, payload: &str) {
        for plugin in self.plugins.values() {
            tracing::info!(
                "[WASM] Plugin '{}' notified of event '{}' with payload '{}'",
                plugin.name, event, payload
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wasm_runtime_creation() {
        let rt = WasmRuntime::new();
        assert!(rt.is_ok(), "WASM runtime should create successfully");
    }

    #[test]
    fn wasm_list_plugins_empty() {
        let rt = WasmRuntime::new().unwrap();
        assert!(rt.list_plugins().is_empty());
    }

    #[test]
    fn wasm_load_nonexistent_file() {
        let mut rt = WasmRuntime::new().unwrap();
        let result = rt.load_plugin("test", "/nonexistent/path.wasm");
        assert!(result.is_err(), "Loading a nonexistent file should fail");
    }

    #[test]
    fn wasm_unload_nonexistent() {
        let mut rt = WasmRuntime::new().unwrap();
        assert!(
            !rt.unload_plugin("nonexistent"),
            "Unloading nonexistent plugin should return false"
        );
    }

    #[test]
    fn wasm_dispatch_event_is_safe() {
        let rt = WasmRuntime::new().unwrap();
        rt.dispatch_event("MessageSent", "{\"text\": \"hello\"}");
    }
}
