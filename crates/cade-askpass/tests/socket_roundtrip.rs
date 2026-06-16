//! End-to-end test for the cade-askpass binary.
//!
//! Spins up a 127.0.0.1 TCP server that implements the v1 protocol
//! (AUTH → OK → PROMPT → PASSWORD/CANCEL), runs the compiled binary
//! with `CADE_ASKPASS_SOCKET` and `CADE_ASKPASS_TOKEN`, and verifies
//! the full round-trip.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::Command;

use cade_askpass::protocol::Message;

/// Path to the compiled `cade-askpass` binary, set by Cargo.
const BIN: &str = env!("CARGO_BIN_EXE_cade-askpass");
const TOKEN: &str = "deadbeef01234567deadbeef01234567deadbeef01234567deadbeef01234567";

/// Shared helper: accept one connection, do the AUTH handshake, read
/// the PROMPT line, and reply with `response`.
fn serve_one(listener: &TcpListener, response: &Message) {
    let (stream, _peer) = listener.accept().expect("accept");
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut sender = stream;

    // AUTH
    let mut auth_line = String::new();
    reader.read_line(&mut auth_line).expect("read AUTH");
    let msg = Message::decode(auth_line.trim()).unwrap();
    let token = match msg {
        Message::Auth(tok) => tok,
        other => panic!("expected AUTH, got {other:?}"),
    };
    if token == TOKEN {
        sender.write_all(Message::Ok.encode().as_bytes()).unwrap();
    } else {
        sender.write_all(Message::Deny.encode().as_bytes()).unwrap();
        sender.flush().ok();
        return;
    }
    sender.flush().ok();

    // PROMPT
    let mut prompt_line = String::new();
    reader.read_line(&mut prompt_line).expect("read PROMPT");
    let pmsg = Message::decode(prompt_line.trim()).unwrap();
    assert!(matches!(pmsg, Message::Prompt(_)));

    // Response
    sender.write_all(response.encode().as_bytes()).unwrap();
    sender.flush().ok();
}

#[test]
fn full_round_trip_prompt_password() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local_addr");

    let server = std::thread::spawn(move || {
        serve_one(&listener, &Message::Password("hunter2".to_string()));
    });

    let output = Command::new(BIN)
        .arg("Password for ezektec:")
        .env("CADE_ASKPASS_SOCKET", format!("{addr}"))
        .env("CADE_ASKPASS_TOKEN", TOKEN)
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
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local_addr");

    let server = std::thread::spawn(move || {
        serve_one(&listener, &Message::Cancel);
    });

    let output = Command::new(BIN)
        .arg("anything")
        .env("CADE_ASKPASS_SOCKET", format!("{addr}"))
        .env("CADE_ASKPASS_TOKEN", TOKEN)
        .output()
        .expect("spawn cade-askpass");

    server.join().expect("server thread");

    assert!(
        !output.status.success(),
        "askpass binary should exit non-zero on CANCEL"
    );
    assert!(
        output.stdout.is_empty(),
        "no password should be written to stdout on cancel"
    );
}

#[test]
fn bad_token_yields_nonzero_exit() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local_addr");

    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let mut sender = stream;
        let mut auth = String::new();
        reader.read_line(&mut auth).unwrap();
        // Token won't match → DENY
        sender.write_all(b"DENY\n").unwrap();
        sender.flush().ok();
    });

    let output = Command::new(BIN)
        .arg("anything")
        .env("CADE_ASKPASS_SOCKET", format!("{addr}"))
        .env("CADE_ASKPASS_TOKEN", "wrong-token")
        .output()
        .expect("spawn cade-askpass");

    server.join().expect("server thread");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("rejected"),
        "stderr should mention rejection, got: {stderr}"
    );
}

#[test]
fn missing_socket_env_var_yields_nonzero_exit() {
    let output = Command::new(BIN)
        .arg("anything")
        .env_remove("CADE_ASKPASS_SOCKET")
        .env("CADE_ASKPASS_TOKEN", TOKEN)
        .output()
        .expect("spawn cade-askpass");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("CADE_ASKPASS_SOCKET"),
        "stderr should mention missing env var, got: {stderr}"
    );
}

#[test]
fn missing_token_env_var_yields_nonzero_exit() {
    let output = Command::new(BIN)
        .arg("anything")
        .env("CADE_ASKPASS_SOCKET", "127.0.0.1:1")
        .env_remove("CADE_ASKPASS_TOKEN")
        .output()
        .expect("spawn cade-askpass");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("CADE_ASKPASS_TOKEN"),
        "stderr should mention missing env var, got: {stderr}"
    );
}

#[test]
fn missing_prompt_argument_yields_nonzero_exit() {
    let output = Command::new(BIN)
        .env("CADE_ASKPASS_SOCKET", "127.0.0.1:1")
        .env("CADE_ASKPASS_TOKEN", TOKEN)
        .output()
        .expect("spawn cade-askpass");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("missing prompt"),
        "stderr should mention missing argument, got: {stderr}"
    );
}
