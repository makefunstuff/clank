//! clank-web — one web search, or one fetched URL, for a pipeline.
//!
//! The sibling of `clank`, not a tool inside it. `clank` observes the local
//! filesystem; this speaks to a search API and writes result lines:
//!
//! ```sh
//! clank-web "query" | clank -m "summarize with citations"
//! ```
//!
//! stdout is JSONL (`title`, `url`, `snippet`), or `--format text` lines of
//! `title<TAB>url<TAB>snippet`. stderr is diagnostics. Exit `0` when the
//! results were written, including an empty set; `1` when the search or fetch
//! did not complete; `2` for usage.
//!
//! Optional `./.clank/config.toml` supplies `[web]`: the default provider, the
//! result cap, the output format, and the environment variable that holds each
//! key. `--config` or `CLANK_CONFIG` names a different file. `[clank]` in that
//! file is the chat endpoint and is ignored here. A missing file in the working
//! directory leaves Brave, five results, and JSONL in charge. The key itself is
//! never an argument, and it is never printed.
//!
//! One request per invocation. `--fetch` is one GET of one URL: no JavaScript,
//! no crawl, no second request that follows a link the page named.

mod config;

use clap::Parser;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const BRAVE_ENDPOINT: &str = "https://api.search.brave.com/res/v1/web/search";
const TAVILY_ENDPOINT: &str = "https://api.tavily.com/search";
const DEFAULT_BRAVE_KEY_ENV: &str = "BRAVE_API_KEY";
const DEFAULT_TAVILY_KEY_ENV: &str = "TAVILY_API_KEY";
const DEFAULT_LIMIT: u32 = 5;
/// Brave's `count` and Tavily's `max_results` both stop at 20.
const LIMIT_MAX: u32 = 20;
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const FETCH_BYTES: usize = 512 * 1024;
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
      Config: ./.clank/config.toml in the working directory.\n\
      --config PATH or CLANK_CONFIG names a different file. Parents are not searched.\n\
      [web] is for this binary. [clank] is for clank and clank-jev, and is ignored here.\n\
      Precedence: flags, then the environment, then the file, then built-ins.\n\
      A missing working-directory file is not an error. Keys come from the environment."
)]
struct Args {
    /// search query; otherwise the query is read from stdin
    #[arg(value_name = "QUERY", conflicts_with = "fetch")]
    query: Option<String>,

    /// provider: brave (default) or tavily
    #[arg(long, value_name = "NAME", conflicts_with = "fetch")]
    provider: Option<String>,

    /// results to request, 1..=20 (default 5)
    #[arg(long, value_name = "N", conflicts_with = "fetch")]
    limit: Option<u32>,

    /// jsonl (default) or text (`title<TAB>url<TAB>snippet`)
    #[arg(long, value_name = "jsonl|text")]
    format: Option<String>,

    /// replaces the provider endpoint, for a proxy or a stub
    #[arg(long, value_name = "URL", conflicts_with = "fetch")]
    base_url: Option<String>,

    /// config file; otherwise CLANK_CONFIG, otherwise ./.clank/config.toml
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// GET this URL and print one result; no search, no key
    #[arg(long, value_name = "URL")]
    fetch: Option<String>,

    /// per-request timeout, seconds
    #[arg(long, value_name = "SECS", default_value_t = DEFAULT_TIMEOUT_SECS)]
    timeout: u64,

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
    /// Set on a fetched page whose body was cut at the fetch cap.
    truncated: bool,
}

#[derive(Debug)]
struct Resolved {
    provider: Provider,
    endpoint: String,
    api_key: String,
    limit: u32,
    text: bool,
}

fn http_url(url: &str) -> bool {
    config::http_url(url)
}

fn section_keys(web: &config::Web, provider: Provider) -> &config::ProviderKeys {
    match provider {
        Provider::Brave => &web.brave,
        Provider::Tavily => &web.tavily,
    }
}

