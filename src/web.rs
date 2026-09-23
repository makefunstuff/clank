//! clank-web — one web search, or one fetched URL, for a pipeline.
//!
//! The sibling of `clank`, not a tool inside it. `clank` observes the local
//! filesystem; this speaks to a search API and writes result lines:
//!
//! ```sh
//! clank-web "query" | clank -m "summarize with citations"
//! ```
//!
//! stdout is JSONL (`title`, `url`, `snippet`), or `--text` lines of
//! `title<TAB>url<TAB>snippet`. stderr is diagnostics. Exit `0` when the
//! results were written, including an empty set; `1` when the search or fetch
//! did not complete; `2` for usage.
//!
//! `.clank/config.toml` in the working directory is read here and nowhere else
//! in the crate. It names the provider, the result cap, and the environment
//! variable that holds the key. The key itself is never a file or a flag.
//!
//! One request per invocation. `--fetch` is one GET of one URL: no JavaScript,
//! no crawl, no second request that follows a link the page named.

use clap::Parser;
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

const BRAVE_ENDPOINT: &str = "https://api.search.brave.com/res/v1/web/search";
const TAVILY_ENDPOINT: &str = "https://api.tavily.com/search";
const DEFAULT_BRAVE_KEY_ENV: &str = "BRAVE_API_KEY";
const DEFAULT_TAVILY_KEY_ENV: &str = "TAVILY_API_KEY";
const DEFAULT_CONFIG_PATH: &str = ".clank/config.toml";
const DEFAULT_MAX_RESULTS: u32 = 5;
/// Brave's `count` and Tavily's `max_results` both stop at 20.
const MAX_RESULTS_LIMIT: u32 = 20;
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const DEFAULT_FETCH_BYTES: usize = 512 * 1024;
const FETCH_BYTES_LIMIT: usize = 8 * 1024 * 1024;
/// A search response larger than this is not a result set.
const SEARCH_BODY_CAP: usize = 2 * 1024 * 1024;

const OK: i32 = 0;
const FAIL: i32 = 1;
const USAGE: i32 = 2;

#[derive(Parser, Debug)]
#[command(
    name = "clank-web",
    version,
    about = "One web search or one fetched URL, as a pipeline stage",
    after_help = "Examples:\n  \
      clank-web \"rust sigpipe\" | clank -m \"summarize with citations\"\n  \
      printf '%s\\n' \"$query\" | clank-web\n  \
      clank-web --fetch https://example.com | clank -m \"what is this page?\"\n\n\
      Config: .clank/config.toml in the working directory (see .clank/config.example.toml).\n\
      clank does not read that file. Keys come from the environment."
)]
struct Args {
    /// search query; otherwise the query is read from stdin
    #[arg(value_name = "QUERY", conflicts_with = "fetch")]
    query: Option<String>,

    /// provider: brave or tavily; otherwise .clank/config.toml, otherwise the one key that is set
    #[arg(long, value_name = "NAME", conflicts_with = "fetch")]
    provider: Option<String>,

    /// results to request, 1..=20; overrides max_results in the config
    #[arg(long, value_name = "N", conflicts_with = "fetch")]
    max_results: Option<u32>,

    /// config file; default is .clank/config.toml in the working directory, if it exists
    #[arg(long, value_name = "PATH")]
    config: Option<String>,

    /// replaces the provider endpoint, for a proxy or a stub
    #[arg(long, value_name = "URL", conflicts_with = "fetch")]
    base_url: Option<String>,

    /// GET this URL and print one result; no search, no key
    #[arg(long, value_name = "URL")]
    fetch: Option<String>,

    /// cap on the fetched body, bytes (default 524288, max 8388608)
    #[arg(long, value_name = "N")]
    max_bytes: Option<usize>,

    /// per-request timeout, seconds
    #[arg(long, value_name = "SECS", default_value_t = DEFAULT_TIMEOUT_SECS)]
    timeout: u64,

    /// print `title<TAB>url<TAB>snippet` instead of JSONL
    #[arg(long)]
    text: bool,

