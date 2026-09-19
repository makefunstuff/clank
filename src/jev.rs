//! clank-jev — typed decisions, for routing in scripts.
//!
//! The sibling of `clank`, not a mode inside it. `clank` answers a prompt in
//! prose; this answers *your* options with probabilities, so a shell can branch:
//!
//! ```sh
//! route=$(printf '%s' "$task" | clank-jev --ask 'What kind of task is this?' \
//!           --choice code,prose,math --min-prob 0.7) || route=unclear
//! case "$route" in
//!   code) clank --model local-code -m "$task" ;;
//!   *)    clank --model local-fast -m "$task" ;;
//! esac
//! ```
//!
//! It is a separate binary for two reasons: `clank`'s single-stage contract stays
//! intact (one prompt, one request, one answer, no second wire protocol), and a
//! decision stage is useful on its own — in a cron job, a git hook, a Makefile.
//!
//! Credentials come from the environment, never from argv:
//!   `TYPESAFE_API_KEY` (or `JEV_API_KEY`, `JEV_CLI_API_KEY`) -> api.typesafe.ai
//!   `OPENROUTER_API_KEY` -> openrouter.ai Decisions endpoint (the same Jev)
//!
//! Exit codes: 0 decided and passed every gate · 1 a gate failed (the decision is
//! usable, you asked not to trust it) · 2 usage · 3 provider, network or credentials.

use clap::Parser;
use serde_json::{json, Map, Value};
use std::io::Read;
use std::time::{Duration, Instant};

const TYPESAFE_URL: &str = "https://api.typesafe.ai/v1/systemone";
const TYPESAFE_MODEL: &str = "jev-latest";
const OPENROUTER_URL: &str = "https://openrouter.ai/api/alpha/decisions";
const OPENROUTER_MODEL: &str = "typesafe/jev-1.13";
/// A local, trained Jev-family model (kev, or anything else speaking the same
/// contract) needs no credentials: `kev.serve` listens here by default.
const KEV_URL: &str = "http://127.0.0.1:8009/v1/systemone";
const KEV_MODEL: &str = "kev-latest";
const DEFAULT_ID: &str = "answer";

/// Exit codes, named once so the tests and the docs agree with the code.
const OK: i32 = 0;
const GATE_FAILED: i32 = 1;
const USAGE: i32 = 2;
const PROVIDER: i32 = 3;

#[derive(Parser, Debug)]
#[command(
    name = "clank-jev",
    version,
    about = "Typed decisions from a state, for routing in scripts",
    after_help = "Examples:\n  \
      echo \"$task\" | clank-jev --ask 'What kind of task is this?' --choice code,prose,math\n  \
      echo \"$text\" | clank-jev --ask 'Is this a refund request?' --boolean\n  \
      cat state.txt | clank-jev --checks checks.json --min-prob 0.8 --json"
)]
struct Args {
    /// the question to decide, e.g. 'Which team should own this?'
    #[arg(long, value_name = "TEXT")]
    ask: Option<String>,

    /// options for --ask, comma separated (a choice question)
    #[arg(long, value_name = "A,B,C", conflicts_with_all = ["boolean", "score", "checks"])]
    choice: Option<String>,

    /// --ask is yes/no, printed as true or false
    #[arg(long, conflicts_with_all = ["choice", "score", "checks"])]
    boolean: bool,

    /// ordered levels for --ask, comma separated, lowest first (a score question)
    #[arg(long, value_name = "LOW,MID,HIGH", conflicts_with_all = ["choice", "boolean", "checks"])]
    score: Option<String>,

    /// a JSON file of questions: {id: {type, instructions, criteria}}
    #[arg(long, value_name = "FILE", conflicts_with_all = ["ask", "choice", "boolean", "score"])]
    checks: Option<String>,

    /// exit 1 if any answer's probability is below this
    #[arg(long, value_name = "P")]
    min_prob: Option<f64>,

    /// exit 1 unless the decision equals this value (single question)
    #[arg(long, value_name = "VALUE")]
    expect: Option<String>,

    /// exit 1 unless an ordered decision is at least this level (single question)
    #[arg(long, value_name = "N")]
    expect_min: Option<i64>,

    /// provider: auto (default), typesafe, openrouter, or kev (a local server,
    /// no credentials; --base-url overrides where it listens)
    #[arg(long, value_name = "NAME", default_value = "auto")]
    provider: String,

