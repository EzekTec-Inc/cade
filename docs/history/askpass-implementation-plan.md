# Askpass Implementation Plan

## Overview
Securely capture OS-level password challenges (e.g., `sudo`, `ssh`) for background processes running in CADE, without using brittle PTY scraping. This uses the industry-standard Askpass pattern to safely request credentials and pass them directly to the underlying utility.

## Architecture

1. **`cade-askpass` Binary**
   - Create a new binary crate `crates/cade-askpass`.
   - The binary reads the prompt from `argv[1]`.
   - It connects to a local IPC channel (e.g., a local TCP port specified in an environment variable `CADE_ASKPASS_SOCKET`).
   - It sends the prompt over the IPC and blocks, waiting for the password.
   - Upon receiving the password, it prints it to `stdout` and exits with `0`.

2. **IPC Server in CADE (`cade-agent`)**
   - When launching a bash session in `BashTool`, create an ephemeral IPC listener (e.g., `127.0.0.1:0`).
   - Pass `SUDO_ASKPASS` (pointing to the `cade-askpass` binary), `SSH_ASKPASS`, and `CADE_ASKPASS_SOCKET` to the `Command` environment.
   - Optionally, configure the shell profile so that `sudo` aliases to `sudo -A` so the user/agent does not need to explicitly use `-A`.
   - When a connection and prompt arrive from `cade-askpass`, the listener fires an `AskPassword` event via the existing event stream to the main application loop.

3. **TUI Integration (`cade-tui`)**
   - Implement an `ask_password_blocking` method, similar to the existing `ask_question_blocking`.
   - The method displays a blocking modal with an input field masked with `*`.
   - The user enters the password and submits.
   - The TUI returns the answer back through the channel to the IPC server, which relays it to the `cade-askpass` binary.

## Implementation Steps

1. **Scaffold `cade-askpass`**
   - `cargo new crates/cade-askpass --bin`
   - Add to workspace `Cargo.toml` members.
   - Implement simple TCP socket client taking `argv[1]` as the prompt.
   - Read from socket, write response to `stdout`, and exit.

2. **IPC Server setup in `cade-agent/src/tools/bash.rs`**
   - Setup a `TcpListener::bind("127.0.0.1:0")` right before spawning the command.
   - Inject the allocated port and Askpass binary path into the bash `Command` env vars.
   - Spawn a tokio task to handle exactly one incoming connection from the askpass client.
   - Define a channel mechanism to signal the UI thread to request the password.

3. **TUI Modal for Passwords**
   - In `crates/cade-tui/src/app/`, add a password input widget.
   - Ensure the characters are masked.
   - Block execution until the user either submits or cancels (Escape). If cancelled, return an empty string to fail the `sudo` command gracefully.

4. **Security & Cleanup**
   - Ensure the ephemeral TCP port is bound exclusively to `127.0.0.1`.
   - Handle timeout conditions where `askpass` connects but the user never types a password.
   - Include `cade-askpass` in the standard build and release configuration so the executable is always available adjacent to the main `cade` binary.
