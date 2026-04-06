//! Subagent output evaluator — heuristic + optional LLM verification.
//!
//! Sits between `run_headless()` returning and the output being merged
//! into the parent agent's context. Catches hallucinations, empty output,
//! constraint violations, and malformed code before they pollute the
//! main conversation.

// -- EvalVerdict

/// Result of evaluating a subagent's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalVerdict {
    /// Output passes all checks — merge into parent context.
    Accept,
    /// Output failed a check — retry with feedback.
    Retry { feedback: String, attempt: u8 },
    /// Max retries exceeded — return error to parent.
    Reject { reason: String },
}

impl EvalVerdict {
    pub fn is_accept(&self) -> bool {
        matches!(self, Self::Accept)
    }
    pub fn is_retry(&self) -> bool {
        matches!(self, Self::Retry { .. })
    }
    pub fn is_reject(&self) -> bool {
        matches!(self, Self::Reject { .. })
    }
}

/// Default maximum retry attempts before rejecting.
pub const DEFAULT_MAX_RETRIES: u8 = 2;

// -- Heuristic checks

/// Check if the output is empty or an obvious error.
fn check_empty_or_error(output: &str) -> Option<String> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Some("subagent returned empty output".to_string());
    }
    // Common error prefixes from subagent execution
    if trimmed.starts_with("Subagent error:") || trimmed.starts_with("error:") {
        return Some(format!(
            "subagent returned an error: {}",
            trimmed.chars().take(120).collect::<String>()
        ));
    }
    None
}

/// Check for hallucinated Rust crate imports.
///
/// LLMs commonly hallucinate crate names that don't exist. We scan for
/// `use <crate>::` patterns and flag known-bad ones.
fn check_hallucinated_crates(output: &str) -> Option<String> {
    // Known hallucinated crates that LLMs frequently invent
    const HALLUCINATED: &[&str] = &[
        "use phantom_crate::",
        "use fake_utils::",
        "use rust_helpers::",
        "use crate_that_doesnt_exist::",
        "use ai_generated::",
        "use llm_utils::",
        "use magic_lib::",
    ];

    let lower = output.to_lowercase();
    for pattern in HALLUCINATED {
        if lower.contains(pattern) {
            return Some(format!("hallucinated crate import detected: {pattern}"));
        }
    }
    None
}

/// Basic bracket-balance check for Rust code blocks.
///
/// If the output contains `fn ` or `struct `, we expect curly braces
/// to be balanced. Unbalanced braces indicate truncated or malformed code.
fn check_bracket_balance(output: &str) -> Option<String> {
    // Only check if the output looks like it contains Rust code
    let has_rust = output.contains("fn ") || output.contains("struct ") || output.contains("impl ");
    if !has_rust {
        return None;
    }

    // Extract code blocks (between ``` markers) or check the whole output
    let code_sections: Vec<&str> = if output.contains("```rust") || output.contains("```rs") {
        output
            .split("```")
            .enumerate()
            .filter_map(|(i, s)| {
                // Odd-indexed sections are inside ``` blocks
                if i % 2 == 1 {
                    // Strip the language tag from the first line
                    let code = s.strip_prefix("rust").or_else(|| s.strip_prefix("rs")).unwrap_or(s);
                    Some(code)
                } else {
                    None
                }
            })
            .collect()
    } else {
        // No markdown code blocks — check the whole output
        vec![output]
    };

    for code in code_sections {
        let mut depth: i32 = 0;
        for ch in code.chars() {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            if depth < 0 {
                return Some("malformed Rust code: unbalanced braces (extra closing brace)".to_string());
            }
        }
        if depth != 0 {
            return Some(format!(
                "malformed Rust code: unbalanced braces ({depth} unclosed)"
            ));
        }
    }

    None
}

/// Check for constraint violations based on the task prompt.
///
/// If the task prompt says "read-only" or "do not modify", the output
/// should not contain evidence of write tool calls.
fn check_constraint_violation(output: &str, task_prompt: &str) -> Option<String> {
    let prompt_lower = task_prompt.to_lowercase();
    let is_readonly = prompt_lower.contains("read-only")
        || prompt_lower.contains("read only")
        || prompt_lower.contains("do not modify")
        || prompt_lower.contains("don't modify")
        || prompt_lower.contains("no modifications");

    if !is_readonly {
        return None;
    }

    // Check if the output contains evidence of write operations
    let output_lower = output.to_lowercase();
    let write_indicators = [
        "write_file",
        "edit_file",
        "apply_patch",
        "created file",
        "modified file",
        "wrote to",
    ];

    for indicator in &write_indicators {
        if output_lower.contains(indicator) {
            return Some(format!(
                "constraint violation: task was read-only but output contains '{indicator}'"
            ));
        }
    }

    None
}

// -- Main evaluator

