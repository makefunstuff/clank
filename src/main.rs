//! clank — minimal unix-style local-inference harness.
//!
//! Unix contract:
//! - prompt: positional text, `-m TEXT`, or piped stdin
//! - context: piped stdin (when a prompt is also given), or a JSON tree file via `-c`
//! - items: `--each` runs the prompt once per stdin item, one framed answer each
//! - stdout = data only (assistant text, per-item frames, or `--jsonl` events)
//! - stderr = breadcrumbs and errors
//! - SIGPIPE restored, so `clank ... | head` dies like any unix tool
//! - exit codes: 0 ok, 1 failure (model/server/IO, or any failed item), 2 usage
//!
//! Composition is the design: the shell does the searching, the fan-out, the
//! checks and the looping; clank is the model-shaped stage inside that pipeline.
//! Hence the default: one prompt, one request, one answer, and the context is
//! exactly what was piped. `--tools` opts into the read-only observer loop
//! (read_file, list_dir, search, stat) for lookups the pipe did not cover --
//! never a substitute for piping the evidence.
//!
//! Tools and a schema are never in the same request (this server build rejects
//! the pair): with `--tools`, the rounds run unconstrained and then exactly one
//! final request carries the schema and no tools. See PROTOCOL.md.

mod client;
mod context;
mod tools;

use clap::Parser;
use serde_json::{json, Value};
use std::cell::Cell;
use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_MODEL: &str = "qwen3.8-27b-gsq-rco-iq3xxs";
const DEFAULT_BASE_URL: &str = "http://127.0.0.1:40583/v1";

#[derive(Debug)]
pub enum Fail {
    /// usage/config error -> exit 2
    Usage(String),
    /// model/server failure -> exit 1
    Model(String),
    /// filesystem/IO failure -> exit 1
    IO(String),
}

impl Fail {
    pub fn code(&self) -> i32 {
        match self {
            Fail::Usage(_) => 2,
            Fail::Model(_) | Fail::IO(_) => 1,
        }
    }
    pub fn msg(&self) -> &str {
        match self {
            Fail::Usage(m) | Fail::Model(m) | Fail::IO(m) => m,
        }
    }
}

#[derive(Parser)]
#[command(
    name = "clank",
    version,
    about = "Minimal unix-style local-inference harness (stdin/stdout, read-only tools)"
)]
struct Args {
    /// prompt text; when omitted, positional args (or piped stdin) form the prompt
    #[arg(short = 'm', long)]
    message: Option<String>,

    /// prompt text (space-joined) when -m is omitted
    #[arg(trailing_var_arg = true)]
    prompt: Vec<String>,

    /// context files: JSON tree, plain text, or clank JSONL transcript
    /// (repeatable; concatenated in the order given)
    #[arg(short = 'c', long, action = clap::ArgAction::Append)]
    context: Vec<PathBuf>,

    /// emit JSONL events on stdout instead of raw text
    #[arg(short = 'j', long)]
    jsonl: bool,

    /// suppress stderr breadcrumbs
    #[arg(short = 'q', long)]
    quiet: bool,

    /// run the prompt once per item on stdin, one framed answer per item
    #[arg(long)]
    each: bool,

    /// with --each: items are NUL-separated, like `find -print0`
    #[arg(short = '0', long = "null")]
    null_items: bool,

    /// model name
    #[arg(long, env = "CLANK_MODEL", default_value = DEFAULT_MODEL)]
    model: String,

    /// OpenAI-compatible base URL
    #[arg(long, env = "CLANK_BASE_URL", default_value = DEFAULT_BASE_URL)]
    base_url: String,

    /// API key (local servers usually need none)
    #[arg(long, env = "CLANK_API_KEY")]
    api_key: Option<String>,

    /// per-request timeout, seconds
    #[arg(long, value_name = "SECS", default_value_t = 600)]
    timeout: u64,

