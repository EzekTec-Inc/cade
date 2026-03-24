/// Surface adapter: maps canonical tool IDs to provider/model-specific names
/// and back.
///
/// The LLM sees different tool names depending on the model family
/// (e.g. Gemini expects "RunShellCommand" instead of "bash").  This adapter
/// handles that translation in one place so all other CADE code can use the
/// canonical names from [`crate::tool_ids`].
use std::collections::HashMap;

use super::Toolset;

// region:    --- ToolSurfaceAdapter

/// Bidirectional name mapping for one [`Toolset`].
///
/// For `Toolset::Default` and `Toolset::Codex` all canonical names are used
/// as-is (no translation needed), so those instances have empty maps.
#[derive(Clone, Debug, Default)]
pub struct ToolSurfaceAdapter {
    /// canonical_id → external_name (for outbound schema serialization)
    forward: HashMap<&'static str, &'static str>,
    /// external_name → canonical_id (for inbound dispatch lookup)
    reverse: HashMap<&'static str, &'static str>,
}

impl ToolSurfaceAdapter {
    // -- Constructors

    /// Build an adapter for the given toolset.
    pub fn for_toolset(ts: Toolset) -> Self {
        use crate::tool_ids::*;

        let pairs: &[(&'static str, &'static str)] = match ts {
            Toolset::Gemini => &[
                (BASH,       "RunShellCommand"),
                (READ_FILE,  "ReadFileGemini"),
                (WRITE_FILE, "WriteFileGemini"),
                (EDIT_FILE,  "Replace"),
                (GREP,       "SearchFileContent"),
                (GLOB,       "GlobGemini"),
            ],
            // Default and Codex use canonical names — no translation needed.
            _ => &[],
        };

        let mut forward = HashMap::new();
        let mut reverse = HashMap::new();
        for &(canonical, external) in pairs {
            forward.insert(canonical, external);
            reverse.insert(external, canonical);
        }
        Self { forward, reverse }
    }

    // -- Methods

    /// Translate a canonical ID to the external name this toolset uses.
    /// Returns the canonical name unchanged when no translation is registered.
    pub fn to_external<'a>(&'a self, canonical: &'a str) -> &'a str {
        self.forward.get(canonical).copied().unwrap_or(canonical)
    }

    /// Translate an external name (as received from the LLM) back to the
    /// canonical ID.  Returns the name unchanged when no translation is
    /// registered (works correctly for toolsets with no aliases).
    pub fn to_canonical<'a>(&'a self, external: &'a str) -> &'a str {
        self.reverse.get(external).copied().unwrap_or(external)
    }

    /// Apply the forward translation to a JSON tool schema's `"name"` field.
    /// Returns the schema unchanged if no translation is registered for it.
    pub fn translate_schema(&self, mut schema: serde_json::Value) -> serde_json::Value {
        if let Some(name) = schema["name"].as_str() {
            let translated = self.to_external(name);
            if translated != name {
                schema["name"] = serde_json::Value::String(translated.to_string());
            }
        }
        schema
    }
}

// endregion: --- ToolSurfaceAdapter

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adapter_default_identity() {
        // -- Setup & Fixtures
        let adapter = ToolSurfaceAdapter::for_toolset(Toolset::Default);

        // -- Check
        assert_eq!(adapter.to_external("bash"), "bash");
        assert_eq!(adapter.to_canonical("bash"), "bash");
    }

    #[test]
    fn test_adapter_gemini_forward() {
        // -- Setup & Fixtures
        let adapter = ToolSurfaceAdapter::for_toolset(Toolset::Gemini);

        // -- Check
        assert_eq!(adapter.to_external("bash"),       "RunShellCommand");
        assert_eq!(adapter.to_external("read_file"),  "ReadFileGemini");
        assert_eq!(adapter.to_external("edit_file"),  "Replace");
        assert_eq!(adapter.to_external("grep"),       "SearchFileContent");
    }

    #[test]
    fn test_adapter_gemini_reverse() {
        // -- Setup & Fixtures
        let adapter = ToolSurfaceAdapter::for_toolset(Toolset::Gemini);

        // -- Check
        assert_eq!(adapter.to_canonical("RunShellCommand"),  "bash");
        assert_eq!(adapter.to_canonical("ReadFileGemini"),   "read_file");
        assert_eq!(adapter.to_canonical("Replace"),          "edit_file");
        assert_eq!(adapter.to_canonical("SearchFileContent"),"grep");
    }

    #[test]
    fn test_adapter_unknown_passthrough() {
        // -- Setup & Fixtures
        let adapter = ToolSurfaceAdapter::for_toolset(Toolset::Gemini);

        // -- Check — tools with no alias pass through unchanged
        assert_eq!(adapter.to_external("desktop_screenshot"), "desktop_screenshot");
        assert_eq!(adapter.to_canonical("desktop_notify"),    "desktop_notify");
    }

    #[test]
    fn test_adapter_translate_schema() {
        use serde_json::json;
        // -- Setup & Fixtures
        let adapter = ToolSurfaceAdapter::for_toolset(Toolset::Gemini);
        let schema = json!({ "name": "bash", "description": "run shell", "parameters": {} });

        // -- Exec
        let translated = adapter.translate_schema(schema);

        // -- Check
        assert_eq!(translated["name"].as_str(), Some("RunShellCommand"));
        assert_eq!(translated["description"].as_str(), Some("run shell"));
    }
}

// endregion: --- Tests
