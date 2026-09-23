//! clank-web from outside the process: the request it sends, the lines it prints,
//! and the exit code a script branches on.
//!
//! The stub speaks plain HTTP/1.1. Nothing here reaches a search provider.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_clank-web");
const CLANK: &str = env!("CARGO_BIN_EXE_clank");

struct Req {
    method: String,
    target: String,
    headers: std::collections::HashMap<String, String>,
    body: String,
}

struct Stub {
    url: String,
    seen: Receiver<Req>,
}

impl Stub {
    fn start(status: u16, content_type: &str, body: &str) -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}/search");
        let (tx, seen) = channel();
        let content_type = content_type.to_string();
        let payload = body.as_bytes().to_vec();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let Ok(req) = read_request(&mut stream) else { break };
                if tx.send(req).is_err() {
                    break;
                }
                let head = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&payload);
                let _ = stream.flush();
            }
        });
        Stub { url, seen }
    }

    fn only(&self) -> Req {
        let req = self.seen.recv_timeout(Duration::from_secs(5)).expect("one request");
        assert!(self.seen.try_recv().is_err(), "clank-web sent a second request");
        req
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
    let mut headers = std::collections::HashMap::new();
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
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    if let Some(n) = headers.get("content-length") {
        length = n.parse().unwrap_or(0);
    }
    let mut body = vec![0u8; length];
    if length > 0 {
        reader.read_exact(&mut body)?;
    }
    let body = String::from_utf8_lossy(&body).into_owned();
    Ok(Req { method, target, headers, body })
}

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("clank-web-{}-{n}", std::process::id()));
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

fn run(dir: &Path, args: &[&str], env: &[(&str, &str)], stdin: Option<&str>) -> (String, String, i32) {
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .current_dir(dir)
        .env_remove("BRAVE_API_KEY")
        .env_remove("TAVILY_API_KEY")
        .env_remove("FIRECRAWL_API_KEY")
        .env_remove("SEARXNG_API_KEY")
        .env_remove("EXA_API_KEY")
        .env_remove("PERPLEXITY_API_KEY")
        .env_remove("CLANK_WEB_TEST_KEY")
        .env_remove("CLANK_CONFIG")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    if stdin.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    let mut child = cmd.spawn().expect("spawn clank-web");
    if let Some(text) = stdin {
        let _ = child.stdin.as_mut().expect("stdin").write_all(text.as_bytes());
    }
    let out = child.wait_with_output().expect("wait");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn lines(stdout: &str) -> Vec<Value> {
    stdout.lines().map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}"))).collect()
}

#[test]
fn a_brave_search_prints_jsonl_and_sends_the_token_header() {
    let stub = Stub::start(200, "application/json", &json!({"web": {"results": [
        {"title": "Alpha", "url": "https://a.example/x", "description": "first snippet"},
        {"title": "Beta", "url": "https://b.example/y", "description": "second"}
    ]}}).to_string());
    let dir = Tmp::new();
    let (stdout, stderr, code) = run(
        dir.path(),
        &["--provider", "brave", "--base-url", &stub.url, "--limit", "2", "hello web"],
        &[("BRAVE_API_KEY", "test-key")],
        None,
    );
    assert_eq!(code, 0, "{stderr}");
    let hits = lines(&stdout);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0]["title"], "Alpha");
    assert_eq!(hits[0]["url"], "https://a.example/x");
    assert_eq!(hits[0]["snippet"], "first snippet");
    assert!(hits[0].get("truncated").is_none());
    assert!(stderr.contains("2 results from brave"), "{stderr}");

    let req = stub.only();
    assert_eq!(req.method, "GET");
    assert!(req.target.contains("q=hello%20web"), "{}", req.target);
    assert!(req.target.contains("count=2"), "{}", req.target);
    assert_eq!(req.headers.get("x-subscription-token").map(String::as_str), Some("test-key"));
    assert!(req.body.is_empty());
}