    /// maximum tool-call rounds per prompt
    #[arg(long, value_name = "N", default_value_t = 12)]
    max_rounds: usize,

    /// maximum completion tokens
    #[arg(long, value_name = "N", default_value_t = 8192)]
    max_tokens: u32,

    /// constrain the final answer to a JSON schema (inline JSON object)
    #[arg(long, value_name = "JSON")]
    json_schema: Option<String>,

    /// offer the read-only filesystem tools (default: on only when nothing was piped)
    #[arg(long)]
    tools: bool,

    /// never offer the tools, even with nothing to pipe
    #[arg(long, conflicts_with = "tools")]
    no_tools: bool,
    /// reasoning control: off, or a level the server's template may honour
    /// (minimal, low, medium, high, xhigh, max)
    #[arg(long, value_name = "LEVEL")]
    thinking: Option<String>,

    /// stream the model's reasoning to stderr; it is billed for either way
    #[arg(long)]
    show_thinking: bool,

    /// extra system directive, appended to the built-in system prompt
    #[arg(long, env = "CLANK_SYSTEM")]
    system: Option<String>,

    /// print the tool definitions as JSON and exit
    #[arg(long)]
    list_tools: bool,
}

fn main() {
    #[cfg(unix)]
    unsafe {
        // Rust installs SIG_IGN for SIGPIPE; restore the default disposition so
        // `clank ... | head` terminates like a normal unix tool, not a panic.
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let args = Args::parse();
    std::process::exit(run(args));
}

fn run(args: Args) -> i32 {
    match run_inner(&args) {
        Ok(code) => code,
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "clank: {}", e.msg());
            e.code()
        }
    }
}