    /// model id; defaults to jev-latest (typesafe) or typesafe/jev-1.13 (openrouter)
    #[arg(long, value_name = "ID")]
    model: Option<String>,

    /// overrides the provider endpoint, for a local stub or a proxy
    #[arg(long, value_name = "URL")]
    base_url: Option<String>,

    /// per-request timeout, seconds
    #[arg(long, value_name = "SECS", default_value_t = 60)]
    timeout: u64,

    /// print the full result object instead of the bare value
    #[arg(long)]
    json: bool,

    /// print the closed-choice reason instead of the decided value
    #[arg(long)]
    print_reason: bool,

    /// suppress the diagnostic line on stderr
    #[arg(short, long)]
    quiet: bool,
}

// ---------------------------------------------------------------- questions

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    /// wire type `noul`
    Bool,
    Choice,
    Score,
}

#[derive(Debug, Clone)]
struct Question {
    id: String,
    kind: Kind,
    instructions: String,
    /// choice: option name -> description; score: the levels in order.
    criteria: Value,
    /// A closed set of reasons, decided in the *same* request as the value, so a
    /// caller learns not just what was decided but on what basis. The vocabulary
    /// lives in the checks file, not in this binary.
    reasons: Option<Value>,
}

fn split_list(s: &str) -> Vec<String> {
    s.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()
}

fn parse_checks(text: &str) -> Result<Vec<Question>, String> {
    let parsed: Value = serde_json::from_str(text).map_err(|e| format!("--checks is not JSON: {e}"))?;
    let obj = parsed.as_object().ok_or("--checks must be a JSON object of question ids")?;
    if obj.is_empty() {
        return Err("--checks has no questions".into());
    }
    let mut out = Vec::new();
    for (id, q) in obj {
        let q = q.as_object().ok_or(format!("question {id:?} is not an object"))?;
        let instructions = q
            .get("instructions")
            .and_then(Value::as_str)
            .ok_or(format!("question {id:?} needs string `instructions`"))?
            .to_string();
        let wire = q.get("type").and_then(Value::as_str).ok_or(format!("question {id:?} needs `type`"))?;
        let (kind, criteria) = match wire {
            // TypeSafe's wire type is `noul`; `boolean` is the friendlier spelling.
            "noul" | "boolean" => (Kind::Bool, q.get("criteria").cloned().unwrap_or(Value::Null)),
            "choice" => {
                let c = q.get("criteria").cloned().ok_or(format!("choice {id:?} needs `criteria`"))?;
                if !c.is_object() && !c.is_array() {
                    return Err(format!("choice {id:?} criteria must be an object of options or an array"));
                }
                (Kind::Choice, c)
            }
            "score" => {
                let c = q.get("criteria").cloned().ok_or(format!("score {id:?} needs `criteria`"))?;
                if c.as_array().map(|a| a.len() < 2).unwrap_or(true) {
                    return Err(format!("score {id:?} criteria must be an array of at least two levels"));
                }
                (Kind::Score, c)
            }
            other => return Err(format!("question {id:?} has unknown type {other:?}")),
        };
        let reasons = match q.get("reasons") {
            None => None,
            Some(v) if v.is_object() && !v.as_object().unwrap().is_empty() => Some(v.clone()),
            Some(_) => return Err(format!("question {id:?} reasons must be a non-empty object")),
        };
        out.push(Question { id: id.clone(), kind, instructions, criteria, reasons });
    }
    Ok(out)
}

fn questions_from_args(args: &Args) -> Result<Vec<Question>, String> {
    if let Some(path) = &args.checks {
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading --checks {path}: {e}"))?;
        return parse_checks(&text);
    }
    let ask = args.ask.clone().ok_or("--ask or --checks is required")?;
    let (kind, criteria) = if let Some(list) = &args.choice {
        let opts = split_list(list);
        if opts.len() < 2 {
            return Err("--choice needs at least two options".into());
        }
        let map: Map<String, Value> =
            opts.iter().enumerate().map(|(i, o)| (o.clone(), json!(format!("option {}", i + 1)))).collect();
        (Kind::Choice, Value::Object(map))
    } else if args.boolean {
        (Kind::Bool, Value::Null)
    } else if let Some(list) = &args.score {
        let levels = split_list(list);
        if levels.len() < 2 {
            return Err("--score needs at least two levels".into());
        }
        (Kind::Score, json!(levels))
    } else {
        return Err("one of --choice, --boolean or --score is required with --ask".into());
    };
    Ok(vec![Question { id: DEFAULT_ID.to_string(), kind, instructions: ask, criteria, reasons: None }])
}

