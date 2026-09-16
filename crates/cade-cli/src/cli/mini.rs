use crate::Result;
use crate::support::text::sanitize_for_terminal;
use cade_agent::agent::HttpTransport;
use std::io::{self, BufRead, Write};

/// Run an interactive inline stream session (mini mode).
///
/// Unlike the full-screen TUI, mini mode operates purely on stdout/stdin
/// without entering the alternate screen buffer, streaming tokens live and
/// accepting line-oriented inputs.
pub async fn run_mini_interactive(
    client: &HttpTransport,
    agent_id: &str,
    model: &str,
    cwd: &std::path::Path,
) -> Result<()> {
    println!(
        "\x1b[1;36mCADE\x1b[0m \x1b[2m(mini mode) — v{}\x1b[0m",
        env!("CARGO_PKG_VERSION")
    );
    println!(
        "\x1b[2mModel: {} | Workspace: {}\x1b[0m",
        model,
        cwd.display()
    );
    println!("\x1b[2mType /exit to quit, /clear to clear screen, /help for help.\x1b[0m\n");

    let stdin = io::stdin();
    let mut stdin_lock = stdin.lock();

    loop {
        print!("\x1b[1;32mcade\x1b[0m \x1b[2m(mini)\x1b[0m > ");
        io::stdout().flush().ok();

        let mut input = String::new();
        let bytes_read = match stdin_lock.read_line(&mut input) {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!("Failed to read stdin: {e}");
                break;
            }
        };

        if bytes_read == 0 {
            // EOF (Ctrl+D)
            println!("\n\x1b[2mGoodbye!\x1b[0m");
            break;
        }

        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Handle slash commands
        if trimmed == "/exit" || trimmed == "/quit" || trimmed == "exit" || trimmed == "quit" {
            println!("\x1b[2mGoodbye!\x1b[0m");
            break;
        }

        if trimmed == "/clear" {
            print!("\x1b[2J\x1b[H");
            io::stdout().flush().ok();
            continue;
        }

        if trimmed == "/help" {
            println!("\n\x1b[1mCommands:\x1b[0m");
            println!("  /exit, /quit   Exit mini session");
            println!("  /clear         Clear terminal screen");
            println!("  /help          Show this help message\n");
            continue;
        }

        println!();
        let result = client
            .start_run(agent_id, trimmed, None, |msg| {
                if let Some(text) = msg.assistant_text() {
                    print!("{}", sanitize_for_terminal(text));
                    let _ = io::stdout().flush();
                }
            })
            .await;

        println!("\n");

        if let Err(e) = result {
            eprintln!("\x1b[1;31mError:\x1b[0m {}\n", e);
        }
    }

    Ok(())
}
