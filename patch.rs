            } else if tc.name == "finish_task" {
                let args: serde_json::Value = serde_json::from_str(&tc.arguments.to_string()).unwrap_or_default();
                let summary = args.get("summary").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let reason = args.get("reason").and_then(|v| v.as_str()).unwrap_or("").to_string();
                
                let output = std::process::Command::new("git")
                    .args(&["status", "--porcelain"])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_default();
                
                let files_modified = if output.trim().is_empty() {
                    "None".to_string()
                } else {
                    output.lines().map(|l| format!("- {}", l.trim())).collect::<Vec<_>>().join("\n")
                };
                
                let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                
                let log_entry = format!(
                    "\n## {} — {}\n\n**Reason:** {}\n\n**Files modified:**\n{}\n\n---\n",
                    timestamp, summary, reason, files_modified
                );
                
                let path = std::path::Path::new("CADE_AUDIT.md");
                let existing = std::fs::read_to_string(path).unwrap_or_else(|_| "# CADE Audit Log\n\n".to_string());
                let _ = std::fs::write(path, format!("{}{}", existing, log_entry));
                
                cade_agent::tools::manager::ToolResult {
                    tool_call_id: tc.id.clone(),
                    tool_name: tc.name.clone(),
                    output: format!("Task finished. Audit log appended to CADE_AUDIT.md."),
                    is_error: false,
                }
