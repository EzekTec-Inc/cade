//! F5: Structured memory — field-level patches via JSON Pointer (RFC 6901).
//!
//! Pure helpers that parse a block body as JSON, apply a pointer-based
//! operation (`set`, `append`, `remove`), and re-serialize.  No DB
//! dependency — the caller (`ToolRuntime`) fetches/upserts.

use serde_json::Value;

/// Operations supported by `apply_pointer_patch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchOp {
    /// Replace (or create) the node at the pointer with `value`.
    Set,
    /// Push `value` onto the array at the pointer.
    Append,
    /// Delete the key / splice the index at the pointer.
    Remove,
}

impl PatchOp {
    /// Parse from the string the LLM sends.
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "set" => Some(Self::Set),
            "append" => Some(Self::Append),
            "remove" => Some(Self::Remove),
            _ => None,
        }
    }
}

/// Try to parse a block body as JSON.  Returns `Err` with a
/// user-friendly message when the body is not valid JSON.
pub fn parse_block(body: &str) -> Result<Value, String> {
    serde_json::from_str(body).map_err(|e| format!("Block body is not valid JSON: {e}"))
}

/// Serialize a `Value` back to a pretty-printed JSON string.
pub fn serialize_back(val: &Value) -> String {
    serde_json::to_string_pretty(val).unwrap_or_else(|_| val.to_string())
}

/// Apply a JSON-Pointer (`/foo/bar/0`) operation to `root`.
///
/// Follows RFC 6901 pointer syntax.  For `Set`, missing intermediate
/// objects are created automatically; arrays require a valid index or
/// `-` (append).  For `Append`, the target must be an existing array.
/// For `Remove`, the target key/index must exist.
pub fn apply_pointer_patch(
    root: &mut Value,
    pointer: &str,
    op: PatchOp,
    value: Option<Value>,
) -> Result<(), String> {
    if pointer.is_empty() || pointer == "/" {
        // Root-level operations
        match op {
            PatchOp::Set => {
                let v = value.ok_or("'value' is required for set")?;
                *root = v;
                return Ok(());
            }
            PatchOp::Append => {
                return Err("Cannot append to root — target must be an array field".into());
            }
            PatchOp::Remove => {
                return Err("Cannot remove root — use update_memory(delete) instead".into());
            }
        }
    }

    // Split pointer into segments: "/a/b/0" → ["a", "b", "0"]
    let segments = parse_pointer(pointer)?;
    if segments.is_empty() {
        return Err(format!("Invalid JSON pointer: '{pointer}'"));
    }

    match op {
        PatchOp::Set => {
            let v = value.ok_or("'value' is required for set")?;
            ensure_path_and_set(root, &segments, v)
        }
        PatchOp::Append => {
            let v = value.ok_or("'value' is required for append")?;
            let target = resolve_pointer_mut(root, &segments)?;
            match target {
                Value::Array(arr) => {
                    arr.push(v);
                    Ok(())
                }
                _ => Err(format!(
                    "Pointer '{}' does not point to an array (found {})",
                    pointer,
                    kind_name(target)
                )),
            }
        }
        PatchOp::Remove => remove_at_pointer(root, &segments),
    }
}

// ── internal helpers ──────────────────────────────────────────────────────

fn kind_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Parse RFC 6901 pointer into unescaped segments.
fn parse_pointer(pointer: &str) -> Result<Vec<String>, String> {
    if !pointer.starts_with('/') {
        return Err(format!("JSON pointer must start with '/': got '{pointer}'"));
    }
    Ok(pointer[1..]
        .split('/')
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect())
}

/// Walk the tree, creating intermediate objects as needed, and set the
/// leaf to `value`.
fn ensure_path_and_set(root: &mut Value, segments: &[String], value: Value) -> Result<(), String> {
    let (parents, leaf) = segments.split_at(segments.len() - 1);

    let mut current = root;
    for seg in parents {
        current = match current {
            Value::Object(map) => {
                if !map.contains_key(seg.as_str()) {
                    map.insert(seg.clone(), Value::Object(serde_json::Map::new()));
                }
                map.get_mut(seg.as_str()).unwrap()
            }
            Value::Array(arr) => {
                let idx = parse_array_index(seg, arr.len())?;
                arr.get_mut(idx)
                    .ok_or_else(|| format!("Array index {idx} out of bounds"))?
            }
            other => {
                return Err(format!(
                    "Cannot traverse into {}: expected object or array",
                    kind_name(other)
                ));
            }
        };
    }

    let leaf_key = &leaf[0];
    match current {
        Value::Object(map) => {
            map.insert(leaf_key.clone(), value);
            Ok(())
        }
        Value::Array(arr) => {
            if leaf_key == "-" {
                arr.push(value);
                Ok(())
            } else {
                let idx = parse_array_index(leaf_key, arr.len())?;
                if idx < arr.len() {
                    arr[idx] = value;
                    Ok(())
                } else {
                    Err(format!(
                        "Array index {idx} out of bounds (len={})",
                        arr.len()
                    ))
                }
            }
        }
        other => Err(format!(
            "Cannot set field on {}: expected object or array",
            kind_name(other)
        )),
    }
}

