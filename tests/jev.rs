//! clank-jev from outside the process: the request it sends, the answer it prints,
//! and the exit code a script branches on.
//!
//! The stub speaks plain HTTP/1.1 with a JSON body — no SSE here — and records what
//! it was asked, so these assertions are made against the bytes on the wire.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::thread;

const BIN: &str = env!("CARGO_BIN_EXE_clank-jev");

struct Stub {
    url: String,
    seen: Receiver<Value>,
}

impl Stub {
    /// `reply` is the JSON body to answer every request with, and the status code.
    fn start(status: u16, reply: Value) -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
        let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
        let (tx, seen) = channel();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let body = read_request(&mut stream).unwrap_or(Value::Null);
                if tx.send(body).is_err() {
                    break;
                }
                let payload = serde_json::to_string(&reply).unwrap_or_default();
                let head = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(payload.as_bytes());
                let _ = stream.flush();
            }
        });
        Stub { url, seen }
    }

    fn request(&self) -> Value {
        self.seen.try_iter().next().expect("one request")
    }
}

/// Reads one HTTP request and returns its JSON body.
fn read_request(stream: &mut std::net::TcpStream) -> Option<Value> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = rest.trim().parse().ok()?;
        }
        if line == "\r\n" {
            break;
        }
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

fn run(args: &[&str], state: &str, url: &str, key: Option<&str>) -> (String, String, i32) {
    run_provider(args, state, url, key, "typesafe")
}

fn run_provider(args: &[&str], state: &str, url: &str, key: Option<&str>, provider: &str)
    -> (String, String, i32) {
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .arg("--base-url")
        .arg(url)
        .arg("--provider")
        .arg(provider)
        .env_remove("TYPESAFE_API_KEY")
        .env_remove("JEV_API_KEY")
        .env_remove("JEV_CLI_API_KEY")
        .env_remove("OPENROUTER_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(k) = key {
        cmd.env("TYPESAFE_API_KEY", k);
    }
    let mut child = cmd.spawn().expect("spawn clank-jev");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(state.as_bytes())
        .expect("write state");
    let out = child.wait_with_output().expect("wait");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

fn decision(value: &str, probability: f64) -> Value {
    json!({"model": "jev-1.13.0", "answers": {
        "answer": {"type": "choice", "choice": value, "probabilities": {value: probability}},
    }, "usage": {"input_tokens": 41, "output_tokens": 3}})
}

#[test]
fn one_question_prints_the_bare_value_and_sends_the_wire_shape() {
    let stub = Stub::start(200, decision("code", 0.97));
    let (stdout, stderr, code) = run(
        &["--ask", "What kind of task is this?", "--choice", "code,prose,math"],
        "write me a function",
        &stub.url,
        Some("test-key"),
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "code", "a script wants the value alone");

    let sent = stub.request();
    assert_eq!(sent["state"], "write me a function");
    assert_eq!(sent["model"], "jev-latest", "the TypeSafe route's default model");
    let q = &sent["questions"]["answer"];
    assert_eq!(q["type"], "choice");
    assert_eq!(q["instructions"], "What kind of task is this?");
    assert!(q["criteria"].is_object(), "choice carries its options: {q}");
    assert_eq!(q["criteria"].as_object().unwrap().len(), 3);
}

#[test]
fn a_boolean_question_goes_on_the_wire_as_noul() {
    let stub = Stub::start(200, json!({"model": "jev-1.13.0",
        "answers": {"answer": {"type": "noul", "noul": 0.88}}, "usage": {}}));
    let (stdout, _, code) = run(&["--ask", "Is this a refund request?", "--boolean"],
                                "I was charged twice", &stub.url, Some("k"));
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "true");
    assert_eq!(stub.request()["questions"]["answer"]["type"], "noul");
}

#[test]
fn a_low_probability_is_exit_1_but_the_decision_still_prints() {
    let stub = Stub::start(200, decision("prose", 0.41));
    let (stdout, stderr, code) = run(
        &["--ask", "What kind of task is this?", "--choice", "code,prose,math", "--min-prob", "0.7"],
        "maybe write something", &stub.url, Some("k"));
    assert_eq!(code, 1, "the gate failed");
    assert_eq!(stdout.trim(), "prose", "a caller can still see what it decided");
    assert!(stderr.contains("below --min-prob"), "{stderr}");
}

#[test]
fn expect_mismatch_is_exit_1() {
    let stub = Stub::start(200, decision("math", 0.95));
    let (stdout, stderr, code) = run(
        &["--ask", "What kind of task is this?", "--choice", "code,prose,math", "--expect", "code"],
        "2+2", &stub.url, Some("k"));
    assert_eq!(code, 1);
    assert_eq!(stdout.trim(), "math");
    assert!(stderr.contains("expected \"code\", decided \"math\""), "{stderr}");
}