    /// suppress the result-count line on stderr
    #[arg(short, long)]
    quiet: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Provider {
    Brave,
    Tavily,
}

impl Provider {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "brave" => Some(Provider::Brave),
            "tavily" => Some(Provider::Tavily),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Provider::Brave => "brave",
            Provider::Tavily => "tavily",
        }
    }

    fn default_endpoint(self) -> &'static str {
        match self {
            Provider::Brave => BRAVE_ENDPOINT,
            Provider::Tavily => TAVILY_ENDPOINT,
        }
    }

    fn default_key_env(self) -> &'static str {
        match self {
            Provider::Brave => DEFAULT_BRAVE_KEY_ENV,
            Provider::Tavily => DEFAULT_TAVILY_KEY_ENV,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Hit {
    title: String,
    url: String,
    snippet: String,
    /// Set on a fetched page whose body was cut at `--max-bytes`.
    truncated: bool,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct FileConfig {
    provider: Option<String>,
    max_results: Option<u32>,
    brave: Option<KeyEnv>,
    tavily: Option<KeyEnv>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct KeyEnv {
    api_key_env: Option<String>,
}

struct Loaded {
    config: FileConfig,
    /// Set when a file was actually read, so a missing key can name it.
    path: Option<String>,
}

#[derive(Debug)]
struct Resolved {
    provider: Provider,
    endpoint: String,
    api_key: String,
    max_results: u32,
}

fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    name.len() <= 128 && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn http_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")) else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty()
}

fn parse_config(text: &str) -> Result<FileConfig, String> {
    let cfg: FileConfig = toml::from_str(text).map_err(|e| {
        let msg = e.to_string();
        if msg.contains("unknown field `api_key`") {
            format!("{msg}; a key is not stored in this file — api_key_env names the environment variable")
        } else {
            msg
        }
    })?;
    if let Some(name) = cfg.provider.as_deref() {
        if Provider::parse(name).is_none() {
            return Err(format!("unknown provider {name:?} (brave, tavily)"));
        }
    }
    if let Some(n) = cfg.max_results {
        if !(1..=MAX_RESULTS_LIMIT).contains(&n) {
            return Err(format!("max_results {n} is outside 1..={MAX_RESULTS_LIMIT}"));
        }
    }
    for (label, section) in [("brave", &cfg.brave), ("tavily", &cfg.tavily)] {
        if let Some(env) = section.as_ref().and_then(|s| s.api_key_env.as_deref()) {
            if !valid_env_name(env) {
                return Err(format!("{label}.api_key_env {env:?} is not an environment variable name"));
            }
        }
    }
    Ok(cfg)
}

fn load_config(path: Option<&str>) -> Result<Loaded, (String, i32)> {
    let (text, origin) = match path {
        Some(p) => {
            let text = std::fs::read_to_string(p).map_err(|e| (format!("reading --config {p}: {e}"), USAGE))?;
            (text, Some(p.to_string()))
        }
        None => {
            if !Path::new(DEFAULT_CONFIG_PATH).is_file() {
                return Ok(Loaded { config: FileConfig::default(), path: None });
            }
            let text = std::fs::read_to_string(DEFAULT_CONFIG_PATH)
                .map_err(|e| (format!("reading {DEFAULT_CONFIG_PATH}: {e}"), USAGE))?;
            (text, Some(DEFAULT_CONFIG_PATH.to_string()))
        }
    };
    match parse_config(&text) {
        Ok(config) => Ok(Loaded { config, path: origin }),
        Err(e) => Err((format!("{}: {e}", origin.unwrap_or_default()), USAGE)),
    }
}

/// The env var the provider's key is read from, and whether the file named it.
fn key_env<'a>(config: &'a FileConfig, provider: Provider) -> (&'a str, bool) {
    let custom = match provider {
        Provider::Brave => config.brave.as_ref().and_then(|s| s.api_key_env.as_deref()),
        Provider::Tavily => config.tavily.as_ref().and_then(|s| s.api_key_env.as_deref()),
    };
    match custom {
        Some(name) => (name, true),
        None => (provider.default_key_env(), false),
    }
}

fn getenv(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn resolve(
    provider_flag: Option<&str>,
    base_url: Option<&str>,
    max_flag: Option<u32>,
    loaded: &Loaded,
    env: impl Fn(&str) -> Option<String>,
) -> Result<Resolved, (String, i32)> {
    let max_results = max_flag.or(loaded.config.max_results).unwrap_or(DEFAULT_MAX_RESULTS);
    if !(1..=MAX_RESULTS_LIMIT).contains(&max_results) {
        return Err((format!("--max-results {max_results} is outside 1..={MAX_RESULTS_LIMIT}"), USAGE));
    }
    if let Some(url) = base_url {
        if !http_url(url) {
            return Err((format!("--base-url {url:?} must be an http or https URL"), USAGE));
        }
    }

    let provider = if let Some(name) = provider_flag {
        Provider::parse(name).ok_or_else(|| (format!("unknown --provider {name:?} (brave, tavily)"), USAGE))?
    } else if let Some(name) = loaded.config.provider.as_deref() {
        // parse_config already rejected an unknown name; a mismatch here is a bug in that check.
        Provider::parse(name).ok_or_else(|| (format!("unknown provider {name:?} (brave, tavily)"), USAGE))?
    } else {
        let (brave_env, _) = key_env(&loaded.config, Provider::Brave);
        let (tavily_env, _) = key_env(&loaded.config, Provider::Tavily);
        let brave_set = env(brave_env).is_some();
        let tavily_set = env(tavily_env).is_some();
        match (brave_set, tavily_set) {
            (true, false) => Provider::Brave,
            (false, true) => Provider::Tavily,
            (true, true) if brave_env == tavily_env => {
                return Err((
                    format!("{brave_env} is set and no provider was chosen: pass --provider or set provider in {DEFAULT_CONFIG_PATH}"),
                    USAGE,
                ));
            }
            (true, true) => {
                return Err((
                    format!("both {brave_env} and {tavily_env} are set: pass --provider or set provider in {DEFAULT_CONFIG_PATH}"),
                    USAGE,
                ));
            }
            (false, false) => {
                return Err((
                    format!("no API key: set {brave_env} or {tavily_env}, or set provider in {DEFAULT_CONFIG_PATH}"),
                    FAIL,
                ));
            }
        }
    };

    let (env_name, named_in_file) = key_env(&loaded.config, provider);
    let api_key = match env(env_name) {
        Some(k) => k,
        None if named_in_file => {
            let path = loaded.path.as_deref().unwrap_or(DEFAULT_CONFIG_PATH);
            return Err((format!("no API key: {path} names {env_name}, and it is unset or empty"), FAIL));
        }
        None => return Err((format!("no API key: set {env_name}"), FAIL)),
    };
    let endpoint = base_url.unwrap_or_else(|| provider.default_endpoint()).to_string();
    Ok(Resolved { provider, endpoint, api_key, max_results })
}

fn percent_encode(s: &str) -> String {
    const HEX: &[u8] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => {
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0xf) as usize] as char);
            }
        }
    }
    out
}