#[test]
fn an_http_error_is_exit_1_and_stdout_stays_empty() {
    let stub = Stub::start(401, "application/json", r#"{"error":"bad token"}"#);
    let dir = Tmp::new();
    let (stdout, stderr, code) = run(
        dir.path(),
        &["--provider", "brave", "--base-url", &stub.url, "q"],
        &[("BRAVE_API_KEY", "test-key")],
        None,
    );
    assert_eq!(code, 1, "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.contains("401"), "{stderr}");
    assert!(stderr.contains("bad token"), "{stderr}");
    let _ = stub.only();
}

#[test]
fn a_tavily_search_posts_the_query_and_not_the_key() {
    let stub = Stub::start(200, "application/json", &json!({"results": [
        {"title": "T", "url": "https://t.example", "content": "body", "score": 0.4}
    ]}).to_string());
    let dir = Tmp::new();
    let (stdout, stderr, code) = run(
        dir.path(),
        &["--provider", "tavily", "--base-url", &stub.url, "--format", "text", "widgets"],
        &[("TAVILY_API_KEY", "tvly-test")],
        None,
    );
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout.trim(), "T\thttps://t.example\tbody");
    let req = stub.only();
    assert_eq!(req.method, "POST");
    let body: Value = serde_json::from_str(&req.body).expect(&req.body);
    assert_eq!(body["query"], "widgets");
    assert_eq!(body["max_results"], 5);
    assert_eq!(body["search_depth"], "basic");
    assert!(body.get("api_key").is_none(), "the key stays in the header: {body}");
    assert_eq!(req.headers.get("authorization").map(String::as_str), Some("Bearer tvly-test"));
}

#[test]
fn zero_results_is_a_completed_search() {
    let stub = Stub::start(200, "application/json", r#"{"web":{"results":[]}}"#);
    let dir = Tmp::new();
    let (stdout, stderr, code) = run(
        dir.path(),
        &["--provider", "brave", "--base-url", &stub.url, "--quiet", "nothing"],
        &[("BRAVE_API_KEY", "k")],
        None,
    );
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.is_empty());
    assert!(!stderr.contains("results from"), "{stderr}");
}

#[test]
fn a_missing_key_is_exit_1_and_usage_is_exit_2() {
    let dir = Tmp::new();
    let (stdout, stderr, code) = run(dir.path(), &["--provider", "brave", "q"], &[], None);
    assert_eq!(code, 1, "{stderr}");
    assert!(stdout.is_empty());
    assert!(stderr.contains("BRAVE_API_KEY"), "{stderr}");

    let (stdout, stderr, code) = run(dir.path(), &["--provider", "nope", "q"], &[], None);
    assert_eq!(code, 2, "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");

    let (stdout, stderr, code) = run(dir.path(), &[], &[], None);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("empty query"), "{stderr}");
    assert!(stdout.is_empty());

    let (stdout, stderr, code) = run(dir.path(), &["q"], &[("TAVILY_API_KEY", "tavily-secret-value")], None);
    assert_eq!(code, 1, "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.contains("BRAVE_API_KEY"), "{stderr}");
    assert!(!stderr.contains("tavily-secret-value"), "the unused key stays off stderr: {stderr}");
}

#[test]
fn stdin_query_hits_the_stub_and_a_closed_port_is_exit_1() {
    let stub = Stub::start(200, "application/json", &json!({"results": [
        {"title": "S", "url": "https://s.example", "content": "via stdin"}
    ]}).to_string());
    let dir = Tmp::new();
    let (stdout, stderr, code) = run(
        dir.path(),
        &["--provider", "tavily", "--base-url", &stub.url],
        &[("TAVILY_API_KEY", "k")],
        Some("piped query\n"),
    );
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(lines(&stdout)[0]["snippet"], "via stdin");
    assert_eq!(serde_json::from_str::<Value>(&stub.only().body).unwrap()["query"], "piped query");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let url = format!("http://127.0.0.1:{port}/");
    let (stdout, stderr, code) = run(dir.path(), &["--fetch", &url], &[], None);
    assert_eq!(code, 1, "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.contains("failed"), "{stderr}");
}

#[test]
fn the_config_file_names_the_provider_the_cap_and_the_env_var() {
    let stub = Stub::start(200, "application/json", &json!({"web": {"results": [
        {"title": "C", "url": "https://c.example", "description": "cfg"}
    ]}}).to_string());
    let dir = Tmp::new();
    std::fs::create_dir(dir.path().join(".clank")).unwrap();
    std::fs::write(
        dir.path().join(".clank/config.toml"),
        "[web]\ndefault_provider = \"brave\"\nlimit = 4\n\n[web.brave]\napi_key_env = \"CLANK_WEB_TEST_KEY\"\n",
    )
    .unwrap();
    let (stdout, stderr, code) = run(
        dir.path(),
        &["--base-url", &stub.url, "configured"],
        &[("CLANK_WEB_TEST_KEY", "from-env"), ("BRAVE_API_KEY", "not-this")],
        None,
    );
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(lines(&stdout)[0]["title"], "C");
    assert!(stderr.contains("from brave"), "{stderr}");
    let req = stub.only();
    assert!(req.target.contains("count=4"), "{}", req.target);
    assert_eq!(req.headers.get("x-subscription-token").map(String::as_str), Some("from-env"));
}

#[test]
fn a_broken_config_fails_clank_web_and_clank_still_lists_its_tools() {
    let dir = Tmp::new();
    std::fs::create_dir(dir.path().join(".clank")).unwrap();
    std::fs::write(dir.path().join(".clank/config.toml"), "provider = \"nope\"\n").unwrap();

    let (stdout, stderr, code) = run(dir.path(), &["q"], &[("BRAVE_API_KEY", "k")], None);
    assert_eq!(code, 2, "{stderr}");
    assert!(stdout.is_empty());
    assert!(stderr.contains("config.toml"), "{stderr}");

    let out = Command::new(CLANK)
        .current_dir(dir.path())
        .arg("--list-tools")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let tools: Value = serde_json::from_slice(&out.stdout).unwrap();
    let names: Vec<&str> = tools.as_array().unwrap().iter().map(|t| t["function"]["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["read_file", "list_dir", "search", "stat"]);
}

#[test]
fn fetch_strips_html_and_a_short_cap_marks_the_line_truncated() {
    let html = "<html><head><title>Hello &amp; co</title><style>h1{}</style></head>\
                <body><script>secret()</script><p>Visible text.</p></body></html>";
    let stub = Stub::start(200, "text/html", html);
    let dir = Tmp::new();
    let (stdout, stderr, code) = run(dir.path(), &["--fetch", &stub.url], &[], None);
    assert_eq!(code, 0, "{stderr}");
    let hit = &lines(&stdout)[0];
    assert_eq!(hit["title"], "Hello & co");
    assert_eq!(hit["url"], stub.url);
    assert!(hit["snippet"].as_str().unwrap().contains("Visible text"), "{hit}");
    assert!(!hit["snippet"].as_str().unwrap().contains("secret"), "{hit}");
    assert!(hit.get("truncated").is_none());

    let mut big = String::from("<html><head><title>Big</title></head><body>");
    big.push_str(&"x".repeat(512 * 1024));
    big.push_str("</body></html>");
    let stub = Stub::start(200, "text/html", &big);
    let (stdout, stderr, code) = run(dir.path(), &["--fetch", &stub.url, "--quiet"], &[], None);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(lines(&stdout)[0]["truncated"], true);
    assert!(stderr.contains("truncated"), "{stderr}");
}

#[test]
fn firecrawl_posts_one_search_and_reads_either_shape() {
    let stub = Stub::start(200, "application/json", &json!({"success": true, "data": [
        {"title": "F", "url": "https://f.example", "description": "v1 snippet"}
    ]}).to_string());
    let dir = Tmp::new();
    let (stdout, stderr, code) = run(
        dir.path(),
        &["--provider", "firecrawl", "--base-url", &stub.url, "--limit", "3", "widgets"],
        &[("FIRECRAWL_API_KEY", "fc-test")],
        None,
    );
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(lines(&stdout)[0]["snippet"], "v1 snippet");
    let req = stub.only();
    assert_eq!(req.method, "POST");
    let body: Value = serde_json::from_str(&req.body).unwrap();
    assert_eq!(body["query"], "widgets");
    assert_eq!(body["limit"], 3);
    assert!(body.get("scrapeOptions").is_none(), "{body}");
    assert!(body.get("api_key").is_none(), "{body}");
    assert_eq!(req.headers.get("authorization").map(String::as_str), Some("Bearer fc-test"));
    assert!(!stderr.contains("fc-test"), "{stderr}");

    let stub = Stub::start(200, "application/json", &json!({"success": true, "data": {"web": [
        {"title": "F2", "url": "https://f2.example", "description": "v2 snippet"}
    ]}}).to_string());
    let (stdout, stderr, code) = run(
        dir.path(),
        &["--provider", "firecrawl", "--base-url", &stub.url, "local"],
        &[],
        None,
    );
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(lines(&stdout)[0]["snippet"], "v2 snippet");
    let req = stub.only();
    assert!(req.headers.get("authorization").is_none(), "a local URL sends no key: {:?}", req.headers);
}

#[test]
fn searxng_queries_json_and_a_missing_url_is_usage() {
    let dir = Tmp::new();
    let (stdout, stderr, code) = run(dir.path(), &["--provider", "searxng", "q"], &[("SEARXNG_API_KEY", "sx-secret")], None);
    assert_eq!(code, 2, "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.contains("--base-url"), "{stderr}");
    assert!(stderr.contains("[web.searxng].base_url"), "{stderr}");
    assert!(!stderr.contains("sx-secret"), "{stderr}");

    let stub = Stub::start(200, "application/json", &json!({"results": [
        {"title": "S", "url": "https://s.example", "content": "from the instance"}
    ]}).to_string());
    let (stdout, stderr, code) = run(
        dir.path(),
        &["--provider", "searxng", "--base-url", &stub.url, "piped widgets"],
        &[],
        None,
    );
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(lines(&stdout)[0]["snippet"], "from the instance");
    let req = stub.only();
    assert_eq!(req.method, "GET");
    assert!(req.target.contains("q=piped%20widgets"), "{}", req.target);
    assert!(req.target.contains("format=json"), "{}", req.target);
    assert!(req.headers.get("authorization").is_none(), "{:?}", req.headers);
    assert!(!req.target.contains("key"), "{}", req.target);

    let stub = Stub::start(401, "application/json", r#"{"error":"bad sx-secret"}"#);
    let (stdout, stderr, code) = run(
        dir.path(),
        &["--provider", "searxng", "--base-url", &stub.url, "q"],
        &[("SEARXNG_API_KEY", "sx-secret")],
        None,
    );
    assert_eq!(code, 1, "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.contains("***"), "{stderr}");
    assert!(!stderr.contains("sx-secret"), "{stderr}");
    let req = stub.only();
    assert_eq!(req.headers.get("authorization").map(String::as_str), Some("Bearer sx-secret"));
    assert!(!req.target.contains("sx-secret"), "{}", req.target);
}

#[test]
fn the_config_file_points_searxng_at_its_instance() {
    let stub = Stub::start(200, "application/json", &json!({"results": [
        {"title": "C", "url": "https://c.example", "content": "configured"}
    ]}).to_string());
    let dir = Tmp::new();
    std::fs::create_dir(dir.path().join(".clank")).unwrap();
    std::fs::write(
        dir.path().join(".clank/config.toml"),
        format!("[web.searxng]\nbase_url = \"{}\"\n", stub.url),
    )
    .unwrap();
    let (stdout, stderr, code) = run(dir.path(), &["--provider", "searxng", "from file"], &[], None);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(lines(&stdout)[0]["title"], "C");
    let req = stub.only();
    assert!(req.target.contains("q=from%20file"), "{}", req.target);
    assert!(req.target.contains("format=json"), "{}", req.target);
}

#[test]
fn exa_sends_the_key_header_and_reads_the_first_highlight() {
    let stub = Stub::start(200, "application/json", &json!({"results": [
        {"title": "E", "url": "https://e.example", "highlights": ["first hit", "second"], "text": "full page"}
    ]}).to_string());
    let dir = Tmp::new();
    let (stdout, stderr, code) = run(
        dir.path(),
        &["--provider", "exa", "--base-url", &stub.url, "--limit", "4", "highlights"],
        &[("EXA_API_KEY", "exa-test")],
        None,
    );
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(lines(&stdout)[0]["snippet"], "first hit");
    let req = stub.only();
    assert_eq!(req.method, "POST");
    let body: Value = serde_json::from_str(&req.body).unwrap();
    assert_eq!(body["query"], "highlights");
    assert_eq!(body["numResults"], 4);
    assert_eq!(body["contents"]["highlights"], true);
    assert!(body.get("stream").is_none(), "{body}");
    assert!(body.get("api_key").is_none() && body.get("x-api-key").is_none(), "{body}");
    assert_eq!(req.headers.get("x-api-key").map(String::as_str), Some("exa-test"));
    assert!(req.headers.get("authorization").is_none(), "{:?}", req.headers);
    assert!(!stderr.contains("exa-test"), "{stderr}");

    let (stdout, stderr, code) = run(dir.path(), &["--provider", "exa", "cloud"], &[], None);
    assert_eq!(code, 1, "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.contains("EXA_API_KEY"), "{stderr}");
}

#[test]
fn perplexity_posts_the_search_body_and_reads_the_snippet() {
    let stub = Stub::start(200, "application/json", &json!({"results": [
        {"title": "P", "url": "https://p.example", "snippet": "cited line"}
    ]}).to_string());
    let dir = Tmp::new();
    let (stdout, stderr, code) = run(
        dir.path(),
        &["--provider", "perplexity", "--base-url", &stub.url, "citations"],
        &[("PERPLEXITY_API_KEY", "pplx-test")],
        None,
    );
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(lines(&stdout)[0]["snippet"], "cited line");
    let req = stub.only();
    assert_eq!(req.method, "POST");
    let body: Value = serde_json::from_str(&req.body).unwrap();
    assert_eq!(body["query"], "citations");
    assert_eq!(body["max_results"], 5);
    assert!(body.get("search_context_size").is_none(), "{body}");
    assert!(body.get("messages").is_none(), "{body}");
    assert_eq!(req.headers.get("authorization").map(String::as_str), Some("Bearer pplx-test"));
    assert!(!stderr.contains("pplx-test"), "{stderr}");

    let (stdout, stderr, code) = run(
        dir.path(),
        &["--provider", "perplexity", "--base-url", &stub.url, "local"],
        &[],
        None,
    );
    assert_eq!(code, 0, "{stderr}");
    assert!(stub.only().headers.get("authorization").is_none());
    assert_eq!(lines(&stdout)[0]["title"], "P");
}

#[test]
fn clank_tools_stay_on_the_local_filesystem() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let tools = std::fs::read_to_string(root.join("src/tools.rs")).unwrap();
    assert!(!tools.contains("clank-web"), "a web search is not a clank tool");
    assert!(!tools.contains("brave"));
    assert!(!tools.contains("tavily"));
    assert!(!tools.contains("firecrawl"));
    assert!(!tools.contains("searxng"));
    assert!(!tools.contains("perplexity"));
    assert!(!tools.contains("config.toml"));
    for rel in ["src/client.rs", "src/context.rs"] {
        let text = std::fs::read_to_string(root.join(rel)).unwrap();
        assert!(!text.contains("clank-web"), "{rel}");
        assert!(!text.contains("brave"), "{rel}");
    }
}
