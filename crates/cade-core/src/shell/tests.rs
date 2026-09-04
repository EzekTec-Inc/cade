use super::*;
use crate::Result;
use std::time::Duration;

#[tokio::test]
async fn test_shell_execution_engine_real_os_process() -> Result<()> {
    let engine = ShellExecutionEngine::new();
    let req = ShellRequest::new("echo hello_world_cadeshell");
    let result = engine.execute(req).await?;

    assert!(result.stdout.contains("hello_world_cadeshell"));
    assert_eq!(result.exit_code, 0);
    assert!(!result.truncated);
    Ok(())
}

#[test]
fn test_head_tail_middle_truncation() {
    let mut long_text = String::new();
    for i in 0..10_000 {
        long_text.push_str(&format!("line_{i}\n"));
    }

    let (truncated, is_trunc) = truncate_head_tail(&long_text, 1_000);
    assert!(is_trunc);
    assert!(truncated.starts_with("line_0"));
    assert!(truncated.contains("characters omitted from middle"));
    assert!(truncated.ends_with("line_9999\n"));
}

#[test]
fn test_shell_request_builder() {
    let req = ShellRequest::new("cargo test")
        .with_timeout(Duration::from_secs(30))
        .with_env("TEST_KEY", "TEST_VAL");

    assert_eq!(req.command, "cargo test");
    assert_eq!(req.timeout, Duration::from_secs(30));
    assert_eq!(
        req.env.get("TEST_KEY").map(|s| s.as_str()),
        Some("TEST_VAL")
    );
}

#[tokio::test]
async fn test_shell_command_legacy_compatibility() -> Result<()> {
    let out = shell_command("echo legacy_compat").output().await?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("legacy_compat"));
    Ok(())
}