/// The provider's body: TypeSafe wants `noul`, OpenRouter too.
fn body_for(model: &str, state: &str, questions: &[Question]) -> Value {
    let mut qs = Map::new();
    for q in questions {
        let mut entry = Map::new();
        entry.insert(
            "type".into(),
            json!(match q.kind {
                Kind::Bool => "noul",
                Kind::Choice => "choice",
                Kind::Score => "score",
            }),
        );
        entry.insert("instructions".into(), json!(q.instructions));
        if !q.criteria.is_null() {
            entry.insert("criteria".into(), q.criteria.clone());
        }
        qs.insert(q.id.clone(), Value::Object(entry));
    }
    // The reason is a second question in the same request: Jev answers all
    // questions against one state in one pass, so this costs almost nothing.
    for q in questions {
        if let Some(reasons) = &q.reasons {
            qs.insert(
                reason_id(&q.id),
                json!({
                    "type": "choice",
                    "instructions": format!(
                        "Which of these is the reason for the answer to {:?}? Answer from this list only.",
                        q.instructions
                    ),
                    "criteria": reasons,
                }),
            );
        }
    }
    json!({ "model": model, "state": state, "questions": qs })
}

// ---------------------------------------------------------------- answers

#[derive(Debug, Clone, PartialEq)]
struct Answer {
    kind: Kind,
    value: Value,
    probability: Option<f64>,
    distribution: Option<Value>,
    /// The closed-choice reason, when the question declared one.
    reason: Option<String>,
    reason_probability: Option<f64>,
    /// The provider's own confidence, when it sends one (Jev does, for choice and
    /// score: a normalized margin rather than the winner's share).
    provider_confidence: Option<f64>,
}

impl Answer {
    /// How sure the decision is, as opposed to how probable "yes" is. For a yes/no
    /// question those differ: a confident *no* has a probability near 0, and a gate
    /// on the raw probability would reject it for being decisive.
    fn confidence(&self) -> Option<f64> {
        match self.kind {
            Kind::Bool => self.probability.map(|p| p.max(1.0 - p)),
            _ => self.provider_confidence.or(self.probability),
        }
    }
}

/// One answer, in the shape both providers return once normalised.
fn read_answer(kind: &Kind, raw: &Value) -> Result<Answer, String> {
    let obj = raw.as_object().ok_or("answer is not an object")?;
    let num = |k: &str| obj.get(k).and_then(Value::as_f64);
    match kind {
        Kind::Bool => {
            let p = num("noul").ok_or("a noul answer without `noul`")?;
            Ok(Answer {
                kind: Kind::Bool,
                value: json!(p >= 0.5),
                probability: Some(p),
                distribution: None,
                reason: None,
                reason_probability: None,
                provider_confidence: num("confidence"),
            })
        }
        Kind::Choice => {
            // `choice` is the option name on the TypeSafe route; the OpenRouter
            // route has answered with the distribution itself, so take both shapes.
            let (chosen, dist) = match obj.get("choice") {
                Some(Value::String(s)) => (s.clone(), obj.get("probabilities").cloned()),
                Some(Value::Object(map)) => {
                    let (top, _) = map
                        .iter()
                        .max_by(|a, b| a.1.as_f64().unwrap_or(0.0).total_cmp(&b.1.as_f64().unwrap_or(0.0)))
                        .ok_or("empty choice distribution")?;
                    (top.clone(), Some(Value::Object(map.clone())))
                }
                _ => return Err("a choice answer without `choice` or a distribution".into()),
            };
            let probability = dist
                .as_ref()
                .and_then(|d| d.get(&chosen))
                .and_then(Value::as_f64)
                .or_else(|| num("confidence"));
            Ok(Answer {
                kind: Kind::Choice,
                value: json!(chosen),
                probability,
                distribution: dist,
                reason: None,
                reason_probability: None,
                provider_confidence: num("confidence"),
            })
        }
        Kind::Score => {
            // TypeSafe returns a fractional, probability-weighted position; the
            // OpenRouter route returns a level index. A script wants the level,
            // so the decided value is the winning level and `score` keeps the raw.
            let dist = obj.get("probabilities").cloned();
            let level = dist
                .as_ref()
                .and_then(|d| d.as_object())
                .and_then(|m| {
                    m.iter()
                        .filter_map(|(k, v)| v.as_f64().map(|v| (k.parse::<i64>().ok(), v)))
                        .filter_map(|(k, v)| k.map(|k| (k, v)))
                        .max_by(|a, b| a.1.total_cmp(&b.1))
                        .map(|(k, _)| k)
                })
                .or_else(|| obj.get("score").and_then(Value::as_f64).map(|s| s.round() as i64))
                .ok_or("a score answer without `score` or probabilities")?;
            let probability = dist
                .as_ref()
                .and_then(|d| d.get(level.to_string()))
                .and_then(Value::as_f64)
                .or_else(|| num("confidence"));
            Ok(Answer {
                kind: Kind::Score,
                value: json!(level),
                probability,
                distribution: dist,
                reason: None,
                reason_probability: None,
                provider_confidence: num("confidence"),
            })
        }
    }
}

