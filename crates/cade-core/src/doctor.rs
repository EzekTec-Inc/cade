//! Doctor health check and diagnostics module.
//!
//! Inspects terminal multiplexer environment (tmux, screen), detects key
//! passthrough conflicts (such as un-prefixed root bindings `bind -n H/J/K/L`),
//! and provides actionable remediation guidance.

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultiplexerKind {
    Tmux {
        socket: String,
        pane: Option<String>,
    },
    Screen {
        session: String,
    },
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPassthroughIssue {
    pub key: String,
    pub action: String,
    pub description: String,
    pub remediation: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorReport {
    pub multiplexer: MultiplexerKind,
    pub warnings: Vec<String>,
    pub info: Vec<String>,
    pub key_passthrough_issues: Vec<KeyPassthroughIssue>,
}

impl DoctorReport {
    pub fn is_healthy(&self) -> bool {
        self.key_passthrough_issues.is_empty() && self.warnings.is_empty()
    }

    pub fn to_formatted_summary(&self) -> String {
        let mut out = String::new();
        match &self.multiplexer {
            MultiplexerKind::Tmux { socket, pane } => {
                out.push_str("  Multiplexer: tmux detected\n");
                out.push_str(&format!("    socket: {socket}\n"));
                if let Some(p) = pane {
                    out.push_str(&format!("    pane: {p}\n"));
                }
            }
            MultiplexerKind::Screen { session } => {
                out.push_str(&format!("  Multiplexer: GNU Screen detected (session: {session})\n"));
            }
            MultiplexerKind::None => {
                out.push_str("  Multiplexer: None (direct TTY session)\n");
            }
        }

        if !self.key_passthrough_issues.is_empty() {
            out.push_str("\n  ⚠ Key Passthrough Conflicts:\n");
            for issue in &self.key_passthrough_issues {
                out.push_str(&format!("    • Key '{}': {}\n", issue.key, issue.description));
                out.push_str(&format!("      Action: {}\n", issue.action));
                out.push_str(&format!("      Remediation: {}\n", issue.remediation));
            }
        } else if matches!(self.multiplexer, MultiplexerKind::Tmux { .. }) {
            out.push_str("  ✓ Key Passthrough: No conflicting un-prefixed root bindings detected in tmux.\n");
        }

        for info in &self.info {
            out.push_str(&format!("  ℹ {info}\n"));
        }

        for warn in &self.warnings {
            out.push_str(&format!("  ⚠ {warn}\n"));
        }

        out
    }
}

/// Parse tmux root bindings string and return any single-character printable intercepts.
pub fn parse_tmux_root_key_issues(root_table_output: &str) -> Vec<KeyPassthroughIssue> {
    let mut issues = Vec::new();

    for line in root_table_output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Expected format from `tmux list-keys -T root`:
        // bind-key -T root H resize-pane -L 5
        // or from tmux.conf:
        // bind -n H resize-pane -L 5
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let (key_token, action_tokens) = if tokens.len() >= 5
            && (tokens[0] == "bind-key" || tokens[0] == "bind")
            && tokens[1] == "-T"
            && tokens[2] == "root"
        {
            (tokens[3], &tokens[4..])
        } else if tokens.len() >= 4
            && (tokens[0] == "bind-key" || tokens[0] == "bind")
            && tokens[1] == "-n"
        {
            (tokens[2], &tokens[3..])
        } else {
            continue;
        };

        // Check if the key is a bare single printable character (e.g. 'H', 'J', 'K', 'L', 'a'-'z', '0'-'9')
        // Modifiers like M-H or C-h have prefix/hyphen and len > 1.
        let is_bare_printable = (key_token.len() == 1
            && key_token.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()))
            || is_shifted_bare_letter(key_token);

        if is_bare_printable {
            let action = action_tokens.join(" ");
            issues.push(KeyPassthroughIssue {
                key: key_token.to_string(),
                action: action.clone(),
                description: format!(
                    "Un-prefixed root key binding intercepts '{key_token}' before it reaches terminal applications"
                ),
                remediation: format!(
                    "In ~/.tmux.conf, change 'bind -n {key_token} {action}' to 'bind {key_token} {action}' (requires prefix) or 'bind -n M-{key_token} {action}' (requires Alt)"
                ),
            });
        }
    }

    issues
}

fn is_shifted_bare_letter(token: &str) -> bool {
    // Some formats might be "S-H" or single uppercase char
    if token.len() == 1 {
        let c = token.chars().next().unwrap();
        c.is_ascii_uppercase()
    } else if let Some(stripped) = token.strip_prefix("S-") {
        stripped.len() == 1 && stripped.chars().next().unwrap().is_ascii_alphabetic()
    } else {
        false
    }
}

/// Check terminal multiplexer and key passthrough using custom env getter and optional tmux root bindings.
pub fn check_multiplexer_with_env<F>(get_env: F, tmux_root_keys: Option<&str>) -> DoctorReport
where
    F: Fn(&str) -> Option<String>,
{
    let mut warnings = Vec::new();
    let mut info = Vec::new();
    let mut key_passthrough_issues = Vec::new();

    let multiplexer = if let Some(tmux_val) = get_env("TMUX") {
        let pane = get_env("TMUX_PANE");
        let socket = tmux_val.split(',').next().unwrap_or(&tmux_val).to_string();

        info.push("Running inside a tmux session. Standard keys should pass through transparently unless bound in root table (-T root or bind -n).".to_string());

        if let Some(keys_output) = tmux_root_keys {
            let found_issues = parse_tmux_root_key_issues(keys_output);
            if !found_issues.is_empty() {
                warnings.push(format!(
                    "Detected {} un-prefixed root keybinding(s) in tmux that hijack keystrokes!",
                    found_issues.len()
                ));
                key_passthrough_issues = found_issues;
            }
        }

        MultiplexerKind::Tmux { socket, pane }
    } else if let Some(sty_val) = get_env("STY") {
        info.push("Running inside a GNU Screen session ($STY set).".to_string());
        MultiplexerKind::Screen { session: sty_val }
    } else {
        MultiplexerKind::None
    };

    DoctorReport {
        multiplexer,
        warnings,
        info,
        key_passthrough_issues,
    }
}

/// Check terminal multiplexer and key passthrough using system environment and live tmux query.
pub fn check_multiplexer_and_keys() -> DoctorReport {
    let tmux_keys = if std::env::var("TMUX").is_ok() {
        query_live_tmux_root_keys()
    } else {
        None
    };

    check_multiplexer_with_env(|var| std::env::var(var).ok(), tmux_keys.as_deref())
}

fn query_live_tmux_root_keys() -> Option<String> {
    // 1. Try querying running tmux server directly
    if let Ok(output) = std::process::Command::new("tmux")
        .args(["list-keys", "-T", "root"])
        .output()
        && output.status.success()
    {
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        if !stdout.trim().is_empty() {
            return Some(stdout);
        }
    }

    // 2. Fallback: inspect user's ~/.tmux.conf if available
    if let Some(home) = dirs::home_dir() {
        let conf_path = home.join(".tmux.conf");
        if conf_path.exists()
            && let Ok(content) = std::fs::read_to_string(conf_path)
        {
            return Some(content);
        }
    }

    None
}
