//! End-to-end test for the cade-askpass binary.
//!
//! Spins up a 127.0.0.1 TCP server, runs the compiled binary with
//! `CADE_ASKPASS_SOCKET` pointing at it, captures its stdout, and
//! verifies the full PROMPT → PASSWORD round-trip plus the CANCEL
//! exit-code path.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::Command;

/// Path to the compiled `cade-askpass` binary, set by Cargo.
const BIN: &str = env!("CARGO_BIN_EXE_cade-askpass");

#[test]
fn full_round_trip_prompt_password() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1");
    let addr = listener.local_addr().expect("local_addr");

    // Server thread: accept one connection, read the PROMPT line,
    // write back PASSWORD.
    let server = std::thread::spawn(move || {
        let (stream, _peer) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("read prompt line");

        // Decode the PROMPT line manually to avoid leaking lib state
        // through binary integration tests.
        assert!(line.starts_with("PROMPT\t"), "got: {line:?}");
        assert!(line.contains("Password for ezektec:"));

        let mut sender = stream;
        sender
            .write_all(b"PASSWORD\thunter2\n")
            .expect("write password");
        sender.flush().ok();
    });

    let output = Command::new(BIN)
        .arg("Password for ezektec:")
        .env("CADE_ASKPASS_SOCKET", format!("{addr}"))
        .output()
        .expect("spawn cade-askpass");

    server.join().expect("server thread");

    assert!(
        output.status.success(),
        "askpass binary exited non-zero: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "hunter2",
        "stdout should contain only the password (no newline)"
    );
}

#[test]
fn cancel_response_yields_nonzero_exit() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1");
    let addr = listener.local_addr().expect("local_addr");

    let server = std::thread::spawn(move || {
        let (stream, _peer) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("read prompt");
        let mut sender = stream;
        sender.write_all(b"CANCEL\n").expect("write cancel");
        sender.flush().ok();
    });

    let output = Command::new(BIN)
        .arg("anything")
        .env("CADE_ASKPASS_SOCKET", format!("{addr}"))
        .output()
        .expect("spawn cade-askpass");

    server.join().expect("server thread");

    assert!(
        !output.status.success(),
        "askpass binary should exit non-zero on CANCEL"
    );
    assert!(
        output.stdout.is_empty(),
        "no password should be written to stdout on cancel: {:?}",
        output.stdout
    );
}

#[test]
fn missing_socket_env_var_yields_nonzero_exit() {
    let output = Command::new(BIN)
        .arg("anything")
        .env_remove("CADE_ASKPASS_SOCKET")
        .output()
        .expect("spawn cade-askpass");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("CADE_ASKPASS_SOCKET"),
        "stderr should mention the missing env var, got: {stderr}"
    );
}

#[test]
fn missing_prompt_argument_yields_nonzero_exit() {
    let output = Command::new(BIN)
        .env("CADE_ASKPASS_SOCKET", "127.0.0.1:1") // unreachable, but we exit before connecting
        .output()
        .expect("spawn cade-askpass");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("missing prompt"),
        "stderr should mention the missing argument, got: {stderr}"
    );
}
