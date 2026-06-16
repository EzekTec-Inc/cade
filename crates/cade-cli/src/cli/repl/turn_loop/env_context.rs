use super::Repl;

impl Repl {
    pub(crate) fn build_env_context(&self) -> String {
        use std::process::Command;

        let now = chrono::Local::now().format("%Y-%m-%d %H:%M %Z");

        // OS / kernel — platform-conditional detection
        let os_info = {
            #[cfg(unix)]
            {
                let uname = {
                    let mut cmd = Command::new("uname");
                    cade_core::agent_env::apply_agent_env(&mut cmd);
                    cmd.arg("-sr").output()
                }
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();

                // Try /etc/os-release for distro name (Linux)
                let distro = std::fs::read_to_string("/etc/os-release")
                    .unwrap_or_default()
                    .lines()
                    .find(|l| l.starts_with("PRETTY_NAME="))
                    .map(|l| {
                        l.trim_start_matches("PRETTY_NAME=")
                            .trim_matches('"')
                            .to_string()
                    })
                    .unwrap_or_default();

                if distro.is_empty() {
                    uname.trim().to_string()
                } else {
                    format!("{} ({})", uname.trim(), distro)
                }
            }
            #[cfg(windows)]
            {
                let ver = {
                    let mut cmd = Command::new("cmd.exe");
                    cade_core::agent_env::apply_agent_env(&mut cmd);
                    cmd.args(["/C", "ver"]).output()
                }
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();
                let ver = ver.trim();
                if ver.is_empty() {
                    "Windows".to_string()
                } else {
                    ver.to_string()
                }
            }
        };

        // CWD
        let cwd = self.cwd.display().to_string();

        // Git info (git works the same on all platforms)
        let git_info = {
            let branch = {
                let mut cmd = Command::new("git");
                cade_core::agent_env::apply_agent_env(&mut cmd);
                cmd.args(["-C", &cwd, "rev-parse", "--abbrev-ref", "HEAD"])
                    .output()
            }
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    String::from_utf8(o.stdout).ok()
                } else {
                    None
                }
            })
            .map(|s| s.trim().to_string());

            let status = {
                let mut cmd = Command::new("git");
                cade_core::agent_env::apply_agent_env(&mut cmd);
                cmd.args(["-C", &cwd, "status", "--porcelain"]).output()
            }
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    String::from_utf8(o.stdout).ok()
                } else {
                    None
                }
            });

            match (branch, status) {
                (Some(b), Some(s)) if !b.is_empty() => {
                    let lines: Vec<&str> = s.lines().collect();
                    if lines.is_empty() {
                        format!("branch={b}, clean")
                    } else {
                        format!(
                            "branch={b}, {} uncommitted change{}",
                            lines.len(),
                            if lines.len() == 1 { "" } else { "s" }
                        )
                    }
                }
                _ => String::new(),
            }
        };

        let mut parts = vec![
            format!("Date:   {now}"),
            format!("OS:     {os_info}"),
            format!("CWD:    {cwd}"),
        ];
        if !git_info.is_empty() {
            parts.push(format!("Git:    {git_info}"));
        }
        format!("<environment>\n{}\n</environment>", parts.join("\n"))
    }
}
