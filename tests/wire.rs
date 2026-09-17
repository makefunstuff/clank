//! What clank sends, and what it writes, observed from outside the process.
//!
//! The stub speaks just enough HTTP/1.1 + SSE to satisfy the real client and it
//! records every request body, so these assertions are made against the bytes
//! on the wire rather than against clank's own opinion of what it sent.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::thread;

const SCHEMA: &str = r#"{"type":"object","properties":{"script":{"type":"string"}},"required":["script"]}"#;

/// One scripted round: what the stub streams back the first time it is asked.
enum Reply {
    Text(&'static str),
    /// Text that the server cut off at the token budget.
    Truncated(&'static str),
    /// A round with neither text nor a tool call (all budget spent thinking).
    Empty,
    /// A round that streams reasoning before the answer.
    Thinking {
        thinking: &'static str,
        text: &'static str,
    },
    ToolCall {
        name: &'static str,
        arguments: &'static str,
    },
    HttpError(u16),
}

struct Stub {
    base_url: String,
    seen: Receiver<Value>,
}

impl Stub {
    fn start(replies: Vec<Reply>) -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let (tx, seen) = channel();

        thread::spawn(move || {
            let mut remaining = replies;
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let Some(body) = read_request(&mut stream) else {
                    continue;
                };
                if tx.send(body).is_err() {
                    break;
                }
                let reply = if remaining.is_empty() {
                    Reply::HttpError(500)
                } else {
                    remaining.remove(0)
                };
                let _ = stream.write_all(&reply.response());
                let _ = stream.flush();
            }
        });

        Stub { base_url, seen }
    }

    /// Every request the stub answered. Drained; call once per test, after clank
    /// has exited.
    fn requests(&self) -> Vec<Value> {
        self.seen.try_iter().collect()
    }
}

impl Reply {
    fn response(&self) -> Vec<u8> {
        let body = match self {
            Reply::HttpError(code) => {
                return format!(
                    "HTTP/1.1 {code} Error\r\nContent-Type: application/json\r\n\
                     Content-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                )
                .into_bytes()
            }
            other => other
                .chunks()
                .iter()
                .map(|c| format!("data: {c}\n\n"))
                .collect::<String>()
                + "data: [DONE]\n\n",
        };
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    /// The SSE chunks of one round, in the shape llama.cpp streams them.
    fn chunks(&self) -> Vec<Value> {
        match self {
            Reply::Text(text) => vec![
                json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]}),
                json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
            ],
            Reply::Truncated(text) => vec![
                json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]}),
                json!({"choices":[{"index":0,"delta":{},"finish_reason":"length"}]}),
            ],
            Reply::Empty => vec![json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})],
            Reply::Thinking { thinking, text } => vec![
                json!({"choices":[{"index":0,"delta":{"reasoning_content":thinking},"finish_reason":null}]}),
                json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]}),
                json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
            ],
            Reply::ToolCall { name, arguments } => vec![
                json!({"choices":[{"index":0,"delta":{"tool_calls":[{
                    "index":0,"id":"call_1","type":"function",
                    "function":{"name":name,"arguments":arguments}}]},"finish_reason":null}]}),
                json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
            ],
            Reply::HttpError(_) => Vec::new(),
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Option<Value> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut len = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(v) = trimmed.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().ok()?;
        }
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn clank(args: &[&str], stdin: &str) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_clank"))
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env_remove("CLANK_DEBUG")
        .env_remove("CLANK_SYSTEM")
        .env_remove("CLANK_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn clank");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    let out: Output = child.wait_with_output().expect("wait clank");
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn tool_count(request: &Value) -> usize {
    request["tools"].as_array().map(Vec::len).unwrap_or(0)
}

fn messages(request: &Value) -> &Vec<Value> {
    request["messages"].as_array().expect("messages array")
}

/// Everything except the trailing per-item block: identical request to request
/// means the server can reuse the prompt prefix across items.
fn shared_prefix(request: &Value) -> Value {
    let all = messages(request);
    json!(all[..all.len() - 1].to_vec())
}

