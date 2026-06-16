//! Platform-appropriate shell command builders.
//!
//! Every crate that needs to spawn a shell command should call one of these
//! helpers instead of hard-coding `Command::new("bash")`.
//!
//! | Platform        | Shell used       |
//! |-----------------|------------------|
//! | Unix (Linux/macOS) | `bash -c`     |
//! | Windows         | `cmd.exe /C`     |

/// Build an async (`tokio`) shell command pre-configured for the host platform.
///
/// Returns a `tokio::process::Command` with the shell binary and `-c` / `/C`
/// argument already set.  Callers still need to apply `agent_env`,
/// `.current_dir()`, and `Stdio` configuration as needed.
pub fn shell_command(command: &str) -> tokio::process::Command {
    #[cfg(unix)]
    {
        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("-c").arg(command);
        cmd
    }
    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("cmd.exe");
        cmd.arg("/C").arg(command);
        cmd
    }
}

/// Same as [`shell_command`] but returns a synchronous `std::process::Command`.
///
/// Use in contexts where async is not available (e.g. `build_env_context`).
pub fn shell_command_sync(command: &str) -> std::process::Command {
    #[cfg(unix)]
    {
        let mut cmd = std::process::Command::new("bash");
        cmd.arg("-c").arg(command);
        cmd
    }
    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new("cmd.exe");
        cmd.arg("/C").arg(command);
        cmd
    }
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

    use super::*;

    #[tokio::test]
    async fn shell_command_echo() -> Result<()> {
        let out = shell_command("echo hello").output().await?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("hello"), "got: {stdout}");
        Ok(())
    }

    #[test]
    fn shell_command_sync_echo() -> Result<()> {
        let out = shell_command_sync("echo hello").output()?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("hello"), "got: {stdout}");
        Ok(())
    }
}

// endregion: --- Tests