fn run_inner(args: &Args) -> Result<i32, Fail> {
    // A schema may be written inline or authored on disk — `clank … > schema.json`
    // followed by `--json-schema @schema.json` is how a pipeline grows a
    // contract without anyone editing clank.
    let schema: Option<Value> = match &args.json_schema {
        Some(s) => Some(serde_json::from_str(&payload(s)?).map_err(|e| {
            Fail::Usage(format!("invalid --json-schema: {e}"))
        })?),
        None => None,
    };

    if args.list_tools {
        println!(
            "{}",
            serde_json::to_string_pretty(&tools::definitions()).unwrap_or_default()
        );
        return Ok(0);
    }

    // stdin: non-tty means content. With --each that content is the item list;
    // otherwise it is the prompt when no other prompt is given, else context.
    let stdin = if std::io::stdin().is_terminal() {
        None
    } else {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| Fail::Usage(format!("reading stdin: {e}")))?;
        Some(buf)
    };

    let mut prompt = args
        .message
        .clone()
        .or_else(|| (!args.prompt.is_empty()).then(|| args.prompt.join(" ")))
        .filter(|p| !p.trim().is_empty());

    // TTY without a prompt: read one line, unix-style.
    if prompt.is_none() && !args.each && std::io::stdin().is_terminal() {
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_ok() {
            prompt = Some(line.trim().to_string()).filter(|l| !l.is_empty());
        }
    }

    let items: Option<Vec<String>> = if args.each {
        let src = stdin.as_deref().ok_or_else(|| {
            Fail::Usage("--each reads its item list from stdin; pipe them in".into())
        })?;
        Some(parse_items(src, args.null_items))
    } else {
        None
    };

    let mut prompt_from_stdin = false;
    let prompt = match (prompt, args.each) {
        (Some(p), _) => p,
        (None, true) => {
            return Err(Fail::Usage(
                "--each: pass the prompt with -m TEXT or positionally (stdin is the item list)"
                    .into(),
            ))
        }
        // piped stdin is the prompt when nothing else is given
        (None, false) => match stdin.as_deref().filter(|s| !s.trim().is_empty()) {
            Some(s) => {
                prompt_from_stdin = true;
                s.to_string()
            }
            None => {
                return Err(Fail::Usage(
                    "no prompt: pass -m TEXT, positional text, pipe stdin, or run on a TTY".into(),
                ))
            }
        },
    };

    // context: `-c` files (repeatable) in order, then piped stdin as the last
    // node. Both are kept: a pipe that reaches the process is evidence, and
    // dropping it quietly would be the one bug this tool must not have.
    let mut nodes: Vec<context::Node> = Vec::new();
    for path in &args.context {
        nodes.extend(context::load_file(path)?.nodes);
    }
    if !args.each && !prompt_from_stdin {
        if let Some(s) = &stdin {
            if !s.trim().is_empty() {
                // The pipe is evidence: it parses leniently and never panics.
                nodes.extend(context::parse_evidence(s).nodes);
            }
        }
    }
    let tree: Option<context::Tree> = (!nodes.is_empty()).then_some(context::Tree { nodes });

    // Tools are the fallback for the case with no evidence: with nothing piped
    // and no context file and no items, the only honest way to answer a question
    // about the workspace is to look at it, and every lookup lands on stderr.
    // When evidence *was* supplied it is the evidence -- written by the shell's
    // own tools -- so the model gets that and nothing else. A schema asks for a
    // shaped answer from what it was given, so it does not switch the tools on.
    let evidence = tree.is_some() || items.is_some();
    let tools_enabled = if args.tools {
        true
    } else if args.no_tools {
        false
    } else {
        !evidence && schema.is_none()
    };

    let base = if tools_enabled {
        tools::SYSTEM_PROMPT_WITH_TOOLS
    } else {
        tools::SYSTEM_PROMPT
    };
    let mut system = base.to_string();
    if !evidence && !tools_enabled {
        // Nothing was piped and nothing can look: without this the model
        // answers an empty context with a fabricated `file:line` citation,
        // observed and reproduced on 2026-09-17.
        system.push_str(
            "\nNo context was provided for this question: answer from what you know, and \
             do not cite a file or line you were not given.\n",
        );
    }
    let system = match &args.system {
        Some(s) => format!("{system}\n\n{}", payload(s)?),
        None => system,
    };
    let mut head: Vec<Value> = vec![json!({ "role": "system", "content": system })];
    if let Some(t) = tree {
        head.push(json!({
            "role": "user",
            "content": format!("Context (tree, document order):\n\n{}", t.render()),
        }));
    }
    // The prompt is the last of the shared messages: with --each, everything
    // before the per-item block is byte-identical from item to item, which is
    // what lets the server reuse the prompt prefix.
    head.push(json!({ "role": "user", "content": prompt }));

    let thinking = match args.thinking.as_deref() {
        Some(level) => parse_thinking(level)?,
        None => None,
    };
    if args.jsonl {
        emit_run_header(args, &system, tools_enabled);
    }
    let runner = Runner::new(args, schema, thinking, tools_enabled);
    match items {
        None => {
            let out = Emitter::new(args, None);
            let ended = Cell::new(true);
            let mut emit = emit_deltas(!args.jsonl, &ended);
            let text = runner.converse(&head, None, &mut emit, &out)?;
            out.assistant(&text);
            Ok(0)
        }
        Some(items) => runner.run_items(&head, &items),
    }
}

/// Split an item list. Line mode trims trailing whitespace (CRLF pipes are
/// common); NUL mode keeps items byte-exact, as `find -print0` promises.
/// Items that are entirely whitespace are skipped -- there is nothing to ask.
fn parse_items(src: &str, null: bool) -> Vec<String> {
    let raw: Vec<&str> = if null {
        src.split('\0').collect()
    } else {
        src.lines().collect()
    };
    raw.into_iter()
        .filter(|s| !s.trim().is_empty())
        .map(|s| if null { s.to_string() } else { s.trim_end().to_string() })
        .collect()
}

