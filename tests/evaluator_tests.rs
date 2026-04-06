#![allow(clippy::empty_line_after_doc_comments)]
/// Integration tests for the subagent evaluator (HMAS Evaluator Pattern).
///
/// These tests verify that the evaluator catches common subagent failure
/// modes and drives the retry lifecycle correctly without needing a live
/// LLM or server.

#[cfg(test)]
mod evaluator_heuristic_tests {
    use cade::subagents::evaluator::{
        evaluate_subagent_output, EvalVerdict, DEFAULT_MAX_RETRIES,
    };

    /// Simulates a subagent that returns completely empty output (model failure).
    /// The evaluator must catch this and request a retry.
    #[test]
    fn catches_empty_subagent_output() {
        let output = "";
        let task = "Analyze the imports in crates/cade-core/src/permissions/mod.rs";

        let verdict = evaluate_subagent_output(output, task, DEFAULT_MAX_RETRIES, 0);
        assert!(
            verdict.is_retry(),
            "empty output should trigger retry, got: {verdict:?}"
        );
    }

    /// Simulates a subagent that hallucinates a Rust crate.
    /// The evaluator must catch the phantom import and retry.
    #[test]
    fn catches_hallucinated_dependency() {
        let output = r#"
Here's the refactored code:

```rust
use phantom_crate::Widget;
use serde::Serialize;

#[derive(Serialize)]
struct Config {
    name: String,
}
```

I added the `phantom_crate` dependency for better widget handling.
"#;
        let task = "Refactor the Config struct to use serde";

        let verdict = evaluate_subagent_output(output, task, DEFAULT_MAX_RETRIES, 0);
        assert!(
            verdict.is_retry(),
            "hallucinated crate should trigger retry, got: {verdict:?}"
        );
    }

    /// Simulates a subagent that returns truncated/malformed Rust code.
    /// The evaluator must detect unbalanced braces and retry.
    #[test]
    fn catches_malformed_rust_output() {
        let output = r#"
Here's the implementation:

```rust
fn process_items(items: &[Item]) -> Result<Vec<Output>> {
    let mut results = Vec::new();
    for item in items {
        let output = transform(item)?;
        results.push(output);
    // Missing closing braces — truncated response
```
"#;
        let task = "Implement the process_items function";

        let verdict = evaluate_subagent_output(output, task, DEFAULT_MAX_RETRIES, 0);
        assert!(
            verdict.is_retry(),
            "malformed code should trigger retry, got: {verdict:?}"
        );
    }

    /// Simulates a read-only subagent (e.g., reviewer) that violates
    /// its constraint by attempting writes.
    #[test]
    fn catches_readonly_constraint_violation() {
        let output = "I found several issues and used write_file to fix them automatically.";
        let task = "read-only: Review the permission module for security issues";

        let verdict = evaluate_subagent_output(output, task, DEFAULT_MAX_RETRIES, 0);
        assert!(
            verdict.is_retry(),
            "constraint violation should trigger retry, got: {verdict:?}"
        );
    }

    /// Simulates a successful subagent that returns valid, clean output.
    /// The evaluator must accept it without retry.
    #[test]
    fn accepts_valid_subagent_output() {
        let output = r#"
## Analysis of crates/cade-core/src/permissions/mod.rs

### Key Findings:
1. The `resolve()` function at line 1180 correctly implements defence-in-depth
2. Protected paths are checked before mode-based logic
3. No security issues found in the deny rule evaluation

### Files Examined:
- `crates/cade-core/src/permissions/mod.rs` (1400 lines)
- `tests/approval_tests.rs` (190 lines)

All permission invariants appear to hold correctly.
"#;
        let task = "read-only: Analyze the permission module";

        let verdict = evaluate_subagent_output(output, task, DEFAULT_MAX_RETRIES, 0);
        assert!(
            verdict.is_accept(),
            "valid output should be accepted, got: {verdict:?}"
        );
    }

    /// Simulates the full retry lifecycle: first attempt fails, retry succeeds.
    #[test]
    fn retry_lifecycle_first_fails_second_succeeds() {
        let task = "Implement a helper function";

        // Attempt 0: subagent returns empty → Retry(attempt=1)
        let v1 = evaluate_subagent_output("", task, DEFAULT_MAX_RETRIES, 0);
        assert_eq!(
            v1,
            EvalVerdict::Retry {
                feedback: "subagent returned empty output".into(),
                attempt: 1,
            }
        );

        // Attempt 1: subagent returns valid code → Accept
        let good_output = "```rust\nfn helper() -> bool {\n    true\n}\n```";
        let v2 = evaluate_subagent_output(good_output, task, DEFAULT_MAX_RETRIES, 1);
        assert!(v2.is_accept(), "valid retry should be accepted, got: {v2:?}");
    }

    /// Simulates exhausting all retries — evaluator must reject.
    #[test]
    fn reject_after_max_retries_exhausted() {
        let task = "Generate a config parser";

        // Attempt 0: empty → Retry
        let v1 = evaluate_subagent_output("", task, DEFAULT_MAX_RETRIES, 0);
        assert!(v1.is_retry());

        // Attempt 1: still empty → Retry
        let v2 = evaluate_subagent_output("", task, DEFAULT_MAX_RETRIES, 1);
        assert!(v2.is_retry());

        // Attempt 2: still empty → Reject (max_retries=2, attempt=2)
        let v3 = evaluate_subagent_output("", task, DEFAULT_MAX_RETRIES, 2);
        assert!(
            v3.is_reject(),
            "should reject after exhausting retries, got: {v3:?}"
        );
    }

    /// Simulates a subagent error message being caught.
    #[test]
    fn catches_subagent_error_message() {
        let output = "Subagent error: Anthropic 404 Not Found: model: claude-3-5-haiku-20241022";
        let task = "Analyze the codebase structure";

        let verdict = evaluate_subagent_output(output, task, DEFAULT_MAX_RETRIES, 0);
        assert!(
            verdict.is_retry(),
            "error message should trigger retry, got: {verdict:?}"
        );
    }
}