/// Flags, then `[web]`, then built-ins. The search endpoint is `--base-url` or
/// the provider's own URL. `[clank].base_url` is a chat server and is not used.
fn resolve(
    provider_flag: Option<&str>,
    base_url: Option<&str>,
    limit_flag: Option<u32>,
    format_flag: Option<&str>,
    file: Option<&config::File>,
    env: impl Fn(&str) -> Option<String>,
) -> Result<Resolved, (String, i32)> {
    let web = file.map(|f| &f.web);
    if let Some(url) = base_url {
        if !http_url(url) {
            return Err((format!("--base-url {url:?} must be an http or https URL"), USAGE));
        }
    }
    let provider = if let Some(name) = provider_flag {
        Provider::parse(name).ok_or_else(|| (format!("unknown --provider {name:?} (brave, tavily)"), USAGE))?
    } else if let Some(name) = web.and_then(|w| w.default_provider.as_deref()) {
        Provider::parse(name).ok_or_else(|| (format!("unknown provider {name:?} (brave, tavily)"), USAGE))?
    } else {
        Provider::Brave
    };
    let limit = limit_flag.or(web.and_then(|w| w.limit)).unwrap_or(DEFAULT_LIMIT);
    if !(1..=LIMIT_MAX).contains(&limit) {
        return Err((format!("--limit {limit} is outside 1..={LIMIT_MAX}"), USAGE));
    }
    let format = format_flag.or(web.and_then(|w| w.format.as_deref()));
    let text = match format {
        None | Some("jsonl") => false,
        Some("text") => true,
        Some(other) => return Err((format!("--format {other:?} must be jsonl or text"), USAGE)),
    };
    let keys = web.map(|w| section_keys(w, provider));
    let named = keys.and_then(|k| k.api_key_env.as_deref());
    let inline = keys.and_then(|k| k.api_key.as_deref());
    let api_key = match config::key_from(named, inline, Some(provider.default_key_env()), &env) {
        Some(k) => k,
        None => {
            let name = named.unwrap_or(provider.default_key_env());
            let msg = match (named, file) {
                (Some(_), Some(f)) => format!("no API key: {} names {name}, and it is unset or empty", f.path.display()),
                _ => format!("no API key: set {name}"),
            };
            return Err((msg, FAIL));
        }
    };
    let endpoint = base_url.unwrap_or_else(|| provider.default_endpoint()).to_string();
    Ok(Resolved { provider, endpoint, api_key, limit, text })
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

fn hide(resolved: &Resolved, text: &str) -> String {
    config::redact(text, &resolved.api_key)
}

fn search(timeout: u64, resolved: &Resolved, query: &str) -> Result<(Vec<Hit>, u32), String> {
    let agent = agent(timeout);
    let resp = match resolved.provider {
        Provider::Brave => {
            let url = with_query(&resolved.endpoint, &[("q", query), ("count", &resolved.limit.to_string())]);
            agent
                .get(&url)
                .header("Accept", "application/json")
                .header("X-Subscription-Token", &resolved.api_key)
                .call()
                .map_err(|e| hide(resolved, &format!("request to {} failed: {e}", resolved.endpoint)))?
        }
        Provider::Tavily => {
            // An unspecified depth can be promoted to advanced (two credits).
            // One invocation is one basic search.
            let body = json!({
                "query": query,
                "max_results": resolved.limit,
                "search_depth": "basic",
            });
            agent
                .post(&resolved.endpoint)
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", resolved.api_key))
                .send_json(&body)
                .map_err(|e| hide(resolved, &format!("request to {} failed: {e}", resolved.endpoint)))?
        }
    };
    let raw = take_response(resp, SEARCH_BODY_CAP)?;
    if raw.truncated {
        return Err(hide(resolved, &format!("response exceeded {SEARCH_BODY_CAP} bytes")));
    }
    if raw.status_not_ok {
        return Err(hide(resolved, &http_failure(raw.status_label, &raw.body)));
    }
    let payload: Value = serde_json::from_str(&raw.body).map_err(|e| hide(resolved, &format!("provider returned non-JSON: {e}")))?;
    if payload.get("error").is_some() {
        return Err(hide(resolved, &format!("provider returned an error: {}", error_detail(&raw.body))));
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
    let file = match config::load(args.config.as_deref()) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("clank-web: {e}");
            return USAGE;
        }
    };
    // `--fetch` has no provider and no key. Format still comes from the flag or `[web]`.
    let text = match args.format.as_deref().or(file.as_ref().and_then(|f| f.web.format.as_deref())) {
        None | Some("jsonl") => false,
        Some("text") => true,
        Some(other) => {
            eprintln!("clank-web: --format {other:?} must be jsonl or text");
            return USAGE;
        }
    };

    if let Some(url) = args.fetch.clone() {
        if !http_url(&url) {
            eprintln!("clank-web: --fetch {url:?} must be an http or https URL");
            return USAGE;
        }
        let started = Instant::now();
        let hit = match fetch_url(args.timeout, &url, FETCH_BYTES) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("clank-web: {e}");
                return FAIL;
            }
        };
        let truncated = hit.truncated;
        if let Err(e) = write_hits(text, std::slice::from_ref(&hit)) {
            eprintln!("clank-web: {e}");
            return FAIL;
        }
        if truncated {
            eprintln!("clank-web: response truncated at {FETCH_BYTES} bytes");
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
    let resolved = match resolve(
        args.provider.as_deref(),
        args.base_url.as_deref(),
        args.limit,
        args.format.as_deref(),
        file.as_ref(),
        config::env_var,
    ) {
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
    if let Err(e) = write_hits(resolved.text, &hits) {
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

    fn file(text: &str) -> config::File {
        let mut parsed = config::parse(text).unwrap();
        parsed.path = std::path::PathBuf::from(".clank/config.toml");
        parsed
    }

    #[test]
    fn flags_beat_the_file_and_the_default_provider_is_brave() {
        let cfg = file("[clank]\nbase_url = \"http://127.0.0.1:1/v1\"\nmodel = \"poison\"\n\n[web]\ndefault_provider = \"tavily\"\nlimit = 3\nformat = \"text\"\n");
        let got = resolve(Some("brave"), None, Some(2), Some("jsonl"), Some(&cfg), env_of(&[("BRAVE_API_KEY", "k")])).unwrap();
        assert_eq!(got.provider, Provider::Brave);
        assert_eq!(got.limit, 2);
        assert!(!got.text);
        assert_eq!(got.api_key, "k");
        assert_eq!(got.endpoint, BRAVE_ENDPOINT, "[clank].base_url is a chat server");

        let from_file = resolve(None, None, None, None, Some(&cfg), env_of(&[("TAVILY_API_KEY", "t")])).unwrap();
        assert_eq!(from_file.provider, Provider::Tavily);
        assert_eq!(from_file.limit, 3);
        assert!(from_file.text);
        assert_eq!(from_file.endpoint, TAVILY_ENDPOINT);

        let both = resolve(None, None, None, None, None, env_of(&[("BRAVE_API_KEY", "a"), ("TAVILY_API_KEY", "b")])).unwrap();
        assert_eq!(both.provider, Provider::Brave);
        assert_eq!(both.api_key, "a");
        assert_eq!(both.limit, DEFAULT_LIMIT);

        let err = resolve(None, None, None, None, None, env_of(&[("TAVILY_API_KEY", "t")])).unwrap_err();
        assert_eq!(err.1, FAIL, "{:?}", err.0);
        assert!(err.0.contains("BRAVE_API_KEY"), "{}", err.0);

        let err = resolve(None, None, None, None, None, env_of(&[])).unwrap_err();
        assert_eq!(err.1, FAIL, "{:?}", err.0);

        let named = file("[web.brave]\napi_key_env = \"SEARCH_TOKEN\"\n");
        let err = resolve(None, None, None, None, Some(&named), env_of(&[])).unwrap_err();
        assert!(err.0.contains("SEARCH_TOKEN"), "{}", err.0);
        assert_eq!(err.1, FAIL);

        let named = file("[web.brave]\napi_key_env = \"SEARCH_TOKEN\"\napi_key = \"inline-secret\"\n");
        let got = resolve(None, None, None, None, Some(&named), env_of(&[("BRAVE_API_KEY", "not-this"), ("SEARCH_TOKEN", "from-env")])).unwrap();
        assert_eq!(got.api_key, "from-env");
        let got = resolve(None, None, None, None, Some(&named), env_of(&[("BRAVE_API_KEY", "not-this")])).unwrap();
        assert_eq!(got.api_key, "inline-secret");

        let err = resolve(Some("nope"), None, None, None, None, env_of(&[])).unwrap_err();
        assert_eq!(err.1, USAGE);
        let err = resolve(None, None, Some(0), None, None, env_of(&[("BRAVE_API_KEY", "k")])).unwrap_err();
        assert_eq!(err.1, USAGE);
        let err = resolve(None, None, None, Some("csv"), None, env_of(&[("BRAVE_API_KEY", "k")])).unwrap_err();
        assert_eq!(err.1, USAGE);
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
        let a = args(&["--format", "text", "--limit", "3", "q"]);
        assert_eq!(a.format.as_deref(), Some("text"));
        assert_eq!(a.limit, Some(3));
    }
}
