//! Process-global registration for the askpass IPC server.
//!
//! The actual TCP server lives in `cade-askpass` (feature `server`).  This
//! module just stores the *socket address + token* the server published so
//! that any Command spawned by the agent (typically `BashTool`) can have the
//! standard `SUDO_ASKPASS`/`SSH_ASKPASS`/`GIT_ASKPASS` plus
//! `CADE_ASKPASS_SOCKET`/`CADE_ASKPASS_TOKEN` env vars injected.
//!
//! ## Why a global?
//!
//! Bash invocations happen across many code paths (REPL, headless CLI,
//! server agent loop, hook script runner, MCP child processes, …) and we
//! want all of them to inherit the same askpass channel without threading a
//! handle through every call site.  The TUI binary registers its server once
//! at startup; everything downstream simply reads `current()`.

use std::path::PathBuf;
use std::sync::RwLock;

/// Standard env var name read by the `cade-askpass` helper binary.
pub const ENV_SOCKET: &str = "CADE_ASKPASS_SOCKET";
/// Standard env var name read by the `cade-askpass` helper binary.
pub const ENV_TOKEN: &str = "CADE_ASKPASS_TOKEN";

/// Snapshot of the registered askpass server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskpassChannel {
    /// e.g. `127.0.0.1:38271`.
    pub socket: String,
    /// 64-char hex token shared with the helper binary.
    pub token: String,
}

static CHANNEL: RwLock<Option<AskpassChannel>> = RwLock::new(None);

/// Register (or replace) the active askpass channel.
///
/// The TUI binary calls this once after starting its `AskpassServer`.
pub fn register(channel: AskpassChannel) {
    if let Ok(mut guard) = CHANNEL.write() {
        *guard = Some(channel);
    }
}

/// Clear the active askpass channel.  Used by tests and when the TUI exits.
pub fn clear() {
    if let Ok(mut guard) = CHANNEL.write() {
        *guard = None;
    }
}

/// Return a snapshot of the current channel (`None` if no server is active).
pub fn current() -> Option<AskpassChannel> {
    CHANNEL.read().ok().and_then(|g| g.clone())
}

/// Resolve the path to the `cade-askpass` helper binary.
///
/// Lookup order:
/// 1. Sibling of the running executable (`/path/to/cade` →
///    `/path/to/cade-askpass`).  This is the production case.
/// 2. The literal name `cade-askpass`, relying on `$PATH`.
pub fn helper_binary_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let bin_name = if cfg!(windows) {
            "cade-askpass.exe"
        } else {
            "cade-askpass"
        };
        let sibling = dir.join(bin_name);
        if sibling.exists() {
            return sibling;
        }
    }
    // Last resort: rely on $PATH.
    PathBuf::from(if cfg!(windows) {
        "cade-askpass.exe"
    } else {
        "cade-askpass"
    })
}

/// Trait shared with `agent_env::CommandAgentEnv` — implemented for both
/// std and tokio `Command`.
pub trait CommandAskpassEnv {
    fn set_askpass_env(&mut self, key: &str, value: &str);
}