fn with_query(endpoint: &str, pairs: &[(&str, &str)]) -> String {
    let mut out = String::from(endpoint);
    let mut sep = if endpoint.contains('?') { '&' } else { '?' };
    for (k, v) in pairs {
        out.push(sep);
        out.push_str(&percent_encode(k));
        out.push('=');
        out.push_str(&percent_encode(v));
        sep = '&';
    }
    out
}

fn agent(timeout: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout)))
        .http_status_as_error(false)
        .build()
        .new_agent()
}

fn read_capped(resp: &mut ureq::http::Response<ureq::Body>, cap: usize) -> Result<(String, bool), String> {
    let mut buf = Vec::new();
    let mut reader = resp.body_mut().as_reader().take((cap as u64).saturating_add(1));
    reader.read_to_end(&mut buf).map_err(|e| format!("reading the response: {e}"))?;
    let truncated = buf.len() > cap;
    if truncated {
        buf.truncate(cap);
    }
    match String::from_utf8(buf) {
        Ok(s) => Ok((s, truncated)),
        Err(_) => Err("response was not UTF-8".into()),
    }
}

fn content_type(resp: &ureq::http::Response<ureq::Body>) -> String {
    resp.headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

fn error_detail(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "empty body".into();
    }
    let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
        return trimmed.chars().take(300).collect();
    };
    fn stringish(v: &Value) -> Option<String> {
        match v {
            Value::String(s) if !s.trim().is_empty() => Some(s.trim().chars().take(300).collect()),
            Value::Object(map) => ["message", "detail", "error", "title"]
                .iter()
                .find_map(|k| map.get(*k).and_then(stringish)),
            _ => None,
        }
    }
    ["error", "message", "detail"].iter().find_map(|k| v.get(*k).and_then(stringish)).unwrap_or_else(|| {
        trimmed.chars().take(300).collect()
    })
}

