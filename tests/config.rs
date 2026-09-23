//! `.clank/config.toml` from outside the process: who reads which section, and
//! which layer wins. Stubs only. Nothing here reaches a model or a search API.

use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

const CLANK: &str = env!("CARGO_BIN_EXE_clank");
const JEV: &str = env!("CARGO_BIN_EXE_clank-jev");
const WEB: &str = env!("CARGO_BIN_EXE_clank-web");

struct Req {
    method: String,
    target: String,
    authorization: Option<String>,
    body: String,
}

struct Stub {
    url: String,
    seen: Receiver<Req>,
}

impl Stub {
    /// `base` is appended to `http://addr` (`/v1` for clank, `/search` for clank-web).
    fn start(base: &str) -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}{base}");
        let (tx, seen) = channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let Ok(req) = read_request(&mut stream) else { break };
                if tx.send(req).is_err() {
                    break;
                }
                let body = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"pong\"},\"finish_reason\":null}]}\n\n\
                            data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
                            data: [DONE]\n\n";
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.flush();
            }
        });
        Stub { url, seen }
    }

    fn only(&self) -> Req {
        let req = self.seen.recv_timeout(Duration::from_secs(5)).expect("one request");
        assert!(self.seen.try_recv().is_err(), "a second request was sent");
        req
    }

    fn none(&self) {
        assert!(self.seen.recv_timeout(Duration::from_millis(200)).is_err(), "this stub was contacted");
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> std::io::Result<Req> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut first = String::new();
    if reader.read_line(&mut first)? == 0 {
        return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "empty request"));
    }
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let mut authorization = None;
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if name == "content-length" {
                length = value.parse().unwrap_or(0);
            } else if name == "authorization" {
                authorization = Some(value);
            }
        }
    }
    let mut body = vec![0u8; length];
    if length > 0 {
        reader.read_exact(&mut body)?;
    }
    Ok(Req { method, target, authorization, body: String::from_utf8_lossy(&body).into_owned() })
}

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("clank-cfg-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Tmp(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn clear(cmd: &mut Command) {
    for key in [
        "CLANK_MODEL",
        "CLANK_BASE_URL",
        "CLANK_API_KEY",
        "CLANK_TIMEOUT",
        "CLANK_SYSTEM",
        "CLANK_DEBUG",
        "BRAVE_API_KEY",
        "TAVILY_API_KEY",
        "FIRECRAWL_API_KEY",
        "SEARXNG_API_KEY",
        "EXA_API_KEY",
        "PERPLEXITY_API_KEY",
        "CLANK_WEB_TEST_KEY",
        "TYPESAFE_API_KEY",
        "JEV_API_KEY",
        "JEV_CLI_API_KEY",
        "OPENROUTER_API_KEY",
        "CLANK_CONFIG",
    ] {
        cmd.env_remove(key);
    }
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn spawn(bin: &str, dir: &Path, args: &[&str], env: &[(&str, &str)], stdin: Option<&str>) -> Out {
    let mut cmd = Command::new(bin);
    cmd.args(args).current_dir(dir).stdout(Stdio::piped()).stderr(Stdio::piped());
    clear(&mut cmd);
    for (k, v) in env {
        cmd.env(k, v);
    }
    if stdin.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    let mut child = cmd.spawn().expect("spawn");
    if let Some(text) = stdin {
        let _ = child.stdin.as_mut().unwrap().write_all(text.as_bytes());
    }
    let out = child.wait_with_output().expect("wait");
    Out {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn body_of(req: &Req) -> Value {
    serde_json::from_str(&req.body).unwrap_or_else(|e| panic!("{e}: {}", req.body))
}

#[test]
fn a_missing_file_leaves_flags_and_the_environment_in_charge() {
    let dir = Tmp::new();
    let listed = spawn(CLANK, dir.path(), &["--list-tools"], &[], None);
    assert_eq!(listed.code, 0, "{}", listed.stderr);
    let tools: Value = serde_json::from_str(&listed.stdout).unwrap();
    let names: Vec<&str> = tools.as_array().unwrap().iter().map(|t| t["function"]["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["read_file", "list_dir", "search", "stat"]);

    let missing = spawn(CLANK, dir.path(), &["--no-tools", "-m", "hi"], &[], None);
    assert_eq!(missing.code, 2, "{}", missing.stderr);
    assert!(missing.stdout.is_empty(), "{}", missing.stdout);
    assert!(missing.stderr.contains("CLANK_MODEL") || missing.stderr.contains("--model"), "{}", missing.stderr);
    assert!(!missing.stderr.contains("40583"), "{}", missing.stderr);
    assert!(!missing.stderr.contains("qwen"), "{}", missing.stderr);

    let stub = Stub::start("/v1");
    let ran = spawn(
        CLANK,
        dir.path(),
        &["--no-tools", "-m", "hi"],
        &[("CLANK_BASE_URL", &stub.url), ("CLANK_MODEL", "from-env")],
        None,
    );
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert!(ran.stdout.contains("pong"), "{}", ran.stdout);
    let body = body_of(&stub.only());
    assert_eq!(body["model"], "from-env");
    assert_eq!(body["max_tokens"], 8192, "the built-in cap, with no file to override it");
    assert!(!body.to_string().contains("40583"));
    assert!(!body.to_string().contains("qwen"));
}

#[test]
fn a_clank_section_sets_the_endpoint_and_a_web_section_does_not() {
    let chat = Stub::start("/v1");
    let dir = Tmp::new();
    std::fs::create_dir(dir.path().join(".clank")).unwrap();
    std::fs::write(
        dir.path().join(".clank/config.toml"),
        format!("[clank]\nbase_url = \"{}\"\nmodel = \"from-file\"\nmax_tokens = 123\n", chat.url),
    )
    .unwrap();

    let ran = spawn(CLANK, dir.path(), &["--no-tools", "-m", "hi"], &[], None);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    let body = body_of(&chat.only());
    assert_eq!(body["model"], "from-file");
    assert_eq!(body["max_tokens"], 123);
    assert!(!ran.stdout.contains("40583"));
    assert!(!ran.stderr.contains("40583"));
    assert!(!body.to_string().contains("qwen"));

    let (search_url, seen) = json_once(
        br#"{"web":{"results":[{"title":"C","url":"https://c.example","description":"cfg"}]}}"#,
    );
    let web = spawn(
        WEB,
        dir.path(),
        &["--base-url", &search_url, "hello"],
        &[("BRAVE_API_KEY", "brave-secret")],
        None,
    );
    assert_eq!(web.code, 0, "{}", web.stderr);
    let req = seen.recv_timeout(Duration::from_secs(5)).expect("search request");
    assert_eq!(req.method, "GET");
    assert!(req.target.contains("q=hello"), "{}", req.target);
    assert!(req.target.contains("count=5"), "{}", req.target);
    assert!(!web.stdout.contains("brave-secret"), "{}", web.stdout);
    assert!(!web.stderr.contains("brave-secret"), "{}", web.stderr);
    chat.none();
}

#[test]
fn flags_override_the_file_and_the_environment() {
    let configured = Stub::start("/v1");
    let flagged = Stub::start("/v1");
    let dir = Tmp::new();
    std::fs::create_dir(dir.path().join(".clank")).unwrap();
    std::fs::write(
        dir.path().join(".clank/config.toml"),
        format!("[clank]\nbase_url = \"{}\"\nmodel = \"from-file\"\nmax_tokens = 123\n", configured.url),
    )
    .unwrap();
    let ran = spawn(
        CLANK,
        dir.path(),
        &["--no-tools", "--model", "from-flag", "--base-url", &flagged.url, "--max-tokens", "50", "-m", "hi"],
        &[("CLANK_MODEL", "from-env"), ("CLANK_BASE_URL", &configured.url)],
        None,
    );
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    let body = body_of(&flagged.only());
    assert_eq!(body["model"], "from-flag");
    assert_eq!(body["max_tokens"], 50);
    configured.none();
}

#[test]
fn a_config_in_a_parent_directory_is_not_read() {
    let stub = Stub::start("/v1");
    let root = Tmp::new();
    let parent = root.path().join("parent");
    let child = parent.join("child");
    std::fs::create_dir_all(parent.join(".clank")).unwrap();
    std::fs::create_dir_all(&child).unwrap();
    std::fs::write(
        parent.join(".clank/config.toml"),
        format!("[clank]\nbase_url = \"{}\"\nmodel = \"from-parent\"\n", stub.url),
    )
    .unwrap();

    let ignored = spawn(CLANK, &child, &["--no-tools", "-m", "hi"], &[], None);
    assert_eq!(ignored.code, 2, "{}", ignored.stderr);
    assert!(ignored.stdout.is_empty(), "{}", ignored.stdout);
    assert!(!ignored.stderr.contains("from-parent"), "{}", ignored.stderr);
    stub.none();

    std::fs::create_dir_all(child.join(".clank")).unwrap();
    std::fs::write(
        child.join(".clank/config.toml"),
        format!("[clank]\nbase_url = \"{}\"\nmodel = \"from-child\"\n", stub.url),
    )
    .unwrap();
    let from_child = spawn(CLANK, &child, &["--no-tools", "-m", "hi"], &[], None);
    assert_eq!(from_child.code, 0, "{}", from_child.stderr);
    assert_eq!(body_of(&stub.only())["model"], "from-child");
}

#[test]
fn an_explicit_config_path_overrides_the_working_directory() {
    let stub = Stub::start("/v1");
    let dir = Tmp::new();
    std::fs::create_dir(dir.path().join(".clank")).unwrap();
    std::fs::write(
        dir.path().join(".clank/config.toml"),
        format!("[clank]\nbase_url = \"{}\"\nmodel = \"from-cwd\"\n", stub.url),
    )
    .unwrap();
    let flagged = dir.path().join("flagged.toml");
    let from_env = dir.path().join("from-env.toml");
    std::fs::write(&flagged, format!("[clank]\nbase_url = \"{}\"\nmodel = \"from-flag\"\n", stub.url)).unwrap();
    std::fs::write(&from_env, format!("[clank]\nbase_url = \"{}\"\nmodel = \"from-env-path\"\n", stub.url)).unwrap();
    let flagged_s = flagged.to_str().unwrap();
    let from_env_s = from_env.to_str().unwrap();

    let by_flag = spawn(CLANK, dir.path(), &["--no-tools", "--config", flagged_s, "-m", "hi"], &[("CLANK_CONFIG", from_env_s)], None);
    assert_eq!(by_flag.code, 0, "{}", by_flag.stderr);
    assert_eq!(body_of(&stub.only())["model"], "from-flag");

    let by_env = spawn(CLANK, dir.path(), &["--no-tools", "-m", "hi"], &[("CLANK_CONFIG", from_env_s)], None);
    assert_eq!(by_env.code, 0, "{}", by_env.stderr);
    assert_eq!(body_of(&stub.only())["model"], "from-env-path");

    let missing = spawn(CLANK, dir.path(), &["--no-tools", "--config", "/no/such/clank.toml", "-m", "hi"], &[], None);
    assert_eq!(missing.code, 2, "{}", missing.stderr);
    assert!(missing.stdout.is_empty(), "{}", missing.stdout);
    assert!(missing.stderr.contains("reading"), "{}", missing.stderr);

    let missing_env = spawn(CLANK, dir.path(), &["--no-tools", "-m", "hi"], &[("CLANK_CONFIG", "/no/such/clank.toml")], None);
    assert_eq!(missing_env.code, 2, "{}", missing_env.stderr);
    assert!(missing_env.stderr.contains("reading"), "{}", missing_env.stderr);

    let jev = spawn(
        JEV,
        dir.path(),
        &["--config", "/no/such/clank.toml", "--provider", "typesafe", "--ask", "which?", "--choice", "a,b"],
        &[],
        Some("state"),
    );
    assert_eq!(jev.code, 2, "{}", jev.stderr);
}

fn json_once(payload: &'static [u8]) -> (String, Receiver<Req>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/search", listener.local_addr().unwrap());
    let (tx, seen) = channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let req = read_request(&mut stream).unwrap();
        let _ = tx.send(req);
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(payload);
    });
    (url, seen)
}

#[test]
fn the_web_section_overrides_the_brave_default() {
    let (url, seen) = json_once(br#"{"results":[{"title":"T","url":"https://t.example","content":"body"}]}"#);
    let dir = Tmp::new();
    std::fs::create_dir(dir.path().join(".clank")).unwrap();
    std::fs::write(
        dir.path().join(".clank/config.toml"),
        "[clank]\nbase_url = \"http://127.0.0.1:1/v1\"\nmodel = \"poison\"\n\n\
         [web]\ndefault_provider = \"tavily\"\nlimit = 2\nformat = \"text\"\n",
    )
    .unwrap();
    let ran = spawn(WEB, dir.path(), &["--base-url", &url, "widgets"], &[("TAVILY_API_KEY", "tvly-test")], None);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert_eq!(ran.stdout.trim(), "T\thttps://t.example\tbody");
    let req = seen.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req.method, "POST");
    let body: Value = serde_json::from_str(&req.body).unwrap();
    assert_eq!(body["query"], "widgets");
    assert_eq!(body["max_results"], 2);
    assert_eq!(body["search_depth"], "basic");
    assert!(body.get("api_key").is_none(), "{body}");
    assert_eq!(req.authorization.as_deref(), Some("Bearer tvly-test"));
    assert!(!ran.stderr.contains("tvly-test"), "{}", ran.stderr);
    assert!(!ran.stderr.contains("poison"), "{}", ran.stderr);
}

#[test]
fn brave_flags_override_a_tavily_default() {
    let (url, seen) = json_once(
        br#"{"web":{"results":[{"title":"A","url":"https://a.example","description":"snippet"}]}}"#,
    );
    let dir = Tmp::new();
    std::fs::create_dir(dir.path().join(".clank")).unwrap();
    std::fs::write(
        dir.path().join(".clank/config.toml"),
        "[web]\ndefault_provider = \"tavily\"\nlimit = 2\nformat = \"text\"\n",
    )
    .unwrap();
    let ran = spawn(
        WEB,
        dir.path(),
        &["--provider", "brave", "--limit", "3", "--format", "jsonl", "--base-url", &url, "hello web"],
        &[("BRAVE_API_KEY", "brave-secret")],
        None,
    );
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    let hit: Value = serde_json::from_str(ran.stdout.lines().next().unwrap()).unwrap();
    assert_eq!(hit["title"], "A");
    assert!(hit.get("truncated").is_none());
    let req = seen.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req.method, "GET");
    assert!(req.target.contains("count=3"), "{}", req.target);
    assert!(req.target.contains("q=hello%20web"), "{}", req.target);
    assert!(!ran.stderr.contains("brave-secret"), "{}", ran.stderr);
}

#[test]
fn a_brave_key_does_not_become_a_clank_or_jev_credential() {
    let chat = Stub::start("/v1");
    let dir = Tmp::new();
    std::fs::create_dir(dir.path().join(".clank")).unwrap();
    std::fs::write(dir.path().join(".clank/config.toml"), "[web.brave]\napi_key_env = \"BRAVE_API_KEY\"\n").unwrap();

    let listed = spawn(CLANK, dir.path(), &["--list-tools"], &[("BRAVE_API_KEY", "brave-secret")], None);
    assert_eq!(listed.code, 0, "{}", listed.stderr);
    assert!(!listed.stdout.contains("brave-secret"));

    let ran = spawn(
        CLANK,
        dir.path(),
        &["--no-tools", "--model", "stub", "--base-url", &chat.url, "-m", "hi"],
        &[("BRAVE_API_KEY", "brave-secret")],
        None,
    );
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    let req = chat.only();
    assert!(req.authorization.is_none(), "a Brave key is not a chat key: {:?}", req.authorization);
    assert!(!ran.stdout.contains("brave-secret"), "{}", ran.stdout);
    assert!(!ran.stderr.contains("brave-secret"), "{}", ran.stderr);

    let jev = spawn(JEV, dir.path(), &["--provider", "typesafe", "--ask", "which?", "--choice", "a,b"], &[("BRAVE_API_KEY", "brave-secret")], Some("state"));
    assert_eq!(jev.code, 3, "{}", jev.stderr);
    assert!(jev.stdout.is_empty(), "{}", jev.stdout);
    assert!(!jev.stderr.contains("brave-secret"), "{}", jev.stderr);
    assert!(jev.stderr.contains("TYPESAFE_API_KEY"), "{}", jev.stderr);
}

#[test]
fn an_exa_section_does_not_become_a_jev_credential() {
    let dir = Tmp::new();
    std::fs::create_dir(dir.path().join(".clank")).unwrap();
    std::fs::write(dir.path().join(".clank/config.toml"), "[web.exa]\napi_key_env = \"EXA_API_KEY\"\n").unwrap();
    let jev = spawn(
        JEV,
        dir.path(),
        &["--provider", "typesafe", "--ask", "which?", "--choice", "a,b"],
        &[("EXA_API_KEY", "exa-secret")],
        Some("state"),
    );
    assert_eq!(jev.code, 3, "{}", jev.stderr);
    assert!(jev.stdout.is_empty(), "{}", jev.stdout);
    assert!(!jev.stderr.contains("exa-secret"), "{}", jev.stderr);
    assert!(jev.stderr.contains("TYPESAFE_API_KEY"), "{}", jev.stderr);

    let listed = spawn(CLANK, dir.path(), &["--list-tools"], &[("EXA_API_KEY", "exa-secret")], None);
    assert_eq!(listed.code, 0, "{}", listed.stderr);
    assert!(!listed.stdout.contains("exa-secret"));
}

#[test]
fn an_inline_key_is_the_fallback_and_stays_out_of_the_log() {
    let stub = Stub::start("/v1");
    let dir = Tmp::new();
    std::fs::create_dir(dir.path().join(".clank")).unwrap();
    std::fs::write(
        dir.path().join(".clank/config.toml"),
        format!("[clank]\nbase_url = \"{}\"\nmodel = \"m\"\napi_key = \"inline-secret\"\n", stub.url),
    )
    .unwrap();
    let ran = spawn(CLANK, dir.path(), &["--no-tools", "-m", "hi"], &[], None);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert_eq!(stub.only().authorization.as_deref(), Some("Bearer inline-secret"));
    assert!(!ran.stdout.contains("inline-secret"), "{}", ran.stdout);
    assert!(!ran.stderr.contains("inline-secret"), "{}", ran.stderr);

    let stub = Stub::start("/v1");
    std::fs::write(
        dir.path().join(".clank/config.toml"),
        format!("[clank]\nbase_url = \"{}\"\nmodel = \"m\"\napi_key_env = \"CLANK_API_KEY\"\napi_key = \"inline-secret\"\n", stub.url),
    )
    .unwrap();
    let ran = spawn(CLANK, dir.path(), &["--no-tools", "-m", "hi"], &[("CLANK_API_KEY", "from-env")], None);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert_eq!(stub.only().authorization.as_deref(), Some("Bearer from-env"));
    assert!(!ran.stderr.contains("inline-secret"), "{}", ran.stderr);
    assert!(!ran.stderr.contains("from-env"), "{}", ran.stderr);
}
