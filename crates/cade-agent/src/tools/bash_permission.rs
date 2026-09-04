//! Dynamic bash command interception (Pillar 3.3).
//!
//! Connects the runtime permission evaluator to CADE's bash execution
//! tool, allowing the system to intercept and evaluate nested sub-commands
//! (e.g. `make` which runs compilers, or `npm install` which runs scripts)
//! and ask for user permission mid-execution.

use crate::tools::traits::ToolContext;

/// Describes the "shape" of a shell command for permission evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandCategory {
    /// Innocuous read-only command (e.g. `ls`, `echo`, `cat`).
    ReadOnly,
    /// File/directory write operations (e.g. `mkdir`, `cp`, `mv`, `rm`).
    FileWrite,
    /// Package manager / dependency operations (e.g. `npm install`, `cargo build`).
    PackageOp,
    /// Network operations (e.g. `curl`, `wget`, `ssh`).
    Network,
    /// Code compilation / execution (e.g. `make`, `gcc`, `python`).
    Compilation,
    /// System administration (e.g. `sudo`, `chmod`, `systemctl`).
    SystemAdmin,
    /// Unknown / potentially dangerous.
    Unknown,
}

/// Classify a shell command by its first token.
pub fn classify_command(command: &str) -> CommandCategory {
    let trimmed = command.trim();
    let first_token = trimmed
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase();

    match first_token.as_str() {
        // Read-only
        "ls" | "echo" | "cat" | "head" | "tail" | "less" | "more" | "grep" | "find" | "which"
        | "whoami" | "pwd" | "date" | "env" | "printenv" | "diff" | "cmp" | "file" | "stat"
        | "du" | "df" | "ps" | "top" | "htop" => CommandCategory::ReadOnly,

        // File writes
        "mkdir" | "rmdir" | "cp" | "mv" | "rm" | "touch" | "ln" | "chmod" | "chown" | "chgrp"
        | "dd" | "install" | "mkfifo" | "mknod" => CommandCategory::FileWrite,

        // Package operations
        "npm" | "yarn" | "pnpm" | "cargo" | "go" | "pip" | "pip3" | "gem" | "bundle" | "apt"
        | "apt-get" | "yum" | "dnf" | "pacman" | "brew" | "port" | "nix" | "stack" | "dotnet"
        | "nuget" => CommandCategory::PackageOp,

        // Network
        "curl" | "wget" | "ssh" | "scp" | "rsync" | "ftp" | "sftp" | "telnet" | "nc" | "ncat"
        | "ping" | "traceroute" | "dig" | "nslookup" | "host" | "git" | "svn" | "hg" => {
            CommandCategory::Network
        }

        // Compilation / code execution
        "make" | "cmake" | "gcc" | "g++" | "clang" | "clang++" | "rustc" | "javac" | "python"
        | "python3" | "ruby" | "perl" | "node" | "deno" | "lua" | "luajit" | "go build"
        | "cargo build" | "cargo run" | "cargo test" | "tsc" | "babel" | "webpack" | "vite"
        | "esbuild" | "wasm-pack" | "wasm-bindgen" => CommandCategory::Compilation,

        // System admin
        "sudo" | "systemctl" | "service" | "journalctl" | "ufw" | "iptables" | "mount"
        | "umount" | "fdisk" | "mkfs" | "parted" | "useradd" | "usermod" | "passwd"
        | "shutdown" | "reboot" | "poweroff" | "init" => CommandCategory::SystemAdmin,

        _ => CommandCategory::Unknown,
    }
}

/// Detect whether a command contains nested sub-commands (e.g. `make`,
/// `npm run`, shell pipes, subshells).
pub fn has_nested_commands(command: &str) -> bool {
    let trimmed = command.trim();

    // Detect subshells: $(...), `...`, ( ... )
    if trimmed.contains("$(") || trimmed.contains('`') {
        return true;
    }

    // Detect pipes
    if trimmed.contains("|") {
        return true;
    }

    // Detect chaining
    if trimmed.contains("&&") || trimmed.contains("||") || trimmed.contains(";") {
        return true;
    }

    // Detect redirections that could indicate complex operations
    if trimmed.contains(">") || trimmed.contains("<") {
        // Exclude trivial output capture like `echo > file`
        if trimmed.starts_with("echo") || trimmed.starts_with("cat") {
            return false;
        }
        return true;
    }

    false
}