fn http_failure(status: impl std::fmt::Display, body: &str) -> String {
    format!("HTTP {status}: {}", error_detail(body))
}

struct Raw {
    status_not_ok: bool,
    status_label: String,
    content_type: String,
    body: String,
    truncated: bool,
}

fn take_response(mut resp: ureq::http::Response<ureq::Body>, cap: usize) -> Result<Raw, String> {
    let status_not_ok = resp.status() != 200;
    let status_label = resp.status().to_string();
    let content_type = content_type(&resp);
    let (body, truncated) = read_capped(&mut resp, cap)?;
    Ok(Raw { status_not_ok, status_label, content_type, body, truncated })
}

fn search(timeout: u64, resolved: &Resolved, query: &str) -> Result<(Vec<Hit>, u32), String> {
    let agent = agent(timeout);
    let resp = match resolved.provider {
        Provider::Brave => {
            let url = with_query(&resolved.endpoint, &[("q", query), ("count", &resolved.max_results.to_string())]);
            agent
                .get(&url)
                .header("Accept", "application/json")
                .header("X-Subscription-Token", &resolved.api_key)
                .call()
                .map_err(|e| format!("request to {} failed: {e}", resolved.endpoint))?
        }
        Provider::Tavily => {
            // An unspecified depth can be promoted to advanced (two credits).
            // One invocation is one basic search.
            let body = json!({
                "query": query,
                "max_results": resolved.max_results,
                "search_depth": "basic",
            });
            agent
                .post(&resolved.endpoint)
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", resolved.api_key))
                .send_json(&body)
                .map_err(|e| format!("request to {} failed: {e}", resolved.endpoint))?
        }
    };
    let raw = take_response(resp, SEARCH_BODY_CAP)?;
    if raw.truncated {
        return Err(format!("response exceeded {SEARCH_BODY_CAP} bytes"));
    }
    if raw.status_not_ok {
        return Err(http_failure(raw.status_label, &raw.body));
    }
    let payload: Value = serde_json::from_str(&raw.body).map_err(|e| format!("provider returned non-JSON: {e}"))?;
    if payload.get("error").is_some() {
        return Err(format!("provider returned an error: {}", error_detail(&raw.body)));
    }
    parse_hits(resolved.provider, &payload)
}

fn parse_hits(provider: Provider, payload: &Value) -> Result<(Vec<Hit>, u32), String> {
    let arr = match provider {
        Provider::Brave => match payload.get("web") {
            None | Some(Value::Null) => return Ok((Vec::new(), 0)),
            Some(web) => web.get("results").and_then(Value::as_array).ok_or("brave response has web but no results array")?,
        },
        Provider::Tavily => match payload.get("results") {
            None | Some(Value::Null) => return Ok((Vec::new(), 0)),
            Some(results) => results.as_array().ok_or("tavily response has results but it is not an array")?,
        },
    };
    let snippet_key = match provider {
        Provider::Brave => "description",
        Provider::Tavily => "content",
    };
    let mut hits = Vec::new();
    let mut skipped = 0u32;
    for item in arr {
        let url = item.get("url").and_then(Value::as_str).unwrap_or("").trim();
        if url.is_empty() {
            skipped += 1;
            continue;
        }
        hits.push(Hit {
            title: item.get("title").and_then(Value::as_str).unwrap_or("").trim().to_string(),
            url: url.to_string(),
            snippet: item.get(snippet_key).and_then(Value::as_str).unwrap_or("").trim().to_string(),
            truncated: false,
        });
    }
    if hits.is_empty() && skipped > 0 {
        return Err(format!("provider returned {skipped} results and none had a url"));
    }
    Ok((hits, skipped))
}