fn reason_id(id: &str) -> String {
    format!("{id}.reason")
}

fn read_answers(questions: &[Question], payload: &Value) -> Result<Vec<Answer>, String> {
    let answers = payload
        .get("answers")
        .and_then(Value::as_object)
        .ok_or("the provider returned no `answers` object")?;
    let mut out = Vec::new();
    for q in questions {
        let raw = answers.get(&q.id).ok_or(format!("the provider skipped question {:?}", q.id))?;
        let mut answer = read_answer(&q.kind, raw).map_err(|e| format!("question {:?}: {e}", q.id))?;
        if q.reasons.is_some() {
            let rid = reason_id(&q.id);
            let rraw = answers
                .get(&rid)
                .ok_or(format!("the provider skipped the reason question {rid:?}"))?;
            let reason = read_answer(&Kind::Choice, rraw)
                .map_err(|e| format!("reason for {:?}: {e}", q.id))?;
            answer.reason = reason.value.as_str().map(str::to_string);
            answer.reason_probability = reason.probability;
        }
        out.push(answer);
    }
    Ok(out)
}

// ---------------------------------------------------------------- gates

#[derive(Debug, PartialEq)]
struct Gate {
    min_prob: Option<f64>,
    expect: Option<String>,
    expect_min: Option<i64>,
}

/// Returns the reasons the decision must not be trusted; empty means it passed.
fn gate_failures(args: &Args, questions: &[Question], answers: &[Answer]) -> Vec<String> {
    let gate = Gate {
        min_prob: args.min_prob,
        expect: args.expect.clone(),
        expect_min: args.expect_min,
    };
    let mut failures = Vec::new();
    if (gate.expect.is_some() || gate.expect_min.is_some()) && questions.len() != 1 {
        failures.push("--expect/--expect-min need exactly one question".into());
        return failures;
    }
    for (q, a) in questions.iter().zip(answers) {
        if let Some(min) = gate.min_prob {
            let conf = a.confidence();
            match conf {
                Some(c) if c >= min => {}
                Some(c) => failures.push(format!(
                    "{}: confidence {c:.3} below --min-prob {min} (decided {}, p={:.3})",
                    q.id,
                    a.value,
                    a.probability.unwrap_or(f64::NAN)
                )),
                None => failures.push(format!("{}: no probability to check against --min-prob {min}", q.id)),
            }
        }
        if let Some(want) = &gate.expect {
            let got = match &a.value {
                Value::String(s) => s.clone(),
                Value::Bool(b) => b.to_string(),
                Value::Number(n) => n.to_string(),
                _ => String::new(),
            };
            if !got.eq_ignore_ascii_case(want.trim()) {
                failures.push(format!("{}: expected {want:?}, decided {got:?}", q.id));
            }
        }
        if let Some(min) = gate.expect_min {
            match a.value.as_i64() {
                Some(v) if v >= min => {}
                Some(v) => failures.push(format!("{}: decided level {v} below --expect-min {min}", q.id)),
                None => failures.push(format!("{}: --expect-min needs an ordered decision, got {}", q.id, a.value)),
            }
        }
    }
    failures
}

// ---------------------------------------------------------------- provider

/// Which Jev route the decision came from.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Provider {
    Typesafe,
    Openrouter,
    /// A local System One server: same request and response shapes, no auth.
    Kev,
}

impl Provider {
    fn name(self) -> &'static str {
        match self {
            Provider::Typesafe => "typesafe",
            Provider::Openrouter => "openrouter",
            Provider::Kev => "kev",
        }
    }

    fn needs_key(self) -> bool {
        !matches!(self, Provider::Kev)
    }
}