/// Evaluate subagent output using heuristic checks.
///
/// Returns `Accept` if all checks pass, `Retry` with feedback if a check
/// fails and retries remain, or `Reject` if max retries are exceeded.
pub fn evaluate_subagent_output(
    output: &str,
    task_prompt: &str,
    max_retries: u8,
    current_attempt: u8,
) -> EvalVerdict {
    // Run all heuristic checks in priority order
    let checks: &[fn(&str, &str) -> Option<String>] = &[
        |o, _| check_empty_or_error(o),
        |o, _| check_hallucinated_crates(o),
        |o, _| check_bracket_balance(o),
        check_constraint_violation,
    ];

    for check in checks {
        if let Some(failure) = check(output, task_prompt) {
            if current_attempt >= max_retries {
                return EvalVerdict::Reject {
                    reason: format!(
                        "max retries ({max_retries}) exceeded — last failure: {failure}"
                    ),
                };
            }
            return EvalVerdict::Retry {
                feedback: failure,
                attempt: current_attempt + 1,
            };
        }
    }

    EvalVerdict::Accept
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    // -- EvalVerdict

    #[test]
    fn verdict_accessors() {
        assert!(EvalVerdict::Accept.is_accept());
        assert!(!EvalVerdict::Accept.is_retry());
        assert!(!EvalVerdict::Accept.is_reject());

        let retry = EvalVerdict::Retry {
            feedback: "bad".into(),
            attempt: 1,
        };
        assert!(!retry.is_accept());
        assert!(retry.is_retry());
        assert!(!retry.is_reject());

        let reject = EvalVerdict::Reject {
            reason: "done".into(),
        };
        assert!(!reject.is_accept());
        assert!(!reject.is_retry());
        assert!(reject.is_reject());
    }

    // -- check_empty_or_error

    #[test]
    fn empty_output_fails() {
        let v = evaluate_subagent_output("", "do something", 2, 0);
        assert!(v.is_retry());
        if let EvalVerdict::Retry { feedback, .. } = v {
            assert!(feedback.contains("empty"), "got: {feedback}");
        }
    }

    #[test]
    fn whitespace_only_output_fails() {
        let v = evaluate_subagent_output("   \n\t  ", "do something", 2, 0);
        assert!(v.is_retry());
    }

    #[test]
    fn error_prefix_output_fails() {
        let v = evaluate_subagent_output(
            "Subagent error: model not found",
            "do something",
            2,
            0,
        );
        assert!(v.is_retry());
        if let EvalVerdict::Retry { feedback, .. } = v {
            assert!(feedback.contains("error"), "got: {feedback}");
        }
    }

    #[test]
    fn valid_output_passes() {
        let v = evaluate_subagent_output(
            "I found 3 files matching the pattern.",
            "search for config files",
            2,
            0,
        );
        assert!(v.is_accept());
    }

    // -- check_hallucinated_crates

    #[test]
    fn hallucinated_crate_detected() {
        let output = "Here's the fix:\n```rust\nuse phantom_crate::Widget;\nfn main() {}\n```";
        let v = evaluate_subagent_output(output, "fix the bug", 2, 0);
        assert!(v.is_retry());
        if let EvalVerdict::Retry { feedback, .. } = v {
            assert!(feedback.contains("hallucinated"), "got: {feedback}");
        }
    }

    #[test]
    fn real_crate_not_flagged() {
        let output = "```rust\nuse serde::Serialize;\nuse tokio::sync::mpsc;\n```";
        let v = evaluate_subagent_output(output, "add serialization", 2, 0);
        assert!(v.is_accept());
    }

    // -- check_bracket_balance

    #[test]
    fn unbalanced_braces_detected() {
        let output = "```rust\nfn main() {\n    println!(\"hello\");\n```";
        let v = evaluate_subagent_output(output, "write a function", 2, 0);
        assert!(v.is_retry());
        if let EvalVerdict::Retry { feedback, .. } = v {
            assert!(feedback.contains("unbalanced"), "got: {feedback}");
        }
    }

    #[test]
    fn balanced_braces_pass() {
        let output = "```rust\nfn main() {\n    println!(\"hello\");\n}\n```";
        let v = evaluate_subagent_output(output, "write a function", 2, 0);
        assert!(v.is_accept());
    }

    #[test]
    fn non_rust_output_skips_bracket_check() {
        // Unbalanced braces in non-Rust output should not trigger
        let output = "The JSON looks like: { \"key\": \"value\"";
        let v = evaluate_subagent_output(output, "check the json", 2, 0);
        assert!(v.is_accept());
    }

    // -- check_constraint_violation

    #[test]
    fn readonly_constraint_catches_write() {
        let output = "I used write_file to create the new module.";
        let v = evaluate_subagent_output(output, "read-only: analyze the code", 2, 0);
        assert!(v.is_retry());
        if let EvalVerdict::Retry { feedback, .. } = v {
            assert!(feedback.contains("constraint"), "got: {feedback}");
        }
    }

    #[test]
    fn readonly_constraint_allows_reads() {
        let output = "I found the function at src/main.rs:42.";
        let v = evaluate_subagent_output(output, "read-only: find the entry point", 2, 0);
        assert!(v.is_accept());
    }

    #[test]
    fn non_readonly_allows_writes() {
        let output = "I used write_file to create the module.";
        let v = evaluate_subagent_output(output, "create a new module", 2, 0);
        assert!(v.is_accept());
    }

    // -- Retry / Reject lifecycle

    #[test]
    fn first_failure_retries() {
        let v = evaluate_subagent_output("", "task", 2, 0);
        assert_eq!(
            v,
            EvalVerdict::Retry {
                feedback: "subagent returned empty output".to_string(),
                attempt: 1,
            }
        );
    }

    #[test]
    fn second_failure_retries() {
        let v = evaluate_subagent_output("", "task", 2, 1);
        assert_eq!(
            v,
            EvalVerdict::Retry {
                feedback: "subagent returned empty output".to_string(),
                attempt: 2,
            }
        );
    }

    #[test]
    fn exceeding_max_retries_rejects() {
        let v = evaluate_subagent_output("", "task", 2, 2);
        assert!(v.is_reject());
        if let EvalVerdict::Reject { reason } = v {
            assert!(reason.contains("max retries"), "got: {reason}");
        }
    }

    #[test]
    fn zero_max_retries_rejects_immediately() {
        let v = evaluate_subagent_output("", "task", 0, 0);
        assert!(v.is_reject());
    }
}

// endregion: --- Tests
