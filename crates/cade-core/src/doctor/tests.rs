use super::*;

#[test]
fn test_detect_no_multiplexer() {
    let report = check_multiplexer_with_env(|_| None, None);
    assert_eq!(report.multiplexer, MultiplexerKind::None);
    assert!(report.is_healthy());
    assert!(report.warnings.is_empty());
    assert!(report.key_passthrough_issues.is_empty());
}

#[test]
fn test_detect_screen_multiplexer() {
    let report = check_multiplexer_with_env(
        |var| {
            if var == "STY" {
                Some("12345.pts-1.host".to_string())
            } else {
                None
            }
        },
        None,
    );

    assert_eq!(
        report.multiplexer,
        MultiplexerKind::Screen {
            session: "12345.pts-1.host".to_string()
        }
    );
    assert!(report.is_healthy());
}

#[test]
fn test_detect_clean_tmux_multiplexer() {
    let root_keys = "bind-key -T root WheelUpPane select-pane -t = \\; send-keys -M\n";
    let report = check_multiplexer_with_env(
        |var| match var {
            "TMUX" => Some("/tmp/tmux-1000/default,1234,0".to_string()),
            "TMUX_PANE" => Some("%1".to_string()),
            _ => None,
        },
        Some(root_keys),
    );

    assert_eq!(
        report.multiplexer,
        MultiplexerKind::Tmux {
            socket: "/tmp/tmux-1000/default".to_string(),
            pane: Some("%1".to_string()),
        }
    );
    assert!(report.is_healthy());
    assert!(report.key_passthrough_issues.is_empty());
}

#[test]
fn test_detect_tmux_root_conflicts_from_list_keys() {
    let root_keys = r#"
bind-key  -T root H                         resize-pane -L 5
bind-key  -T root J                         resize-pane -D 5
bind-key  -T root K                         resize-pane -U 5
bind-key  -T root L                         resize-pane -R 5
bind-key  -T root WheelUpPane               select-pane -t =
"#;

    let report = check_multiplexer_with_env(
        |var| {
            if var == "TMUX" {
                Some("/tmp/tmux-1000/default,1234,0".to_string())
            } else {
                None
            }
        },
        Some(root_keys),
    );

    assert!(!report.is_healthy());
    assert_eq!(report.key_passthrough_issues.len(), 4);

    let keys: Vec<&str> = report
        .key_passthrough_issues
        .iter()
        .map(|i| i.key.as_str())
        .collect();
    assert_eq!(keys, vec!["H", "J", "K", "L"]);

    assert!(report.key_passthrough_issues[0]
        .remediation
        .contains("In ~/.tmux.conf"));
}

#[test]
fn test_detect_tmux_root_conflicts_from_conf_syntax() {
    let conf = r#"
##### ================= PANE RESIZE (Shift + hjkl) ================= #####
bind -n H resize-pane -L 5
bind -n J resize-pane -D 5
bind -n K resize-pane -U 5
bind -n L resize-pane -R 5
bind -n M-H resize-pane -L 5
"#;

    let issues = parse_tmux_root_key_issues(conf);
    assert_eq!(issues.len(), 4);
    assert_eq!(issues[0].key, "H");
    assert_eq!(issues[1].key, "J");
    assert_eq!(issues[2].key, "K");
    assert_eq!(issues[3].key, "L");
    // Notice M-H (Alt+H) is not flagged as bare printable intercept
}