#[test]
fn the_schema_never_travels_in_the_same_request_as_tools() {
    let stub = Stub::start(vec![
        Reply::ToolCall {
            name: "read_file",
            arguments: r#"{"path": "fixtures/notes.md"}"#,
        },
        Reply::Text("The fixture describes the userData struct."),
        Reply::Text(r#"{"script": "ok"}"#),
    ]);
    let run = clank(
        &[
            "--base-url",
            &stub.base_url,
            "--model",
            "stub",
            "--max-tokens",
            "32",
            "--tools",
            "-m",
            "write a script",
            "--json-schema",
            SCHEMA,
        ],
        "",
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout.trim(), r#"{"script": "ok"}"#);

    let reqs = stub.requests();
    assert_eq!(reqs.len(), 3, "tool round, its follow-up, then the constrained answer");
    for (n, req) in reqs.iter().enumerate() {
        assert!(
            !(tool_count(req) > 0 && req.get("json_schema").is_some()),
            "request {n} carried tools and a schema together: {req}"
        );
    }
    assert!(tool_count(&reqs[0]) > 0, "the first round should offer tools");
    let answer = &reqs[2];
    assert!(answer.get("json_schema").is_some(), "the answer must come from a schema'd request");
    assert_eq!(tool_count(answer), 0, "the schema'd request must not offer tools");
    assert!(
        answer["messages"].to_string().contains("anonymize"),
        "the constrained round must see what the tool round read: {}",
        answer["messages"]
    );
}

#[test]
fn the_default_is_one_prompt_one_request_and_no_tools() {
    let stub = Stub::start(vec![Reply::Text("pong")]);
    let run = clank(
        &["--base-url", &stub.base_url, "--model", "stub", "-m", "ping"],
        "",
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "pong", "stdout is the answer and nothing else");
    let reqs = stub.requests();
    assert_eq!(reqs.len(), 1, "a single-shot prompt is a single request");
    assert_eq!(
        tool_count(&reqs[0]),
        0,
        "tools are opt-in: the context is what was piped"
    );
    let system = messages(&reqs[0])[0]["content"].as_str().unwrap();
    assert!(
        !system.contains("read_file"),
        "the default system prompt must not advertise tools the model does not have: {system}"
    );
}

#[test]
fn a_schema_without_tools_is_one_request() {
    let stub = Stub::start(vec![Reply::Text(r#"{"script": "blind"}"#)]);
    let run = clank(
        &[
            "--base-url",
            &stub.base_url,
            "--model",
            "stub",
            "-m",
            "write a script",
            "--json-schema",
            SCHEMA,
        ],
        "",
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout.trim(), r#"{"script": "blind"}"#);
    let reqs = stub.requests();
    assert_eq!(reqs.len(), 1, "no tools, no exploration: one request");
    assert!(reqs[0].get("json_schema").is_some());
    assert_eq!(tool_count(&reqs[0]), 0);
}

#[test]
fn a_truncated_answer_is_a_failure_not_a_partial_result() {
    let stub = Stub::start(vec![Reply::Truncated("The fixture describes")]);
    let run = clank(
        &["--base-url", &stub.base_url, "--model", "stub", "-m", "describe"],
        "",
    );

    assert_eq!(run.code, 1, "half an answer must not exit 0");
    assert!(run.stderr.contains("truncated"), "stderr: {}", run.stderr);
}

#[test]
fn an_empty_answer_is_a_failure_not_an_empty_success() {
    let stub = Stub::start(vec![Reply::Empty]);
    let run = clank(
        &["--base-url", &stub.base_url, "--model", "stub", "-m", "describe"],
        "",
    );

    assert_eq!(run.code, 1, "no answer must not exit 0");
    assert_eq!(run.stdout, "");
    assert!(run.stderr.contains("no answer"), "stderr: {}", run.stderr);
}

#[test]
fn a_piped_evidence_node_is_not_dropped_when_context_files_are_given() {
    let stub = Stub::start(vec![Reply::Text("both")]);
    let run = clank(
        &[
            "--base-url",
            &stub.base_url,
            "--model",
            "stub",
            "-c",
            "fixtures/notes.md",
            "-m",
            "what is this",
        ],
        "PIPED-EVIDENCE\n",
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let reqs = stub.requests();
    assert_eq!(reqs.len(), 1);
    let context = messages(&reqs[0])[1]["content"].as_str().unwrap().to_string();
    assert!(
        context.contains("anonymize"),
        "the context file must be there: {context}"
    );
    assert!(
        context.contains("PIPED-EVIDENCE"),
        "the pipe must not be dropped when -c is used: {context}"
    );
}

#[test]
fn a_schema_violating_answer_is_a_failure_not_an_empty_success() {
    let stub = Stub::start(vec![Reply::Text("I could not do that.")]);
    let run = clank(
        &[
            "--base-url",
            &stub.base_url,
            "--model",
            "stub",
            "-m",
            "write a script",
            "--json-schema",
            SCHEMA,
        ],
        "",
    );

    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("not valid JSON"), "stderr: {}", run.stderr);
}

#[test]
fn each_item_is_framed_indexed_and_shares_the_prompt_prefix() {
    let stub = Stub::start(vec![Reply::Text("one"), Reply::Text("two")]);
    let run = clank(
        &[
            "--base-url",
            &stub.base_url,
            "--model",
            "stub",
            "-m",
            "describe this item",
            "--each",
        ],
        "alpha\nbeta\n",
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    // Answers here do not end their own line, so the frames only line up if
    // clank terminates them: header, answer, blank line, header, answer.
    assert_eq!(run.stdout, "─── item 1/2 ───\none\n\n─── item 2/2 ───\ntwo\n");

    let reqs = stub.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(shared_prefix(&reqs[0]), shared_prefix(&reqs[1]));
    let last = |r: &Value| {
        messages(r)
            .last()
            .unwrap()["content"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert!(last(&reqs[0]).contains("item 1/2") && last(&reqs[0]).contains("alpha"));
    assert!(last(&reqs[1]).contains("item 2/2") && last(&reqs[1]).contains("beta"));
}

#[test]
fn each_emits_item_events_and_bad_items_as_errors() {
    let stub = Stub::start(vec![Reply::HttpError(500), Reply::Text("two")]);
    let run = clank(
        &[
            "--base-url",
            &stub.base_url,
            "--model",
            "stub",
            "-m",
            "describe this item",
            "--each",
            "--jsonl",
        ],
        "alpha\nbeta\n",
    );

    assert_eq!(run.code, 1, "an item failed, so the exit code has to say so");
    let events: Vec<Value> = run
        .stdout
        .lines()
        .map(|l| serde_json::from_str(l).expect("every stdout line is one event"))
        .collect();
    assert!(events
        .iter()
        .any(|e| e["type"] == "item" && e["i"] == 1 && e["input"] == "alpha"));
    assert!(events.iter().any(|e| e["type"] == "error" && e["i"] == 1));
    assert!(events
        .iter()
        .any(|e| e["type"] == "assistant" && e["i"] == 2 && e["content"] == "two"));
    assert!(run.stderr.contains("item 1/2"), "stderr: {}", run.stderr);
}

#[test]
fn each_with_a_schema_answers_every_item_with_json() {
    let stub = Stub::start(vec![
        Reply::Text("risk looks low"),
        Reply::Text(r#"{"risk": 1}"#),
        Reply::Text("risk looks high"),
        Reply::Text(r#"{"risk": 5}"#),
    ]);
    let run = clank(
        &[
            "--base-url",
            &stub.base_url,
            "--model",
            "stub",
            "-m",
            "rate the risk",
            "--each",
            "--jsonl",
            "--tools",
            "--json-schema",
            r#"{"type":"object","properties":{"risk":{"type":"integer"}},"required":["risk"]}"#,
        ],
        "a.rs\nb.rs\n",
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let events: Vec<Value> = run
        .stdout
        .lines()
        .map(|l| serde_json::from_str(l).expect("every stdout line is one event"))
        .collect();
    let answers: Vec<&Value> = events
        .iter()
        .filter(|e| e["type"] == "assistant")
        .collect();
    assert_eq!(answers.len(), 2, "one answer per item: {events:?}");
    assert_eq!(answers[0]["i"], 1);
    assert_eq!(answers[0]["content"], r#"{"risk": 1}"#);
    assert_eq!(answers[1]["i"], 2);
    assert_eq!(answers[1]["content"], r#"{"risk": 5}"#);
    assert!(
        !events.iter().any(|e| e["type"] == "assistant" && e["content"] == "risk looks low"),
        "intermediate commentary is not an answer: {events:?}"
    );

    let reqs = stub.requests();
    assert_eq!(reqs.len(), 4, "two requests per item: exploration, then the schema");
    for (n, req) in reqs.iter().enumerate() {
        assert!(
            !(tool_count(req) > 0 && req.get("json_schema").is_some()),
            "request {n} carried tools and a schema together: {req}"
        );
    }
}

#[test]
fn an_empty_item_list_is_nothing_to_do_not_a_failure() {
    let stub = Stub::start(vec![]);
    let run = clank(
        &[
            "--base-url",
            &stub.base_url,
            "--model",
            "stub",
            "-m",
            "describe this item",
            "--each",
        ],
        "\n\n",
    );

    assert_eq!(run.code, 0);
    assert_eq!(run.stdout, "");
    assert!(stub.requests().is_empty(), "an empty list must not call the model");
}

#[test]
fn each_without_a_prompt_is_a_usage_error() {
    let stub = Stub::start(vec![]);
    let run = clank(
        &["--base-url", &stub.base_url, "--model", "stub", "--each"],
        "alpha\n",
    );

    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("--each"), "stderr: {}", run.stderr);
    assert!(stub.requests().is_empty());
}

#[test]
fn thinking_off_is_sent_as_a_template_disable() {
    let stub = Stub::start(vec![Reply::Text("pong")]);
    let run = clank(
        &[
            "--base-url",
            &stub.base_url,
            "--model",
            "stub",
            "--thinking",
            "off",
            "-m",
            "ping",
        ],
        "",
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let req = &stub.requests()[0];
    assert_eq!(req["chat_template_kwargs"]["enable_thinking"], false);
    assert!(
        req.get("reasoning_effort").is_none(),
        "off is a template disable, not an effort level: {req}"
    );
}

#[test]
fn a_thinking_level_is_passed_through_and_a_bad_one_is_a_usage_error() {
    let stub = Stub::start(vec![Reply::Text("pong")]);
    let run = clank(
        &[
            "--base-url",
            &stub.base_url,
            "--model",
            "stub",
            "--thinking",
            "low",
            "-m",
            "ping",
        ],
        "",
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert_eq!(stub.requests()[0]["reasoning_effort"], "low");

    let bad = clank(&["--thinking", "sometimes", "-m", "ping"], "");
    assert_eq!(bad.code, 2, "an unknown level is a usage error");
    assert!(bad.stderr.contains("--thinking"), "stderr: {}", bad.stderr);
}

#[test]
fn the_default_sends_no_reasoning_control_at_all() {
    let stub = Stub::start(vec![Reply::Text("pong")]);
    clank(
        &["--base-url", &stub.base_url, "--model", "stub", "-m", "ping"],
        "",
    );
    let req = &stub.requests()[0];
    assert!(req.get("chat_template_kwargs").is_none(), "{req}");
    assert!(req.get("reasoning_effort").is_none(), "{req}");
}

#[test]
fn reasoning_is_visible_on_stderr_only_when_asked_for() {
    let reply = || {
        vec![Reply::Thinking {
            thinking: "let me think about pong",
            text: "pong",
        }]
    };

    let stub = Stub::start(reply());
    let hidden = clank(
        &["--base-url", &stub.base_url, "--model", "stub", "-m", "ping"],
        "",
    );
    assert_eq!(hidden.code, 0, "stderr: {}", hidden.stderr);
    assert_eq!(hidden.stdout, "pong", "stdout stays the answer alone");
    assert!(
        !hidden.stderr.contains("let me think about pong"),
        "thinking must not appear unless asked for: {}",
        hidden.stderr
    );

    let stub = Stub::start(reply());
    let shown = clank(
        &[
            "--base-url",
            &stub.base_url,
            "--model",
            "stub",
            "--show-thinking",
            "-m",
            "ping",
        ],
        "",
    );
    assert_eq!(shown.code, 0);
    assert!(
        shown.stdout == "pong",
        "thinking goes to stderr, never to stdout: {}",
        shown.stdout
    );
    assert!(
        shown.stderr.contains("let me think about pong"),
        "stderr: {}",
        shown.stderr
    );
}

#[test]
fn the_jsonl_stream_starts_with_provenance_and_keeps_keys_out_of_it() {
    let stub = Stub::start(vec![Reply::Text("pong")]);
    let run = clank(
        &[
            "--base-url",
            &stub.base_url,
            "--model",
            "stub",
            "--api-key",
            "sk-secret-value",
            "--jsonl",
            "-m",
            "ping",
        ],
        "",
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let first: Value =
        serde_json::from_str(run.stdout.lines().next().expect("a first line")).unwrap();
    assert_eq!(first["type"], "run");
    assert!(!first["clank"].as_str().unwrap_or("").is_empty());
    assert_eq!(first["model"], "stub");
    assert_eq!(first["base_url"], stub.base_url);
    assert!(
        !run.stdout.contains("sk-secret-value"),
        "an API key must never reach a trace: {}",
        run.stdout
    );
    assert!(
        first["argv"].to_string().contains("--model"),
        "argv is the provenance: {first}"
    );
}

#[test]
fn a_trace_with_a_run_header_still_reads_back_as_context() {
    let trace = std::env::temp_dir().join(format!("clank-wire-{}.jsonl", std::process::id()));
    std::fs::write(
        &trace,
        concat!(
            "{\"type\":\"run\",\"clank\":\"0.1.0\",\"model\":\"stub\"}\n",
            "{\"type\":\"assistant\",\"content\":\"the prior answer\"}\n"
        ),
    )
    .expect("write trace");

    let stub = Stub::start(vec![Reply::Text("ok")]);
    let run = clank(
        &[
            "--base-url",
            &stub.base_url,
            "--model",
            "stub",
            "-c",
            trace.to_str().unwrap(),
            "-m",
            "what did you say",
        ],
        "",
    );
    let _ = std::fs::remove_file(&trace);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let context = messages(&stub.requests()[0])[1]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        context.contains("the prior answer"),
        "the prior turn survives re-ingestion: {context}"
    );
}

#[test]
fn piped_json_reaches_the_model_even_when_it_looks_like_a_trace() {
    // `{"type":"function",…}` is ordinary JSON, not a clank event stream; it
    // must arrive as evidence rather than being swallowed or refused.
    let stub = Stub::start(vec![Reply::Text("seen")]);
    let run = clank(
        &[
            "--base-url",
            &stub.base_url,
            "--model",
            "stub",
            "-m",
            "what is this",
        ],
        "{\"type\":\"function\",\"function\":{\"name\":\"search\"}}\n",
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let context = messages(&stub.requests()[0])[1]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        context.contains("\"name\":\"search\""),
        "the pipe must reach the model: {context}"
    );
}

#[test]
fn a_schema_can_be_read_from_a_file() {
    // The loop this enables: `clank … > schema.json`, then `--json-schema @schema.json`.
    let path = std::env::temp_dir().join(format!("clank-schema-{}.json", std::process::id()));
    std::fs::write(&path, SCHEMA).unwrap();
    let stub = Stub::start(vec![Reply::Text(r#"{"script": "from file"}"#)]);
    let run = clank(
        &[
            "--base-url",
            &stub.base_url,
            "--model",
            "stub",
            "-m",
            "write a script",
            "--json-schema",
            &format!("@{}", path.display()),
        ],
        "",
    );
    let _ = std::fs::remove_file(&path);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout.trim(), r#"{"script": "from file"}"#);
    let req = &stub.requests()[0];
    assert_eq!(req["json_schema"]["required"][0], "script", "{req}");
}

#[test]
fn a_skill_can_be_read_from_a_file_into_the_system_prompt() {
    let path = std::env::temp_dir().join(format!("clank-skill-{}.md", std::process::id()));
    std::fs::write(&path, "Always answer with exactly: SKILLED").unwrap();
    let stub = Stub::start(vec![Reply::Text("SKILLED")]);
    let run = clank(
        &[
            "--base-url",
            &stub.base_url,
            "--model",
            "stub",
            "--system",
            &format!("@{}", path.display()),
            "-m",
            "hi",
        ],
        "",
    );
    let _ = std::fs::remove_file(&path);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let system = messages(&stub.requests()[0])[0]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        system.contains("SKILLED"),
        "the skill must reach the system prompt: {system}"
    );
}