fn resolve_provider(args: &Args) -> Result<(Provider, String, String, String), (String, i32)> {
    let typesafe_key = ["TYPESAFE_API_KEY", "JEV_API_KEY", "JEV_CLI_API_KEY"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()));
    let openrouter_key = std::env::var("OPENROUTER_API_KEY").ok().filter(|v| !v.trim().is_empty());

    let (provider, key) = match args.provider.as_str() {
        "kev" | "local" => (Provider::Kev, String::new()),
        "typesafe" => (Provider::Typesafe, typesafe_key.ok_or_else(|| {
            ("no TypeSafe key: set TYPESAFE_API_KEY (or JEV_API_KEY)".to_string(), PROVIDER)
        })?),
        "openrouter" => (Provider::Openrouter, openrouter_key.ok_or_else(|| {
            ("no OpenRouter key: set OPENROUTER_API_KEY".to_string(), PROVIDER)
        })?),
        "auto" => match (typesafe_key, openrouter_key) {
            (Some(k), _) => (Provider::Typesafe, k),
            (None, Some(k)) => (Provider::Openrouter, k),
            (None, None) => {
                return Err((
                    "no Jev credentials: set TYPESAFE_API_KEY (or JEV_API_KEY), or OPENROUTER_API_KEY".into(),
                    PROVIDER,
                ))
            }
        },
        other => {
            return Err((format!("unknown --provider {other:?} (auto, typesafe, openrouter, kev)"), USAGE))
        }
    };
    let default_model = match provider {
        Provider::Typesafe => TYPESAFE_MODEL,
        Provider::Openrouter => OPENROUTER_MODEL,
        Provider::Kev => KEV_MODEL,
    };
    let default_url = match provider {
        Provider::Typesafe => TYPESAFE_URL,
        Provider::Openrouter => OPENROUTER_URL,
        Provider::Kev => KEV_URL,
    };
    Ok((
        provider,
        args.model.clone().unwrap_or_else(|| default_model.to_string()),
        args.base_url.clone().unwrap_or_else(|| default_url.to_string()),
        key,
    ))
}

fn decide(args: &Args, questions: &[Question], state: &str) -> Result<(Value, Provider, String, u128), (String, i32)> {
    let (provider, model, url, key) = resolve_provider(args)?;
    let body = body_for(&model, state, questions);
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(args.timeout)))
        // A provider's 4xx/5xx carries the reason in its body; treat the status as
        // data so that reason can be printed instead of "http status: 422".
        .http_status_as_error(false)
        .build()
        .new_agent();
    let started = Instant::now();
    let mut request = agent.post(&url).header("Content-Type", "application/json");
    if provider.needs_key() {
        request = request.header("Authorization", format!("Bearer {key}"));
    }
    let mut resp = request
        .send_json(&body)
        .map_err(|e| (format!("request to {url} failed: {e}"), PROVIDER))?;
    let status = resp.status();
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| (format!("reading the provider response: {e}"), PROVIDER))?;
    if status != 200 {
        let detail = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| {
                ["detail", "message", "error"]
                    .iter()
                    .find_map(|k| v.get(*k).map(|d| d.to_string()))
            })
            .unwrap_or_else(|| text.trim().chars().take(300).collect());
        return Err((format!("provider returned HTTP {status}: {detail}"), PROVIDER));
    }
    let payload: Value =
        serde_json::from_str(&text).map_err(|e| (format!("provider returned non-JSON: {e}"), PROVIDER))?;
    Ok((payload, provider, model, started.elapsed().as_millis()))
}

// ---------------------------------------------------------------- output