/// Resolve a pointer to a mutable reference, returning an error if any
/// segment is missing.
fn resolve_pointer_mut<'a>(
    root: &'a mut Value,
    segments: &[String],
) -> Result<&'a mut Value, String> {
    let mut current = root;
    for seg in segments {
        current = match current {
            Value::Object(map) => map
                .get_mut(seg.as_str())
                .ok_or_else(|| format!("Key '{seg}' not found"))?,
            Value::Array(arr) => {
                let idx = parse_array_index(seg, arr.len())?;
                arr.get_mut(idx)
                    .ok_or_else(|| format!("Array index {idx} out of bounds"))?
            }
            other => {
                return Err(format!(
                    "Cannot traverse into {}: expected object or array",
                    kind_name(other)
                ));
            }
        };
    }
    Ok(current)
}

/// Remove the leaf node described by the last segment.
fn remove_at_pointer(root: &mut Value, segments: &[String]) -> Result<(), String> {
    if segments.is_empty() {
        return Err("Cannot remove root".into());
    }

    let (parents, leaf) = segments.split_at(segments.len() - 1);
    let parent = if parents.is_empty() {
        root
    } else {
        resolve_pointer_mut(root, parents)?
    };

    let leaf_key = &leaf[0];
    match parent {
        Value::Object(map) => {
            if map.remove(leaf_key.as_str()).is_some() {
                Ok(())
            } else {
                Err(format!("Key '{leaf_key}' not found — nothing to remove"))
            }
        }
        Value::Array(arr) => {
            let idx = parse_array_index(leaf_key, arr.len())?;
            if idx < arr.len() {
                arr.remove(idx);
                Ok(())
            } else {
                Err(format!(
                    "Array index {idx} out of bounds (len={})",
                    arr.len()
                ))
            }
        }
        other => Err(format!(
            "Cannot remove from {}: expected object or array",
            kind_name(other)
        )),
    }
}

fn parse_array_index(seg: &str, _len: usize) -> Result<usize, String> {
    seg.parse::<usize>()
        .map_err(|_| format!("'{seg}' is not a valid array index"))
}

// ── tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn f5_set_scalar_field() {
        let mut root = json!({"status": "running", "count": 0});
        apply_pointer_patch(&mut root, "/status", PatchOp::Set, Some(json!("done"))).unwrap();
        assert_eq!(root["status"], json!("done"));
        // count unchanged
        assert_eq!(root["count"], json!(0));
    }

    #[test]
    fn f5_set_nested_field() {
        let mut root = json!({"a": {"b": 1}});
        apply_pointer_patch(&mut root, "/a/b", PatchOp::Set, Some(json!(42))).unwrap();
        assert_eq!(root["a"]["b"], json!(42));
    }

    #[test]
    fn f5_creates_missing_intermediate_object() {
        let mut root = json!({"existing": true});
        apply_pointer_patch(
            &mut root,
            "/deep/nested/key",
            PatchOp::Set,
            Some(json!("hello")),
        )
        .unwrap();
        assert_eq!(root["deep"]["nested"]["key"], json!("hello"));
        // original key untouched
        assert_eq!(root["existing"], json!(true));
    }

    #[test]
    fn f5_append_to_array() {
        let mut root = json!({"items": [1, 2]});
        apply_pointer_patch(&mut root, "/items", PatchOp::Append, Some(json!(3))).unwrap();
        assert_eq!(root["items"], json!([1, 2, 3]));
    }

    #[test]
    fn f5_append_rejects_non_array() {
        let mut root = json!({"name": "test"});
        let err =
            apply_pointer_patch(&mut root, "/name", PatchOp::Append, Some(json!("x"))).unwrap_err();
        assert!(err.contains("not point to an array"), "got: {err}");
    }

    #[test]
    fn f5_remove_key() {
        let mut root = json!({"keep": 1, "drop": 2});
        apply_pointer_patch(&mut root, "/drop", PatchOp::Remove, None).unwrap();
        assert_eq!(root, json!({"keep": 1}));
    }

    #[test]
    fn f5_remove_array_index_splices() {
        let mut root = json!({"arr": ["a", "b", "c"]});
        apply_pointer_patch(&mut root, "/arr/1", PatchOp::Remove, None).unwrap();
        assert_eq!(root["arr"], json!(["a", "c"]));
    }

    #[test]
    fn f5_rejects_unstructured_block() {
        let result = parse_block("This is just plain text, not JSON.");
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("not valid JSON"),
            "should mention invalid JSON"
        );
    }

    #[test]
    fn f5_round_trips_json_format() {
        let original = json!({"a": 1, "b": [2, 3]});
        let serialized = serialize_back(&original);
        let reparsed: Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(original, reparsed);
    }

    #[test]
    fn f5_invalid_pointer_returns_error() {
        let mut root = json!({"a": 1});
        // Missing leading slash
        let err =
            apply_pointer_patch(&mut root, "no_slash", PatchOp::Set, Some(json!(1))).unwrap_err();
        assert!(err.contains("must start with '/'"), "got: {err}");
    }

    #[test]
    fn f5_remove_nonexistent_key_errors() {
        let mut root = json!({"a": 1});
        let err = apply_pointer_patch(&mut root, "/missing", PatchOp::Remove, None).unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
    }

    #[test]
    fn f5_set_array_element_by_index() {
        let mut root = json!({"arr": [10, 20, 30]});
        apply_pointer_patch(&mut root, "/arr/1", PatchOp::Set, Some(json!(99))).unwrap();
        assert_eq!(root["arr"], json!([10, 99, 30]));
    }
}