fn fetch_url(timeout: u64, url: &str, cap: usize) -> Result<Hit, String> {
    let agent = agent(timeout);
    let resp = agent.get(url).call().map_err(|e| format!("request to {url} failed: {e}"))?;
    let raw = take_response(resp, cap)?;
    if raw.status_not_ok {
        return Err(http_failure(raw.status_label, &raw.body));
    }
    let (title, snippet) = page_text(&raw.content_type, &raw.body);
    Ok(Hit { title, url: url.to_string(), snippet, truncated: raw.truncated })
}

fn page_text(content_type: &str, body: &str) -> (String, String) {
    if is_html(content_type, body) {
        html_to_text(body)
    } else {
        (String::new(), body.trim().to_string())
    }
}

fn is_html(content_type: &str, body: &str) -> bool {
    let ct = content_type.to_ascii_lowercase();
    if ct.contains("html") {
        return true;
    }
    if ct.contains("json") || ct.contains("text/plain") {
        return false;
    }
    let head: String = body.chars().take(512).collect::<String>().to_ascii_lowercase();
    head.contains("<html") || head.contains("<!doctype") || head.contains("<body") || head.contains("<head")
}

fn starts_tag(bytes: &[u8], name: &str) -> bool {
    let mut prefix = Vec::with_capacity(name.len() + 1);
    prefix.push(b'<');
    prefix.extend(name.as_bytes());
    if bytes.len() < prefix.len() || !bytes[..prefix.len()].eq_ignore_ascii_case(&prefix) {
        return false;
    }
    match bytes.get(prefix.len()) {
        Some(b) if b.is_ascii_alphanumeric() => false,
        _ => true,
    }
}

fn find_end_tag(bytes: &[u8], name: &str) -> Option<usize> {
    let mut needle = Vec::with_capacity(name.len() + 2);
    needle.extend(b"</");
    needle.extend(name.as_bytes());
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        if bytes[i..i + needle.len()].eq_ignore_ascii_case(&needle) {
            if let Some(rel) = bytes[i + needle.len()..].iter().position(|c| *c == b'>') {
                return Some(i + needle.len() + rel + 1);
            }
        }
        i += 1;
    }
    None
}

fn is_block_tag(bytes: &[u8]) -> bool {
    if bytes.first() != Some(&b'<') {
        return false;
    }
    let mut i = 1;
    if bytes.get(i) == Some(&b'/') {
        i += 1;
    }
    let start = i;
    while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
        i += 1;
    }
    if start == i {
        return false;
    }
    const NAMES: &[&[u8]] = &[
        b"p",
        b"div",
        b"br",
        b"li",
        b"tr",
        b"h1",
        b"h2",
        b"h3",
        b"h4",
        b"h5",
        b"h6",
        b"section",
        b"article",
        b"header",
        b"footer",
        b"blockquote",
        b"pre",
        b"hr",
        b"table",
        b"ul",
        b"ol",
    ];
    let name = &bytes[start..i];
    NAMES.iter().any(|n| n.eq_ignore_ascii_case(name))
}