/// A flag that carries a text payload accepts `@path` to read it from a file.
///
/// The artifacts a pipeline generates — a JSON schema, a reusable skill block —
/// are files; this is the read half of that convention. Anything not starting
/// with `@` is the literal value.
fn payload(value: &str) -> Result<String, Fail> {
    let Some(path) = value.strip_prefix('@') else {
        return Ok(value.to_string());
    };
    std::fs::read_to_string(path)
        .map_err(|e| Fail::IO(format!("cannot read {path}: {e}")))
}

/// `--thinking off` disables thinking through the chat template; a level is
/// passed to the server and the template decides whether it can honour it.
/// `default` means "say nothing", which is also what omitting the flag does.
fn parse_thinking(level: &str) -> Result<Option<client::Thinking>, Fail> {
    let level = level.trim().to_ascii_lowercase();
    let thinking = match level.as_str() {
        "" | "default" | "auto" => return Ok(None),
        "off" | "none" | "false" | "no" => client::Thinking::Off,
        "minimal" | "low" | "medium" | "high" | "xhigh" | "max" => {
            client::Thinking::Effort(level)
        }
        other => {
            return Err(Fail::Usage(format!(
                "--thinking: unknown level `{other}` (off, minimal, low, medium, high, xhigh, max)"
            )))
        }
    };
    Ok(Some(thinking))
}

/// The first line of a `--jsonl` run: what produced this trace. A trace is
/// evidence, and evidence without provenance cannot be checked later.
fn emit_run_header(args: &Args, system: &str, tools_enabled: bool) {
    println!(
        "{}",
        json!({
            "type": "run",
            "clank": env!("CARGO_PKG_VERSION"),
            // Which system prompt produced this answer: the built-in one alone,
            // or one with a skill appended. Two traces stay comparable across a
            // prompt change because this changes when the text does.
            "prompt": prompt_id(system),
            "model": args.model,
            "base_url": args.base_url,
            "tools": tools_enabled,
            "thinking": args.thinking,
            "argv": redacted_argv(&std::env::args().collect::<Vec<_>>()),
        })
    );
}

/// A short, stable id for a system prompt: FNV-1a over the text, so no
/// dependency and the same answer on any machine.
fn prompt_id(system: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in system.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", hash >> 32)
}

/// A key on the command line must not end up in a trace file.
fn redacted_argv(argv: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(argv.len());
    let mut hide_next = false;
    for arg in argv {
        if hide_next {
            out.push("***".to_string());
            hide_next = false;
        } else if arg == "--api-key" {
            hide_next = true;
            out.push(arg.clone());
        } else if arg.starts_with("--api-key=") {
            out.push("--api-key=***".to_string());
        } else {
            out.push(arg.clone());
        }
    }
    out
}

/// Assistant text streams to stdout as it arrives. In `--jsonl` mode stdout is
/// a fixed set of events instead, so deltas are suppressed.
///
/// `ended` records whether the last thing written was a newline, so a framed
/// answer can be terminated before the next frame starts.
fn emit_deltas<'a>(on: bool, ended: &'a Cell<bool>) -> impl FnMut(&str) + 'a {
    move |delta: &str| {
        if on {
            let _ = print!("{delta}");
            let _ = std::io::stdout().flush();
            ended.set(delta.ends_with('\n'));
        }
    }
}

// ------------------------------------------------------------------- output

/// The two output channels in one place: stdout carries data (text, framed
/// answers, or JSONL events), stderr carries breadcrumbs and errors.
struct Emitter<'a> {
    args: &'a Args,
    /// (index, total) of the item being answered, when `--each` is in play
    item: Option<(usize, usize)>,
}

impl<'a> Emitter<'a> {
    fn new(args: &'a Args, item: Option<(usize, usize)>) -> Self {
        Self { args, item }
    }