fn render(args: &Args, questions: &[Question], answers: &[Answer], payload: &Value, provider: Provider,
          model: &str, elapsed_ms: u128, failures: &[String]) -> Value {
    let mut out = Map::new();
    out.insert("provider".into(), json!(provider.name()));
    out.insert("model".into(), json!(payload.get("model").and_then(Value::as_str).unwrap_or(model)));
    let mut ans = Map::new();
    for (q, a) in questions.iter().zip(answers) {
        let mut e = Map::new();
        e.insert("type".into(), json!(match a.kind {
            Kind::Bool => "noul",
            Kind::Choice => "choice",
            Kind::Score => "score",
        }));
        e.insert("value".into(), a.value.clone());
        e.insert("probability".into(), a.probability.map(|p| json!(p)).unwrap_or(Value::Null));
        e.insert(
            "confidence".into(),
            a.confidence().map(|c| json!(c)).unwrap_or(Value::Null),
        );
        if let Some(d) = &a.distribution {
            e.insert("distribution".into(), d.clone());
        }
        if let Some(r) = &a.reason {
            e.insert("reason".into(), json!(r));
            e.insert("reason_probability".into(),
                     a.reason_probability.map(|p| json!(p)).unwrap_or(Value::Null));
        }
        if a.kind == Kind::Score {
            if let Some(s) = payload.get("answers").and_then(|x| x.get(&q.id)).and_then(|x| x.get("score")) {
                e.insert("score".into(), s.clone());
            }
        }
        ans.insert(q.id.clone(), Value::Object(e));
    }
    out.insert("answers".into(), Value::Object(ans));
    out.insert("gate".into(), json!({
        "ok": failures.is_empty(),
        "min_prob": args.min_prob.map(|p| json!(p)).unwrap_or(Value::Null),
        "failures": failures,
    }));
    let usage = payload.get("usage").cloned().unwrap_or(json!({}));
    out.insert("usage".into(), json!({
        "input_tokens": usage.get("input_tokens").cloned().unwrap_or(Value::Null),
        "output_tokens": usage.get("output_tokens").cloned().unwrap_or(Value::Null),
        "latency_ms": elapsed_ms,
    }));
    Value::Object(out)
}

fn bare_value(answers: &[Answer]) -> String {
    match &answers.first().map(|a| &a.value) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn run(args: Args) -> i32 {
    let questions = match questions_from_args(&args) {
        Ok(q) => q,
        Err(e) => {
            eprintln!("clank-jev: {e}");
            return USAGE;
        }
    };
    let mut state = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut state) {
        eprintln!("clank-jev: reading stdin: {e}");
        return USAGE;
    }
    if state.trim().is_empty() {
        eprintln!("clank-jev: empty state — pipe the text to decide about");
        return USAGE;
    }
    let (payload, provider, model, elapsed_ms) = match decide(&args, &questions, &state) {
        Ok(v) => v,
        Err((msg, code)) => {
            eprintln!("clank-jev: {msg}");
            return code;
        }
    };
    let answers = match read_answers(&questions, &payload) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("clank-jev: {e}");
            return PROVIDER;
        }
    };
    let failures = gate_failures(&args, &questions, &answers);

    if args.json || questions.len() > 1 {
        let out = render(&args, &questions, &answers, &payload, provider, &model, elapsed_ms, &failures);
        println!("{}", serde_json::to_string(&out).unwrap_or_default());
    } else if args.print_reason {
        println!("{}", answers.first().and_then(|a| a.reason.clone()).unwrap_or_default());
    } else {
        println!("{}", bare_value(&answers));
    }
    if !args.quiet {
        let p = answers
            .first()
            .and_then(|a| a.probability)
            .map(|p| format!("{p:.3}"))
            .unwrap_or_else(|| "n/a".into());
        eprintln!("clank-jev: decided in {elapsed_ms} ms, p={p} ({})", provider.name());
    }
    for f in &failures {
        eprintln!("clank-jev: {f}");
    }
    if failures.is_empty() {
        OK
    } else {
        GATE_FAILED
    }
}