#[test]
fn several_questions_print_one_json_object_with_every_answer() {
    let stub = Stub::start(200, json!({"model": "jev-1.13.0", "answers": {
        "team": {"type": "choice", "choice": "billing", "probabilities": {"billing": 0.93, "technical": 0.07}},
        "blocked": {"type": "noul", "noul": 0.71},
        "severity": {"type": "score", "score": 1.8, "probabilities": {"0": 0.05, "1": 0.2, "2": 0.75}},
    }, "usage": {"input_tokens": 120, "output_tokens": 9}}));
    let checks = write_checks(r#"{
      "team":     {"type": "choice", "instructions": "Which team owns this?", "criteria": {"billing": "payments", "technical": "errors"}},
      "blocked":  {"type": "boolean", "instructions": "Is the user blocked?"},
      "severity": {"type": "score", "instructions": "How bad?", "criteria": ["minor", "material", "critical"]}
    }"#);
    let (stdout, stderr, code) = run(&["--checks", checks.to_str().unwrap(), "--json"],
                                    "charged twice and cannot log in", &stub.url, Some("k"));
    assert_eq!(code, 0, "stderr: {stderr}");
    let out: Value = serde_json::from_str(&stdout).expect("stdout is JSON");
    assert_eq!(out["answers"]["team"]["value"], "billing");
    assert_eq!(out["answers"]["team"]["probability"], 0.93);
    assert_eq!(out["answers"]["blocked"]["value"], true);
    // a fractional score is reported with the level a script routes on, plus the raw
    assert_eq!(out["answers"]["severity"]["value"], 2);
    assert_eq!(out["answers"]["severity"]["score"], 1.8);
    assert_eq!(out["gate"]["ok"], true);
    assert_eq!(out["provider"], "typesafe");
    assert_eq!(out["model"], "jev-1.13.0");
    assert_eq!(out["usage"]["input_tokens"], 120);
}

#[test]
fn a_local_server_needs_no_credentials_and_no_authorization_header() {
    // kev serves the same /v1/systemone shape with no auth, so the local provider
    // must work with an empty environment and must not invent a Bearer header.
    let stub = Stub::start(200, json!({"model": "kev-latest", "answers": {
        "answer": {"type": "choice", "choice": "billing", "probabilities": {"billing": 0.89}},
    }, "usage": {"input_tokens": 101, "output_tokens": 161}}));
    let (stdout, stderr, code) = run_provider(
        &["--ask", "Which team?", "--choice", "billing,shipping"], "charged twice",
        &stub.url, None, "kev");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "billing");
    assert!(!stderr.contains("TYPESAFE_API_KEY"), "no key is required: {stderr}");

    let sent = stub.request();
    assert_eq!(sent["model"], "kev-latest", "the local server's default model");
    assert_eq!(sent["questions"]["answer"]["type"], "choice");
}

#[test]
fn a_local_server_is_still_asked_with_its_own_base_url() {
    // --base-url is how a kev on another port (or another host) is reached.
    let stub = Stub::start(200, decision("code", 0.9));
    let (_, stderr, code) = run_provider(&["--ask", "Which?", "--choice", "code,prose"], "x",
                                         &stub.url, None, "kev");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stub.request()["state"] == "x");
}

#[test]
fn without_credentials_it_exits_3_and_says_which_variable() {
    let stub = Stub::start(200, decision("code", 0.9));
    let (_, stderr, code) = run(&["--ask", "Which?", "--choice", "code,prose"], "x", &stub.url, None);
    assert_eq!(code, 3);
    assert!(stderr.contains("TYPESAFE_API_KEY"), "{stderr}");
}

#[test]
fn a_provider_error_surfaces_its_own_message_with_exit_3() {
    let stub = Stub::start(422, json!({"detail": "criteria for `answer` must not be empty"}));
    let (_, stderr, code) = run(&["--ask", "Which?", "--choice", "code,prose"], "x", &stub.url, Some("k"));
    assert_eq!(code, 3);
    assert!(stderr.contains("HTTP 422"), "{stderr}");
    assert!(stderr.contains("must not be empty"), "the provider's reason, not ours: {stderr}");
}

#[test]
fn usage_errors_are_exit_2_and_never_ask_the_provider() {
    let stub = Stub::start(200, decision("code", 0.9));
    let (_, stderr, code) = run(&["--ask", "Which?"], "x", &stub.url, Some("k"));
    assert_eq!(code, 2);
    assert!(stderr.contains("--choice, --boolean or --score"), "{stderr}");

    let (_, _, code) = run(&["--ask", "Which?", "--choice", "code,prose"], "   ", &stub.url, Some("k"));
    assert_eq!(code, 2, "an empty state is a usage error, not a decision");
}

fn write_checks(body: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("clank-jev-checks-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("checks.json");
    std::fs::write(&path, body).expect("write checks");
    path
}