    /// One JSONL event, tagged with the item it belongs to.
    fn event(&self, kind: &str, mut body: Value) {
        body["type"] = json!(kind);
        if let Some((i, of)) = self.item {
            body["i"] = json!(i);
            body["of"] = json!(of);
        }
        println!("{body}");
    }

    fn item_start(&self, index: usize, total: usize, text: &str) {
        if self.args.jsonl {
            self.event("item", json!({ "input": text }));
        } else {
            if index > 1 {
                println!();
            }
            println!("─── item {index}/{total} ───");
        }
    }

    fn tool_call(&self, name: &str, args: &Value) {
        if self.args.jsonl {
            self.event("tool_call", json!({ "name": name, "arguments": args }));
        } else if !self.args.quiet {
            let _ = eprintln!("> {} {}", name, tools::compact_args(args));
        }
    }

    fn tool_result(&self, name: &str, ok: bool, output: &str) {
        if self.args.jsonl {
            self.event(
                "tool_result",
                json!({ "name": name, "ok": ok, "output": output }),
            );
        } else if !self.args.quiet {
            let _ = eprintln!(
                "< {} {} ({} B)",
                name,
                if ok { "ok" } else { "err" },
                output.len()
            );
        }
    }

    fn assistant(&self, text: &str) {
        if self.args.jsonl {
            self.event("assistant", json!({ "content": text }));
        }
    }

    fn item_error(&self, index: usize, total: usize, message: &str) {
        if self.args.jsonl {
            self.event("error", json!({ "message": message }));
        }
        if !self.args.quiet {
            let _ = eprintln!("clank: item {index}/{total}: {message}");
        }
    }

}

// ------------------------------------------------------------------- runner

struct Runner<'a> {
    args: &'a Args,
    agent: ureq::Agent,
    schema: Option<Value>,
    tools_json: Value,
    debug: Option<String>,
    thinking: Option<client::Thinking>,
    tools_enabled: bool,
}

