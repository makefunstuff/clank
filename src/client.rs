//! Blocking OpenAI-compatible SSE chat client (ureq).
//!
//! Two decorations live here, and nothing else: one field added to the request
//! body (reasoning control), and *routing* of the one stream field that is not
//! the answer (`reasoning_content`). Nothing in this file rewrites, buffers or
//! repairs the model's answer.

use std::io::{BufRead, Read};
use serde_json::{json, Value};
use crate::Fail;

/// One streamed chat round: text deltas plus any tool calls.
pub struct Round {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: Option<String>,
}

pub struct ToolCall {
    pub id: Option<String>,
    pub name: String,
    /// raw JSON string, accumulated from streamed fragments
    pub arguments: String,
}

/// Reasoning control for one request.
///
/// `Off` disables thinking through the chat template; `Effort` asks for a level
/// and lets the template decide whether it can honour it. Which levels a given
/// template accepts is the server's business, not clank's.
pub enum Thinking {
    Off,
    Effort(String),
}

/// Everything one request needs. A struct rather than ten positional arguments.
pub struct Request<'a> {
    pub base_url: &'a str,
    pub model: &'a str,
    pub api_key: Option<&'a str>,
    pub max_tokens: u32,
    pub messages: &'a [Value],
    pub tools: &'a Value,
    pub json_schema: Option<&'a Value>,
    pub thinking: Option<&'a Thinking>,
    pub debug_path: Option<&'a str>,
}

pub fn stream_round(
    agent: &ureq::Agent,
    req: Request<'_>,
    emit: &mut dyn FnMut(&str),
    on_thinking: &mut dyn FnMut(&str),
) -> Result<Round, Fail> {
    let url = format!("{}/chat/completions", req.base_url);
    let mut body = json!({
        "model": req.model,
        "messages": req.messages,
        "tools": req.tools,
        "stream": true,
        "max_tokens": req.max_tokens,
    });
    if let Some(s) = req.json_schema {
        body["json_schema"] = s.clone();
    }
    match req.thinking {
        Some(Thinking::Off) => body["chat_template_kwargs"] = json!({ "enable_thinking": false }),
        Some(Thinking::Effort(level)) => body["reasoning_effort"] = json!(level),
        None => {}
    }
    if let Some(p) = req.debug_path {
        let _ = std::fs::write(p, serde_json::to_string_pretty(&body).unwrap_or_default());
    }

    let mut request = agent.post(&url).header("Content-Type", "application/json");
    if let Some(key) = req.api_key {
        request = request.header("Authorization", format!("Bearer {key}"));
    }
    let mut resp = request
        .send_json(&body)
        .map_err(|e| Fail::Model(format!("request to {url} failed: {e}")))?;

    if resp.status() != 200 {
        let mut detail = String::new();
        let _ = resp.body_mut().as_reader().read_to_string(&mut detail);
        return Err(Fail::Model(format!(
            "model returned HTTP {status}: {detail}",
            status = resp.status()
        )));
    }

    let mut round = Round {
        text: String::new(),
        tool_calls: Vec::new(),
        finish_reason: None,
    };
    let lines = std::io::BufReader::new(resp.body_mut().as_reader()).lines();

    for line in lines {
        let line = line.map_err(|e| Fail::Model(format!("stream read error: {e}")))?;
        let Some(data) = line.strip_prefix("data:") else { continue };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            if data == "[DONE]" {
                break;
            }
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else { continue };

        let Some(choices) = v.get("choices").and_then(|c| c.as_array()) else { continue };
        for ch in choices {
            if let Some(fr) = ch.get("finish_reason").and_then(|f| f.as_str()) {
                round.finish_reason = Some(fr.to_string());
            }
            let Some(delta) = ch.get("delta") else { continue };
            if let Some(t) = delta.get("content").and_then(|c| c.as_str()) {
                round.text.push_str(t);
                emit(t);
            }
            // Thinking is billed for whether or not anyone looks at it; the
            // caller decides whether it is worth showing.
            if let Some(t) = delta.get("reasoning_content").and_then(|c| c.as_str()) {
                on_thinking(t);
            }
            if let Some(tcs) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                for tc in tcs {
                    let Some(idx) = tc.get("index").and_then(|i| i.as_u64()) else { continue };
                    while round.tool_calls.len() <= idx as usize {
                        round.tool_calls.push(ToolCall {
                            id: None,
                            name: String::new(),
                            arguments: String::new(),
                        });
                    }
                    let slot = &mut round.tool_calls[idx as usize];
                    if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                        slot.id = Some(id.to_string());
                    }
                    let fn_ = tc.get("function");
                    if let Some(name) = fn_.and_then(|f| f.get("name")).and_then(|n| n.as_str()) {
                        slot.name = name.to_string();
                    }
                    if let Some(args) = fn_.and_then(|f| f.get("arguments")).and_then(|a| a.as_str()) {
                        slot.arguments.push_str(args);
                    }
                }
            }
        }
    }

    Ok(round)
}