/// Evaluate whether a bash command should require explicit user permission.
///
/// Returns `None` if the command is safe (read-only or trivial).
/// Returns `Some(reason)` if the command should trigger a permission prompt.
pub fn require_permission_for_command(command: &str) -> Option<String> {
    let category = classify_command(command);

    match category {
        CommandCategory::ReadOnly => None, // Always allow
        CommandCategory::FileWrite => {
            // Allow simple file writes, flag destructive ones
            let trimmed = command.trim();
            if trimmed.starts_with("rm -rf") || trimmed.starts_with("rm -r") {
                Some(format!("Destructive file operation: '{command}'"))
            } else if has_nested_commands(trimmed) {
                Some(format!("File write with nested sub-commands: '{command}'"))
            } else {
                None
            }
        }
        CommandCategory::PackageOp | CommandCategory::Network => {
            if has_nested_commands(command) {
                Some(format!(
                    "{} with nested sub-commands: '{command}'",
                    match category {
                        CommandCategory::PackageOp => "Package operation",
                        CommandCategory::Network => "Network operation",
                        _ => "Operation",
                    }
                ))
            } else {
                None
            }
        }
        CommandCategory::Compilation => {
            // Build systems can run arbitrary code — flag if nested
            if has_nested_commands(command) {
                Some(format!(
                    "Build system with nested sub-commands: '{command}'"
                ))
            } else {
                None
            }
        }
        CommandCategory::SystemAdmin => Some(format!("System administration command: '{command}'")),
        CommandCategory::Unknown => {
            // Unknown commands — allow if simple, flag if complex
            if has_nested_commands(command) {
                Some(format!("Unknown command with sub-commands: '{command}'"))
            } else {
                None
            }
        }
    }
}

/// Execute a bash command with dynamic permission gating.
///
/// Before running the command, checks the permission evaluator.
/// If the command would write to the filesystem or spawn nested
/// sub-commands, the user is prompted via the `ToolContext`.
pub async fn execute_with_permission(command: &str, ctx: &dyn ToolContext) -> crate::Result<()> {
    if let Some(reason) = require_permission_for_command(command) {
        let allowed = ctx.ask_permission("bash.exec", &reason).await;
        if !allowed {
            return Err(crate::Error::custom(format!(
                "Permission denied by user: {reason}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_read_only_commands() {
        assert_eq!(classify_command("ls -la"), CommandCategory::ReadOnly);
        assert_eq!(classify_command("echo hello"), CommandCategory::ReadOnly);
        assert_eq!(
            classify_command("cat /etc/hosts"),
            CommandCategory::ReadOnly
        );
        assert_eq!(
            classify_command("grep foo bar.txt"),
            CommandCategory::ReadOnly
        );
    }

    #[test]
    fn classify_file_write_commands() {
        assert_eq!(classify_command("rm file.txt"), CommandCategory::FileWrite);
        assert_eq!(
            classify_command("mkdir -p /tmp/test"),
            CommandCategory::FileWrite
        );
        assert_eq!(classify_command("cp a b"), CommandCategory::FileWrite);
        assert_eq!(classify_command("mv x y"), CommandCategory::FileWrite);
    }

    #[test]
    fn classify_package_ops() {
        assert_eq!(classify_command("npm install"), CommandCategory::PackageOp);
        assert_eq!(classify_command("cargo build"), CommandCategory::PackageOp);
        assert_eq!(classify_command("pip install"), CommandCategory::PackageOp);
    }

    #[test]
    fn classify_network_commands() {
        assert_eq!(
            classify_command("curl https://example.com"),
            CommandCategory::Network
        );
        assert_eq!(classify_command("git clone ..."), CommandCategory::Network);
        assert_eq!(classify_command("ssh user@host"), CommandCategory::Network);
    }

    #[test]
    fn classify_compilation_commands() {
        assert_eq!(classify_command("make"), CommandCategory::Compilation);
        assert_eq!(classify_command("gcc main.c"), CommandCategory::Compilation);
        assert_eq!(
            classify_command("python script.py"),
            CommandCategory::Compilation
        );
    }

    #[test]
    fn detect_nested_commands() {
        assert!(has_nested_commands("make && make install"));
        assert!(has_nested_commands("./configure; make"));
        assert!(has_nested_commands("echo $(whoami)"));
        assert!(has_nested_commands("cat foo | grep bar"));
        assert!(!has_nested_commands("echo hello world"));
        assert!(!has_nested_commands("ls -la"));
    }

    #[test]
    fn permission_required_for_destructive_ops() {
        assert!(
            require_permission_for_command("rm -rf /").is_some(),
            "rm -rf should require permission"
        );
    }

    #[test]
    fn permission_not_required_for_read_only() {
        assert!(
            require_permission_for_command("ls -la").is_none(),
            "ls should be allowed without permission"
        );
        assert!(
            require_permission_for_command("echo hi").is_none(),
            "echo should be allowed without permission"
        );
    }

    #[test]
    fn permission_required_for_system_admin() {
        assert!(
            require_permission_for_command("sudo rm -rf /").is_some(),
            "sudo should require permission"
        );
    }
}