impl<'a> Runner<'a> {
    fn new(
        args: &'a Args,
        schema: Option<Value>,
        thinking: Option<client::Thinking>,
        tools_enabled: bool,
    ) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_per_call(Some(Duration::from_secs(args.timeout)))
            .http_status_as_error(false)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            tools_json: if tools_enabled {
                tools::definitions()
            } else {
                json!([])
            },
            debug: std::env::var("CLANK_DEBUG").ok(),
            schema,
            thinking,
            tools_enabled,
            args,
        }
    }

    fn stream(
        &self,
        messages: &[Value],
        tools: &Value,
        schema: Option<&Value>,
        emit: &mut dyn FnMut(&str),
        on_thinking: &mut dyn FnMut(&str),
    ) -> Result<client::Round, Fail> {
        let round = client::stream_round(
            &self.agent,
            client::Request {
                base_url: &self.args.base_url,
                model: &self.args.model,
                api_key: self.args.api_key.as_deref(),
                max_tokens: self.args.max_tokens,
                messages,
                tools,
                json_schema: schema,
                thinking: self.thinking.as_ref(),
                debug_path: self.debug.as_deref(),
            },
            emit,
            on_thinking,
        )?;
        // Reasoning streams to stderr and does not end its own line, so the next
        // diagnostic starts on a fresh one.
        if self.args.show_thinking {
            let _ = writeln!(std::io::stderr());
        }
        Ok(round)
    }

    /// One prompt, one answer, one conversation -- and by default that is one
    /// request, because the context is what was piped in.
    ///
    /// Phase 1 is the `--tools` observer loop and is skipped unless asked for.
    /// Phase 2 happens only when a schema was requested, and it is the only
    /// request that carries the schema: this server build rejects tools and
    /// schema together, and an intermediate round is not an answer anyway.
    ///
    /// Two things are failures here rather than results, because a pipeline
    /// stage that reports success it cannot back is worse than one that fails:
    /// a round the server cut off at the token budget, and an answer with no
    /// text at all (a model that spends its whole budget thinking emits
    /// neither text nor a tool call, and that is not an empty answer -- it is
    /// no answer).
    fn converse(
        &self,
        head: &[Value],
        tail: Option<Value>,
        emit: &mut dyn FnMut(&str),
        out: &Emitter,
    ) -> Result<String, Fail> {
        let mut messages: Vec<Value> = head.to_vec();
        if let Some(t) = tail {
            messages.push(t);
        }

        let mut interim = String::new();
        let mut explored = false;
        let mut noop = |_: &str| {};
        let showing = self.args.show_thinking;
        let mut thinking_sink = move |delta: &str| {
            if showing {
                let _ = write!(std::io::stderr(), "{delta}");
                let _ = std::io::stderr().flush();
            }
        };

        if self.tools_enabled {
            let mut rounds = 0usize;
            loop {
                // No schema while tools are on: the pair is rejected by the
                // server, and the loop's own text is not the answer.
                let sink: &mut dyn FnMut(&str) = if self.schema.is_some() {
                    &mut noop
                } else {
                    emit
                };
                let round = self.stream(&messages, &self.tools_json, None, sink, &mut thinking_sink)?;
                self.reject_truncated(&round)?;

                if round.tool_calls.is_empty() {
                    interim = round.text;
                    break;
                }

                rounds += 1;
                if rounds > self.args.max_rounds {
                    return Err(Fail::Model(format!(
                        "stopped after {} tool rounds (--max-rounds)",
                        self.args.max_rounds
                    )));
                }
                explored = true;

                let ids: Vec<String> = round
                    .tool_calls
                    .iter()
                    .enumerate()
                    .map(|(k, tc)| {
                        tc.id.clone().unwrap_or_else(|| format!("call_{rounds}_{k}"))
                    })
                    .collect();
                let assistant_tcs: Vec<Value> = round
                    .tool_calls
                    .iter()
                    .zip(&ids)
                    .map(|(tc, id)| {
                        json!({
                            "id": id,
                            "type": "function",
                            "function": { "name": tc.name, "arguments": tc.arguments },
                        })
                    })
                    .collect();
                let content = if round.text.is_empty() {
                    Value::Null
                } else {
                    Value::String(round.text.clone())
                };
                // assistant message first, then its tool results (OpenAI order).
                messages.push(json!({ "role": "assistant", "content": content, "tool_calls": assistant_tcs }));

                for (tc, id) in round.tool_calls.iter().zip(ids) {
                    let args_v: Value = serde_json::from_str(&tc.arguments).unwrap_or(Value::Null);
                    out.tool_call(&tc.name, &args_v);
                    let (text, ok) = tools::execute(&tc.name, &args_v);
                    out.tool_result(&tc.name, ok, &text);
                    messages.push(json!({ "role": "tool", "tool_call_id": id, "content": text }));
                }
            }
        } else if self.schema.is_none() {
            // The one-shot path, and the default one: one prompt, one request,
            // one answer. The context is what was piped in; nothing else.
            let round = self.stream(&messages, &json!([]), None, emit, &mut thinking_sink)?;
            self.reject_truncated(&round)?;
            interim = round.text;
        }

        let Some(schema) = self.schema.as_ref() else {
            if interim.trim().is_empty() {
                return Err(Fail::Model(
                    "no answer: the model returned no text and called no tool".into(),
                ));
            }
            return Ok(interim);
        };

        if !interim.trim().is_empty() {
            // The model's own reading of the evidence, kept as context for the
            // constrained answer rather than thrown away.
            messages.push(json!({ "role": "assistant", "content": interim }));
        }
        if explored || !interim.trim().is_empty() {
            messages.push(json!({
                "role": "user",
                "content": format!(
                    "Answer now, using the evidence above, with a single JSON object matching \
                     this schema and nothing else:\n{}",
                    serde_json::to_string(schema).unwrap_or_default()
                ),
            }));
        }

        let round = self.stream(&messages, &json!([]), Some(schema), emit, &mut thinking_sink)?;
        self.reject_truncated(&round)?;
        if serde_json::from_str::<Value>(&round.text).is_err() {
            return Err(Fail::Model(
                "final output is not valid JSON; --json-schema requested".into(),
            ));
        }
        Ok(round.text)
    }

    /// A round the server cut off at the token budget is half an answer. It is
    /// never a stage result: the exit code has to say so instead.
    fn reject_truncated(&self, round: &client::Round) -> Result<(), Fail> {
        if round.finish_reason.as_deref() == Some("length") {
            return Err(Fail::Model(format!(
                "answer truncated at {} tokens (--max-tokens); raise it or narrow the prompt",
                self.args.max_tokens
            )));
        }
        Ok(())
    }

    /// `--each`: the same prompt, once per item, serially.
    fn run_items(&self, head: &[Value], items: &[String]) -> Result<i32, Fail> {
        let total = items.len();
        if total == 0 {
            if !self.args.quiet {
                let _ = writeln!(std::io::stderr(), "clank: no items on stdin; nothing to do");
            }
            return Ok(0);
        }

        let mut failed = 0usize;
        for (index, item) in items.iter().enumerate() {
            let i = index + 1;
            let out = Emitter::new(self.args, Some((i, total)));
            out.item_start(i, total, item);

            let block = format!("─── item {i}/{total} ───\n{item}");
            let tail = json!({ "role": "user", "content": block });
            let ended = Cell::new(true);
            let mut emit = emit_deltas(!self.args.jsonl, &ended);

            let answer = self.converse(head, Some(tail), &mut emit, &out);
            // A frame is line-oriented: an answer that does not end its own
            // line would run into the next header.
            if !self.args.jsonl && !ended.get() {
                println!();
            }

            match answer {
                Ok(text) => out.assistant(&text),
                Err(e) => {
                    failed += 1;
                    out.item_error(i, total, e.msg());
                }
            }
        }
        out_summary(self.args, total, failed);
        Ok(if failed > 0 { 1 } else { 0 })
    }
}

