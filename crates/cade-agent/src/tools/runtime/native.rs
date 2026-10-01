use super::*;
use serde_json::Value;

impl ToolRuntime {
    pub(crate) async fn handle_edit_via_backend(&self, args: &Value) -> (String, bool) {
        if !self.backend.is_writable() {
            return ("Error: backend is read-only".into(), true);
        }
        let Some(path) = args["path"].as_str().filter(|path| !path.trim().is_empty()) else {
            return ("edit_file: missing 'path'".into(), true);
        };
        let Some(old) = args["old_string"].as_str().filter(|old| !old.is_empty()) else {
            return ("edit_file: nonempty 'old_string' is required".into(), true);
        };
        let Some(new) = args["new_string"].as_str() else {
            return ("edit_file: missing 'new_string'".into(), true);
        };
        let path = std::path::Path::new(path);
        let _lock = crate::tools::file_lock::FileLockManager::global()
            .acquire_lock(path)
            .await;
        let content = match self.backend.read_file(path).await {
            Ok(content) => content,
            Err(error) => return (error.to_string(), true),
        };
        let matches = content.matches(old).count();
        let all = args["replace_all"].as_bool().unwrap_or(false);
        if matches == 0 || (matches > 1 && !all) {
            return (
                format!("edit_file: expected a unique match, found {matches}"),
                true,
            );
        }
        let changed = if all {
            content.replace(old, new)
        } else {
            content.replacen(old, new, 1)
        };
        match self.backend.write_file(path, &changed).await {
            Ok(()) => (format!("Edited {}", path.display()), false),
            Err(error) => (error.to_string(), true),
        }
    }
    pub(crate) async fn handle_bash_via_backend(&self, args: &Value) -> (String, bool) {
        let command = args["command"].as_str().unwrap_or("").to_string();
        let timeout_secs = args["timeout"].as_u64().unwrap_or(120);

        // Safety check even through non-local backends
        if !self.backend.is_writable() && cade_core::permissions::bash_command_is_write(&command) {
            return (
                format!(
                    "Blocked: read-only backend refuses write command: {}",
                    command.chars().take(80).collect::<String>()
                ),
                true,
            );
        }

        match self
            .backend
            .exec_bash(&command, &self.cwd, timeout_secs)
            .await
        {
            Ok(out) => (out.combined(), out.exit_code != 0),
            Err(e) => (format!("Backend exec failed: {e}"), true),
        }
    }

    pub(crate) async fn handle_read_via_backend(&self, args: &Value) -> (String, bool) {
        let path_str = args["path"].as_str().unwrap_or("").trim().to_string();
        if path_str.is_empty() {
            return ("Error: 'path' is required".to_string(), true);
        }
        let path = std::path::Path::new(&path_str);
        match self.backend.read_file(path).await {
            Ok(content) => {
                let offset = args["offset"].as_u64().unwrap_or(0) as usize;
                let limit = args["limit"].as_u64().unwrap_or(0) as usize;
                let lines: Vec<&str> = content.lines().collect();
                let total = lines.len();
                let end = if limit > 0 {
                    (offset + limit).min(total)
                } else {
                    total
                };
                let selected = &lines[offset.min(total)..end];
                let numbered: String = selected
                    .iter()
                    .enumerate()
                    .map(|(i, l)| format!("{:>4}→{}\n", offset + i + 1, l))
                    .collect();
                (format!("{numbered}[{total} lines total]"), false)
            }
            Err(e) => (format!("Read failed: {e}"), true),
        }
    }
}