fn html_to_text(raw: &str) -> (String, String) {
    let bytes = raw.as_bytes();
    let mut title = String::new();
    let mut text = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let rest = &bytes[i..];
            if starts_tag(rest, "script") {
                i += find_end_tag(rest, "script").unwrap_or(rest.len());
                continue;
            }
            if starts_tag(rest, "style") {
                i += find_end_tag(rest, "style").unwrap_or(rest.len());
                continue;
            }
            if starts_tag(rest, "noscript") {
                i += find_end_tag(rest, "noscript").unwrap_or(rest.len());
                continue;
            }
            if starts_tag(rest, "title") && title.is_empty() {
                if let Some(gt) = rest.iter().position(|c| *c == b'>') {
                    let after = i + gt + 1;
                    if let Some(rel) = find_end_tag(&bytes[after..], "title") {
                        let end = after + rel;
                        let inner = &raw[after..end];
                        // `end` sits after `</title>`, so the last `<` opens that close tag.
                        let cut = inner.rfind('<').unwrap_or(inner.len());
                        title = inner[..cut].to_string();
                        i = end;
                        continue;
                    }
                }
            }
            if is_block_tag(rest) {
                text.push('\n');
            }
            match rest.iter().position(|c| *c == b'>') {
                Some(gt) => i += gt + 1,
                None => break,
            }
            continue;
        }
        let Some(ch) = raw[i..].chars().next() else { break };
        text.push(ch);
        i += ch.len_utf8();
    }
    (single_line(&decode_entities(&title)), single_line(&decode_entities(&text)))
}

fn decode_entities(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some(rel) = bytes[i + 1..].iter().position(|c| *c == b';') {
                let semi = i + 1 + rel;
                if let Some(ch) = named_entity(&s[i + 1..semi]) {
                    out.push(ch);
                    i = semi + 1;
                    continue;
                }
            }
        }
        let Some(ch) = s[i..].chars().next() else { break };
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn named_entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => numeric_entity(name),
    }
}

fn numeric_entity(name: &str) -> Option<char> {
    let digits = name.strip_prefix('#')?;
    let code = if let Some(hex) = digits.strip_prefix('x').or_else(|| digits.strip_prefix('X')) {
        u32::from_str_radix(hex, 16).ok()?
    } else {
        digits.parse().ok()?
    };
    char::from_u32(code).filter(|c| *c != '\0' && !c.is_control())
}

fn single_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn write_hits(text_mode: bool, hits: &[Hit]) -> Result<(), String> {
    let mut out = std::io::stdout().lock();
    for hit in hits {
        let line = if text_mode {
            format!("{}\t{}\t{}\n", single_line(&hit.title), single_line(&hit.url), single_line(&hit.snippet))
        } else {
            let mut obj = serde_json::Map::new();
            obj.insert("title".into(), json!(hit.title));
            obj.insert("url".into(), json!(hit.url));
            obj.insert("snippet".into(), json!(hit.snippet));
            if hit.truncated {
                obj.insert("truncated".into(), json!(true));
            }
            let mut s = serde_json::to_string(&Value::Object(obj))
                .map_err(|e| format!("a result did not serialise: {e}"))?;
            s.push('\n');
            s
        };
        out.write_all(line.as_bytes()).map_err(|e| format!("writing stdout: {e}"))?;
    }
    Ok(())
}

fn query_from(args: &Args) -> Result<String, String> {
    if let Some(q) = &args.query {
        let q = q.trim();
        if q.is_empty() {
            return Err("empty query".into());
        }
        return Ok(q.to_string());
    }
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).map_err(|e| format!("reading stdin: {e}"))?;
    let q = buf.trim();
    if q.is_empty() {
        return Err("empty query — pass it as an argument or on stdin".into());
    }
    Ok(q.to_string())
}