fn out_summary(args: &Args, total: usize, failed: usize) {
    if !args.quiet {
        let _ = writeln!(std::io::stderr(), "clank: {total} items, {failed} failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_items_drop_blank_lines_and_cr() {
        assert_eq!(
            parse_items("a\n\n  \nb\r\n", false),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn nul_items_keep_whitespace_but_not_empty_chunks() {
        assert_eq!(
            parse_items("a b\0\0c\0", true),
            vec!["a b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn an_empty_item_list_is_empty_not_one_blank_item() {
        assert!(parse_items("\n\n", false).is_empty());
        assert!(parse_items("", true).is_empty());
    }

    #[test]
    fn items_are_not_split_on_spaces() {
        assert_eq!(parse_items("one two\n", false), vec!["one two".to_string()]);
    }

    #[test]
    fn a_payload_flag_reads_a_file_or_takes_a_literal() {
        assert_eq!(payload("literal").unwrap(), "literal");
        let path = std::env::temp_dir().join(format!("clank-payload-{}.md", std::process::id()));
        std::fs::write(&path, "from disk").unwrap();
        assert_eq!(payload(&format!("@{}", path.display())).unwrap(), "from disk");
        let missing = payload("@/definitely/not/here").unwrap_err();
        assert_eq!(missing.code(), 1, "a missing file is an IO failure, not a usage error");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_prompt_id_is_stable_and_distinguishes_prompts() {
        let base = "you are clank";
        assert_eq!(prompt_id(base), prompt_id(base), "same text, same id");
        assert_ne!(prompt_id(base), prompt_id(&format!("{base} plus a skill")));
        assert_eq!(prompt_id(base).len(), 8, "short enough for a trace line");
        assert!(prompt_id(base).chars().all(|c| c.is_ascii_hexdigit()));
    }
}