impl CommandAskpassEnv for std::process::Command {
    fn set_askpass_env(&mut self, key: &str, value: &str) {
        self.env(key, value);
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl CommandAskpassEnv for tokio::process::Command {
    fn set_askpass_env(&mut self, key: &str, value: &str) {
        self.env(key, value);
    }
}

/// Inject askpass env vars (`SUDO_ASKPASS`, `SSH_ASKPASS`, `GIT_ASKPASS`,
/// `SSH_ASKPASS_REQUIRE`, `DISPLAY`, `CADE_ASKPASS_SOCKET`,
/// `CADE_ASKPASS_TOKEN`) into `cmd`.
///
/// No-op when no channel is registered — sudo/ssh fall back to their
/// default terminal prompt behaviour.  Returns `true` when env vars were
/// injected.
pub fn apply_askpass_env<C: CommandAskpassEnv>(cmd: &mut C) -> bool {
    let Some(channel) = current() else {
        return false;
    };
    let helper = helper_binary_path();
    let helper_str = helper.to_string_lossy().into_owned();

    cmd.set_askpass_env("SUDO_ASKPASS", &helper_str);
    cmd.set_askpass_env("SSH_ASKPASS", &helper_str);
    cmd.set_askpass_env("GIT_ASKPASS", &helper_str);

    // `sudo` normally requires `-A` to use the askpass helper when a terminal is present.
    // By exporting a bash function, any `bash -c` invocation that calls `sudo`
    // will automatically use `-A`, triggering our helper instead of hanging on /dev/tty.
    cmd.set_askpass_env("BASH_FUNC_sudo%%", "() { command sudo -A \"$@\"; }");

    // `ssh` requires SSH_ASKPASS_REQUIRE=force when there is no controlling
    // tty; we always set it to make the behaviour deterministic.
    cmd.set_askpass_env("SSH_ASKPASS_REQUIRE", "force");
    // `DISPLAY` is required for ssh's askpass on X-less systems.
    cmd.set_askpass_env("DISPLAY", ":0");
    cmd.set_askpass_env(ENV_SOCKET, &channel.socket);
    cmd.set_askpass_env(ENV_TOKEN, &channel.token);
    true
}

// region:    --- Tests
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize tests that touch the process-global CHANNEL.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn current_is_none_initially() {
        let _g = TEST_LOCK.lock().unwrap();
        clear();
        assert!(current().is_none());
    }

    #[test]
    fn register_then_current_returns_channel() {
        let _g = TEST_LOCK.lock().unwrap();
        clear();
        let channel = AskpassChannel {
            socket: "127.0.0.1:12345".to_string(),
            token: "deadbeef".to_string(),
        };
        register(channel.clone());
        assert_eq!(current().unwrap(), channel);
        clear();
    }

    #[test]
    fn clear_removes_channel() {
        let _g = TEST_LOCK.lock().unwrap();
        register(AskpassChannel {
            socket: "127.0.0.1:9".into(),
            token: "x".into(),
        });
        clear();
        assert!(current().is_none());
    }

    #[test]
    fn apply_askpass_env_is_noop_when_no_channel() {
        let _g = TEST_LOCK.lock().unwrap();
        clear();
        let mut cmd = std::process::Command::new("true");
        let injected = apply_askpass_env(&mut cmd);
        assert!(!injected);
    }

    #[test]
    fn apply_askpass_env_sets_all_vars_when_registered() {
        let _g = TEST_LOCK.lock().unwrap();
        register(AskpassChannel {
            socket: "127.0.0.1:55555".into(),
            token: "tok".into(),
        });
        let mut cmd = std::process::Command::new("true");
        let injected = apply_askpass_env(&mut cmd);
        assert!(injected);

        let envs: Vec<(String, Option<String>)> = cmd
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|s| s.to_string_lossy().into_owned()),
                )
            })
            .collect();
        let lookup = |key: &str| {
            envs.iter()
                .find(|(k, _)| k == key)
                .and_then(|(_, v)| v.clone())
        };
        assert_eq!(lookup(ENV_SOCKET).as_deref(), Some("127.0.0.1:55555"));
        assert_eq!(lookup(ENV_TOKEN).as_deref(), Some("tok"));
        assert_eq!(lookup("SSH_ASKPASS_REQUIRE").as_deref(), Some("force"));
        assert!(lookup("SUDO_ASKPASS").is_some());
        assert!(lookup("SSH_ASKPASS").is_some());
        assert!(lookup("GIT_ASKPASS").is_some());
        clear();
    }

    #[test]
    fn helper_binary_path_returns_something() {
        let p = helper_binary_path();
        let s = p.to_string_lossy();
        assert!(s.contains("cade-askpass"), "got: {s}");
    }
}
// endregion: --- Tests