fn run(args: Args) -> i32 {
    if args.timeout == 0 {
        eprintln!("clank-web: --timeout must be at least 1");
        return USAGE;
    }
    if let Some(n) = args.max_bytes {
        if args.fetch.is_none() {
            eprintln!("clank-web: --max-bytes applies to --fetch");
            return USAGE;
        }
        if n == 0 || n > FETCH_BYTES_LIMIT {
            eprintln!("clank-web: --max-bytes {n} is outside 1..={FETCH_BYTES_LIMIT}");
            return USAGE;
        }
    }
    let loaded = match load_config(args.config.as_deref()) {
        Ok(c) => c,
        Err((msg, code)) => {
            eprintln!("clank-web: {msg}");
            return code;
        }
    };

    if let Some(url) = args.fetch.clone() {
        if !http_url(&url) {
            eprintln!("clank-web: --fetch {url:?} must be an http or https URL");
            return USAGE;
        }
        let cap = args.max_bytes.unwrap_or(DEFAULT_FETCH_BYTES);
        let started = Instant::now();
        let hit = match fetch_url(args.timeout, &url, cap) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("clank-web: {e}");
                return FAIL;
            }
        };
        let truncated = hit.truncated;
        if let Err(e) = write_hits(args.text, std::slice::from_ref(&hit)) {
            eprintln!("clank-web: {e}");
            return FAIL;
        }
        if truncated {
            eprintln!("clank-web: response truncated at {cap} bytes");
        }
        if !args.quiet {
            eprintln!("clank-web: fetched 1 page in {} ms", started.elapsed().as_millis());
        }
        return OK;
    }

    let query = match query_from(&args) {
        Ok(q) => q,
        Err(e) => {
            eprintln!("clank-web: {e}");
            return USAGE;
        }
    };
    let resolved = match resolve(args.provider.as_deref(), args.base_url.as_deref(), args.max_results, &loaded, getenv) {
        Ok(r) => r,
        Err((msg, code)) => {
            eprintln!("clank-web: {msg}");
            return code;
        }
    };
    let started = Instant::now();
    let (hits, skipped) = match search(args.timeout, &resolved, &query) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("clank-web: {e}");
            return FAIL;
        }
    };
    if let Err(e) = write_hits(args.text, &hits) {
        eprintln!("clank-web: {e}");
        return FAIL;
    }
    if skipped > 0 {
        eprintln!("clank-web: skipped {skipped} results with no url");
    }
    if !args.quiet {
        eprintln!(
            "clank-web: {} results from {} in {} ms",
            hits.len(),
            resolved.provider.name(),
            started.elapsed().as_millis()
        );
    }
    OK
}