fn main() {
    let args = Args::parse();
    std::process::exit(run(args));
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn args(argv: &[&str]) -> Args {
        Args::try_parse_from(std::iter::once("clank-jev").chain(argv.iter().copied())).unwrap()
    }

    #[test]
    fn checks_accept_boolean_as_noul() {
        let qs = parse_checks(r#"{"refund":{"type":"boolean","instructions":"Was a refund issued?"}}"#).unwrap();
        assert_eq!(qs[0].kind, Kind::Bool);
        let body = body_for("jev-latest", "state", &qs);
        assert_eq!(body["questions"]["refund"]["type"], "noul");
        assert_eq!(body["state"], "state");
    }

    #[test]
    fn choice_needs_criteria_and_score_needs_levels() {
        assert!(parse_checks(r#"{"x":{"type":"choice","instructions":"?"}}"#).is_err());
        assert!(parse_checks(r#"{"x":{"type":"score","instructions":"?","criteria":["only"]}}"#).is_err());
        assert!(parse_checks(r#"{"x":{"type":"noul"}}"#).is_err(), "instructions are required");
        assert!(parse_checks("{}").is_err());
    }

    #[test]
    fn args_build_one_question_in_each_shape() {
        let qs = questions_from_args(&args(&["--ask", "Which team?", "--choice", "billing, tech ,account"])).unwrap();
        assert_eq!(qs.len(), 1);
        assert_eq!(qs[0].kind, Kind::Choice);
        assert_eq!(qs[0].criteria.as_object().unwrap().len(), 3);
        assert_eq!(qs[0].id, DEFAULT_ID);

        let qs = questions_from_args(&args(&["--ask", "Blocked?", "--boolean"])).unwrap();
        assert_eq!(qs[0].kind, Kind::Bool);
        assert!(qs[0].criteria.is_null());

        let qs = questions_from_args(&args(&["--ask", "How bad?", "--score", "low,mid,high"])).unwrap();
        assert_eq!(qs[0].criteria.as_array().unwrap().len(), 3);
    }

    #[test]
    fn missing_shape_or_ask_is_usage() {
        assert!(questions_from_args(&args(&["--ask", "?"])).is_err());
        assert!(questions_from_args(&args(&["--choice", "a,b"])).is_err());
        assert!(questions_from_args(&args(&["--ask", "?", "--choice", "solo"])).is_err());
    }

    #[test]
    fn reads_a_typesafe_noul_and_a_fractional_score() {
        let a = read_answer(&Kind::Bool, &json!({"type":"noul","noul":0.99})).unwrap();
        assert_eq!(a.value, json!(true));
        assert_eq!(a.probability, Some(0.99));

        // TypeSafe's score is a probability-weighted position; the level is what
        // a script routes on, and the raw value is kept alongside it.
        let a = read_answer(&Kind::Score, &json!({"type":"score","score":1.78,"probabilities":{"0":0.0,"1":0.22,"2":0.78}})).unwrap();
        assert_eq!(a.value, json!(2));
        assert_eq!(a.probability, Some(0.78));
    }

    #[test]
    fn reads_a_choice_as_a_string_or_a_distribution() {
        let a = read_answer(&Kind::Choice, &json!({"type":"choice","choice":"warm","probabilities":{"warm":0.98,"curt":0.02}})).unwrap();
        assert_eq!(a.value, json!("warm"));
        assert_eq!(a.probability, Some(0.98));

        let a = read_answer(&Kind::Choice, &json!({"type":"choice","choice":{"billing":0.9,"technical":0.1}})).unwrap();
        assert_eq!(a.value, json!("billing"));
        assert_eq!(a.probability, Some(0.9));
    }

    #[test]
    fn gates_report_every_reason_the_decision_must_not_be_trusted() {
        let q = Question { id: DEFAULT_ID.into(), kind: Kind::Choice, instructions: "?".into(), criteria: json!({}), reasons: None };
        let low = Answer { kind: Kind::Choice, value: json!("code"), probability: Some(0.4), distribution: None, reason: None, reason_probability: None, provider_confidence: None };

        let failures = gate_failures(&args(&["--min-prob", "0.7"]), std::slice::from_ref(&q), std::slice::from_ref(&low));
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("below --min-prob"));

        let failures = gate_failures(&args(&["--expect", "prose"]), std::slice::from_ref(&q), std::slice::from_ref(&low));
        assert!(failures[0].contains("expected \"prose\", decided \"code\""));

        let good = Answer { kind: Kind::Choice, value: json!("code"), probability: Some(0.9), distribution: None, reason: None, reason_probability: None, provider_confidence: None };
        assert!(gate_failures(&args(&["--min-prob", "0.7", "--expect", "CODE"]), std::slice::from_ref(&q), std::slice::from_ref(&good)).is_empty());

        let score = Answer { kind: Kind::Score, value: json!(0), probability: Some(0.9), distribution: None, reason: None, reason_probability: None, provider_confidence: None };
        let sq = Question { id: DEFAULT_ID.into(), kind: Kind::Score, instructions: "?".into(), criteria: json!([]), reasons: None };
        let failures = gate_failures(&args(&["--expect-min", "1"]), std::slice::from_ref(&sq), std::slice::from_ref(&score));
        assert!(failures[0].contains("below --expect-min"));
    }

    #[test]
    fn a_confident_no_passes_a_confidence_gate() {
        // The fan-out that found this: asked whether a docs-only commit could break
        // a caller, the model said no at p(true)=0.12 — decisive, and rejected by a
        // gate on the raw probability. The gate is on the decision, not on "yes".
        let q = Question { id: DEFAULT_ID.into(), kind: Kind::Bool, instructions: "?".into(), criteria: Value::Null, reasons: None };
        let no = Answer {
            kind: Kind::Bool,
            value: json!(false),
            probability: Some(0.12),
            distribution: None,
            reason: None,
            reason_probability: None,
            provider_confidence: None,
        };
        assert!(gate_failures(&args(&["--min-prob", "0.7"]), std::slice::from_ref(&q), std::slice::from_ref(&no)).is_empty(),
                "a decisive no is confident, not unclear");

        // ...while a genuinely split answer still fails.
        let torn = Answer { probability: Some(0.55), ..no.clone() };
        let failures = gate_failures(&args(&["--min-prob", "0.7"]), std::slice::from_ref(&q), std::slice::from_ref(&torn));
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("confidence 0.550"), "{failures:?}");
    }

    #[test]
    fn a_provider_confidence_is_used_for_choice_answers() {
        // Jev reports a normalized margin for choice answers; it is a better gate
        // than the winner's share, and it is what --min-prob compares against.
        let a = read_answer(&Kind::Choice, &json!({"type": "choice", "choice": "warm",
            "probabilities": {"warm": 0.6, "curt": 0.4}, "confidence": 0.33})).unwrap();
        assert_eq!(a.probability, Some(0.6));
        assert_eq!(a.confidence(), Some(0.33));
    }

    #[test]
    fn an_answer_without_a_probability_fails_a_probability_gate() {
        let q = Question { id: DEFAULT_ID.into(), kind: Kind::Choice, instructions: "?".into(), criteria: json!({}), reasons: None };
        let a = Answer { kind: Kind::Choice, value: json!("code"), probability: None, distribution: None, reason: None, reason_probability: None, provider_confidence: None };
        let failures = gate_failures(&args(&["--min-prob", "0.5"]), &[q], &[a]);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("no probability"));
    }

    #[test]
    fn a_reason_set_becomes_a_second_question_in_the_same_request() {
        let qs = parse_checks(
            r#"{"claim":{"type":"boolean","instructions":"Did the claim hold?",
                 "reasons":{"no_conflict":"nothing conflicts","verification_contradiction":"a result contradicts it"}}}"#,
        )
        .unwrap();
        let body = body_for("jev-latest", "state", &qs);
        let reason = &body["questions"]["claim.reason"];
        assert_eq!(reason["type"], "choice", "a reason is a closed choice: {reason}");
        assert_eq!(reason["criteria"].as_object().unwrap().len(), 2);
        assert!(reason["instructions"].as_str().unwrap().contains("Did the claim hold?"));

        let payload = json!({"answers": {
            "claim": {"type": "noul", "noul": 0.1},
            "claim.reason": {"type": "choice", "choice": "verification_contradiction",
                             "probabilities": {"verification_contradiction": 0.91, "no_conflict": 0.09}},
        }});
        let answers = read_answers(&qs, &payload).unwrap();
        assert_eq!(answers[0].value, json!(false));
        assert_eq!(answers[0].reason.as_deref(), Some("verification_contradiction"));
        assert_eq!(answers[0].reason_probability, Some(0.91));
    }

    #[test]
    fn a_reason_set_must_be_a_non_empty_object_and_must_be_answered() {
        assert!(parse_checks(r#"{"x":{"type":"boolean","instructions":"?","reasons":{}}}"#).is_err());
        assert!(parse_checks(r#"{"x":{"type":"boolean","instructions":"?","reasons":["a"]}}"#).is_err());

        let qs = parse_checks(r#"{"x":{"type":"boolean","instructions":"?","reasons":{"a":"A","b":"B"}}}"#).unwrap();
        let payload = json!({"answers": {"x": {"type": "noul", "noul": 0.9}}});
        let err = read_answers(&qs, &payload).unwrap_err();
        assert!(err.contains("reason"), "a missing reason is an error, not a default: {err}");
    }

    #[test]
    fn a_provider_that_skips_a_question_is_an_error_not_a_default() {
        let qs = vec![
            Question { id: "a".into(), kind: Kind::Bool, instructions: "?".into(), criteria: Value::Null, reasons: None },
            Question { id: "b".into(), kind: Kind::Bool, instructions: "?".into(), criteria: Value::Null, reasons: None },
        ];
        let payload = json!({"answers": {"a": {"type": "noul", "noul": 0.7}}});
        let err = read_answers(&qs, &payload).unwrap_err();
        assert!(err.contains("\"b\""), "{err}");
    }
}