fn main() {
    #[cfg(unix)]
    unsafe {
        // Rust installs SIG_IGN for SIGPIPE; restore the default disposition so
        // `clank-web ... | head` terminates like a normal unix tool, not a panic.
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let args = Args::parse();
    std::process::exit(run(args));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(argv: &[&str]) -> Args {
        Args::try_parse_from(std::iter::once("clank-web").chain(argv.iter().copied())).unwrap()
    }

    fn env_of<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| (*v).to_string()).filter(|v| !v.is_empty())
    }

    fn loaded(text: &str) -> Loaded {
        Loaded { config: parse_config(text).unwrap(), path: Some(DEFAULT_CONFIG_PATH.into()) }
    }

    #[test]
    fn the_example_config_is_the_one_the_doc_shows() {
        let file = include_str!("../.clank/config.example.toml");
        let doc = include_str!("../docs/clank-web.md");
        let fence = doc.split("```toml\n").nth(1).expect("toml fence in docs/clank-web.md");
        let fence = fence.split("```").next().expect("fence end");
        assert_eq!(fence.trim_end(), file.trim_end(), "docs/clank-web.md and .clank/config.example.toml drifted");
        let cfg = parse_config(file).unwrap();
        assert_eq!(cfg.provider.as_deref(), Some("brave"));
        assert_eq!(cfg.max_results, Some(5));
        assert_eq!(cfg.brave.unwrap().api_key_env.as_deref(), Some("BRAVE_API_KEY"));
        assert_eq!(cfg.tavily.unwrap().api_key_env.as_deref(), Some("TAVILY_API_KEY"));
    }

    #[test]
    fn a_key_in_the_file_is_rejected() {
        let err = parse_config("provider = \"brave\"\napi_key = \"secret\"\n").unwrap_err();
        assert!(err.contains("api_key_env"), "{err}");
        assert!(parse_config("provider = \"google\"\n").is_err());
        assert!(parse_config("max_results = 0\n").is_err());
        assert!(parse_config("max_results = 21\n").is_err());
        assert!(parse_config("[brave]\napi_key_env = \"not a name\"\n").is_err());
        assert!(parse_config("").is_ok());
    }

    #[test]
    fn the_flag_beats_the_file_and_one_key_selects_its_provider() {
        let file = loaded("provider = \"tavily\"\nmax_results = 3\n");
        let got = resolve(Some("brave"), None, Some(2), &file, env_of(&[("BRAVE_API_KEY", "k")])).unwrap();
        assert_eq!(got.provider, Provider::Brave);
        assert_eq!(got.max_results, 2);
        assert_eq!(got.api_key, "k");
        assert_eq!(got.endpoint, BRAVE_ENDPOINT);

        let empty = Loaded { config: FileConfig::default(), path: None };
        let got = resolve(None, None, None, &empty, env_of(&[("TAVILY_API_KEY", "t")])).unwrap();
        assert_eq!(got.provider, Provider::Tavily);
        assert_eq!(got.max_results, DEFAULT_MAX_RESULTS);

        let err = resolve(None, None, None, &empty, env_of(&[("BRAVE_API_KEY", "a"), ("TAVILY_API_KEY", "b")])).unwrap_err();
        assert_eq!(err.1, USAGE);

        let err = resolve(None, None, None, &empty, env_of(&[])).unwrap_err();
        assert_eq!(err.1, FAIL, "{:?}", err.0);

        let named = loaded("provider = \"brave\"\n\n[brave]\napi_key_env = \"SEARCH_TOKEN\"\n");
        let err = resolve(None, None, None, &named, env_of(&[])).unwrap_err();
        assert!(err.0.contains("SEARCH_TOKEN"), "{}", err.0);
        assert_eq!(err.1, FAIL);
    }

    #[test]
    fn brave_and_tavily_results_become_hits_and_a_missing_url_is_skipped() {
        let payload = json!({"web": {"results": [
            {"title": "A", "url": "https://a.example", "description": "snippet a"},
            {"title": "no url", "description": "x"},
            {"url": "https://b.example"}
        ]}});
        let (hits, skipped) = parse_hits(Provider::Brave, &payload).unwrap();
        assert_eq!(skipped, 1);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].snippet, "snippet a");
        assert_eq!(hits[1].title, "");
        assert_eq!(hits[1].snippet, "");

        let (hits, skipped) = parse_hits(Provider::Tavily, &json!({"results": [
            {"title": "T", "url": "https://t.example", "content": "body", "score": 0.9}
        ]})).unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(hits[0].snippet, "body");

        assert!(parse_hits(Provider::Brave, &json!({"web": {"results": [{"title": "x"}]}})).is_err());
        let (hits, _) = parse_hits(Provider::Brave, &json!({})).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn html_drops_script_and_style_and_keeps_the_title() {
        let raw = "<html><head><title>Hello &amp; co</title><style>p{color:red}</style></head>\
                   <body><script>secret()</script><p>Visible text.</p></body></html>";
        let (title, text) = html_to_text(raw);
        assert_eq!(title, "Hello & co");
        assert!(text.contains("Visible text"), "{text}");
        assert!(!text.contains("secret"), "{text}");
        assert!(!text.contains("color"), "{text}");

        let (title, text) = html_to_text("<title>café</title><p>naïve body</p>");
        assert_eq!(title, "café");
        assert!(text.contains("naïve body"), "{text}");
    }

    #[test]
    fn the_query_string_is_encoded_and_appended() {
        assert_eq!(percent_encode("a b/c"), "a%20b%2Fc");
        assert_eq!(
            with_query("http://127.0.0.1:9/search", &[("q", "a b"), ("count", "2")]),
            "http://127.0.0.1:9/search?q=a%20b&count=2"
        );
        assert!(
            Args::try_parse_from(["clank-web", "--provider", "brave", "--fetch", "https://example.com"]).is_err(),
            "--fetch takes no provider"
        );
        let a = args(&["--fetch", "https://example.com"]);
        assert_eq!(a.fetch.as_deref(), Some("https://example.com"));
        assert!(Args::try_parse_from(["clank-web", "--max-bytes", "10", "q"]).is_ok());
    }
}
