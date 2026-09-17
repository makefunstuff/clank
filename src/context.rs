//! Context tree: a document-ordered tree of text and file nodes.
//!
//! Piped stdin becomes a single text node; `-c` loads a file that is either a
//! JSON tree or plain text (plain text = one text node).
//!
//! Node shape (JSON object):
//!   { "text": "..." }        raw text
//!   { "file": "path" }       file contents, read at render time (path relative to cwd)
//!   { "children": [ ... ] }  nested subtree
//! An array of nodes is also accepted. Rendering walks the tree in document
//! order; file leaves are emitted as `─── path ───` followed by their contents.

use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize, Clone)]
pub struct Node {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub children: Option<Vec<Node>>,
}

pub enum Leaf {
    Text(String),
    File { path: String, content: String },
}

pub struct Tree {
    pub nodes: Vec<Node>,
}

/// The six event types a clank stream can contain. Anything else — however
/// JSON-shaped, however much it has a `type` field of its own — is data, not a
/// trace. Recognising traces loosely would silently swallow ordinary JSON.
const EVENT_TYPES: [&str; 6] = ["run", "item", "tool_call", "tool_result", "assistant", "error"];

/// A clank JSONL event stream: every non-empty line is a JSON object whose
/// `type` is one of ours.
pub fn is_clank_jsonl(source: &str) -> bool {
    let mut any = false;
    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        any = true;
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            return false;
        };
        // `get` only answers for objects, so a non-object has already returned.
        let Some(kind) = v.get("type").and_then(|t| t.as_str()) else {
            return false;
        };
        if !EVENT_TYPES.contains(&kind) {
            return false;
        }
    }
    any
}

/// Compact transcript rendering of clank JSONL events, so
/// `clank --jsonl | clank -m "..."` is a real continuation:
///   assistant: <text>
///   > read_file path=a
pub fn render_transcript(source: &str) -> String {
    source
        .lines()
        .filter_map(|raw| {
            let line = raw.trim();
            if line.is_empty() {
                return None;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                return None;
            };
            let Some(t) = v.get("type").and_then(|t| t.as_str()) else {
                return None;
            };
            match t {
                "assistant" => v
                    .get("content")
                    .and_then(|c| c.as_str())
                    .map(|c| format!("assistant: {c}")),
                "tool_call" => v
                    .get("name")
                    .and_then(|n| n.as_str())
                    .map(|n| {
                        let args = v.get("arguments").unwrap_or(&serde_json::Value::Null);
                        format!("> {} {}", n, crate::tools::compact_args(args))
                    }),
                "item" => {
                    let i = v.get("i").and_then(|n| n.as_u64()).unwrap_or(0);
                    let of = v.get("of").and_then(|n| n.as_u64()).unwrap_or(0);
                    let input = v.get("input").and_then(|n| n.as_str()).unwrap_or("");
                    Some(format!("─── item {i}/{of} ───\n{input}"))
                }
                "error" => v
                    .get("message")
                    .and_then(|m| m.as_str())
                    .map(|m| format!("error: {m}")),
                "tool_result" => {
                    let name = v.get("name").and_then(|n| n.as_str())?;
                    let ok = v.get("ok").and_then(|o| o.as_bool()).unwrap_or(false);
                    let mut line = format!("< {name} {}", if ok { "ok" } else { "err" });
                    if let Some(out) = v.get("output").and_then(|o| o.as_str()) {
                        if !out.trim().is_empty() {
                            // The trace keeps everything; a transcript is context,
                            // and context has a budget.
                            let (shown, dropped) = truncate_bytes(out, TRANSCRIPT_OUTPUT_CAP);
                            line.push('\n');
                            for l in shown.lines() {
                                line.push_str("    ");
                                line.push_str(l);
                                line.push('\n');
                            }
                            if dropped > 0 {
                                line.push_str(&format!("    … {dropped} B not shown\n"));
                            }
                            line.pop();
                        }
                    }
                    Some(line)
                }
                _ => None,
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// How much of a tool result's output a transcript keeps. The trace itself is
/// complete; a transcript is context, and context has a budget.
const TRANSCRIPT_OUTPUT_CAP: usize = 400;

/// Split at a byte cap without ever cutting a character in half.
fn truncate_bytes(s: &str, cap: usize) -> (&str, usize) {
    if s.len() <= cap {
        return (s, 0);
    }
    let mut end = cap;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (&s[..end], s.len() - end)
}

/// The transcript of a clank trace, if this source is one *and* it renders to
/// something. A trace of provenance only is not a transcript.
fn as_transcript(source: &str) -> Option<String> {
    if !is_clank_jsonl(source) {
        return None;
    }
    let transcript = render_transcript(source);
    (!transcript.trim().is_empty()).then_some(transcript)
}

/// A node holding `text` as one piece of raw evidence.
fn text_node(text: String) -> Node {
    Node {
        text: Some(text),
        file: None,
        children: None,
    }
}

/// Parse what a *pipe* delivered.
///
/// The pipe is evidence, so this cannot fail and cannot drop: a clank trace
/// becomes a transcript, and anything else — including JSON that happens to
/// have a `type` field, or a JSON object that is not a context node — is
/// handed to the model as text. Configuration lives in `-c`, where strictness
/// belongs.
pub fn parse_evidence(source: &str) -> Tree {
    let node = match as_transcript(source) {
        Some(transcript) => text_node(transcript),
        None => text_node(source.to_string()),
    };
    Tree { nodes: vec![node] }
}

/// Parse a context *file*: a clank trace, a JSON tree, or plain text.
///
/// Unlike a pipe, a file is configuration someone wrote on purpose, so a
/// node-shaped mistake is an error rather than a silent fall-back.
pub fn parse(source: &str) -> Result<Tree, crate::Fail> {
    if let Some(transcript) = as_transcript(source) {
        return Ok(Tree {
            nodes: vec![text_node(transcript)],
        });
    }
    let v = match serde_json::from_str::<serde_json::Value>(source) {
        Ok(v) if v.is_object() || v.is_array() => v,
        _ => return Ok(Tree { nodes: vec![text_node(source.to_string())] }),
    };
    let nodes = if v.is_array() {
        let mut nodes = Vec::new();
        for n in v.as_array().unwrap() {
            nodes.push(parse_node(n)?);
        }
        nodes
    } else {
        vec![parse_node(&v)?]
    };
    Ok(Tree { nodes })
}

fn parse_node(v: &serde_json::Value) -> Result<Node, crate::Fail> {
    let n: Node = serde_json::from_value(v.clone())
        .map_err(|e| crate::Fail::Usage(format!("bad context node: {e}")))?;
    if n.text.is_none() && n.file.is_none() && n.children.is_none() {
        return Err(crate::Fail::Usage(
            "context node needs a 'text', 'file', or 'children' key".into(),
        ));
    }
    Ok(n)
}

/// Load a context file (JSON tree or plain text).
pub fn load_file(path: &Path) -> Result<Tree, crate::Fail> {
    let src = std::fs::read_to_string(path)
        .map_err(|e| crate::Fail::IO(format!("cannot read context file {}: {e}", path.display())))?;
    parse(&src)
}

impl Tree {
    /// Flatten to leaves in document order; file nodes are read eagerly.
    pub fn leaves(&self) -> Result<Vec<Leaf>, crate::Fail> {
        let mut out = Vec::new();
        for n in &self.nodes {
            collect(n, &mut out)?;
        }
        Ok(out)
    }

    /// Rendered context block handed to the model.
    pub fn render(&self) -> String {
        match self.leaves() {
            Ok(leaves) => leaves
                .iter()
                .map(|l| match l {
                    Leaf::Text(t) => t.clone(),
                    Leaf::File { path, content } => format!("─── {path} ───\n{content}"),
                })
                .collect::<Vec<_>>()
                .join("\n\n"),
            Err(e) => format!("[context error: {}]", e.msg()),
        }
    }
}

fn collect(n: &Node, out: &mut Vec<Leaf>) -> Result<(), crate::Fail> {
    if let Some(path) = &n.file {
        // file takes precedence over text/children if both are present.
        let content = std::fs::read_to_string(path)
            .map_err(|e| crate::Fail::IO(format!("cannot read context file {}: {e}", path)))?;
        out.push(Leaf::File { path: path.clone(), content });
    } else {
        if let Some(t) = &n.text {
            out.push(Leaf::Text(t.clone()));
        }
        if let Some(children) = &n.children {
            for c in children {
                collect(c, out)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRANSCRIPT: &str = r#"{ "type": "tool_call", "name": "read_file", "arguments": { "path": "a" } }
{ "type": "tool_result", "name": "read_file", "ok": true, "output": "x" }
{ "type": "assistant", "content": "done" }"#;

    #[test]
    fn detects_clank_jsonl() {
        assert!(is_clank_jsonl(TRANSCRIPT));
        assert!(is_clank_jsonl(r#"{ "type": "assistant", "content": "pong" }"#));
    }

    #[test]
    fn plain_text_and_trees_are_not_jsonl() {
        assert!(!is_clank_jsonl("hello\nworld"));
        assert!(!is_clank_jsonl(""));
        assert!(!is_clank_jsonl(r#"[{"text": "a"}, {"file": "b"}]"#));
        assert!(!is_clank_jsonl(r#"{"text": "not an event"}"#));
    }

    #[test]
    fn transcript_renders_tagged_lines() {
        assert_eq!(
            render_transcript(TRANSCRIPT),
            "> read_file path=a\n< read_file ok\n    x\nassistant: done",
            "a transcript carries what the tool returned, indented under the call"
        );
    }

    #[test]
    fn parse_routes_jsonl_to_transcript_node() {
        let tree = parse(TRANSCRIPT).unwrap();
        let rendered = tree.render();
        assert!(rendered.contains("assistant: done"));
        assert!(rendered.contains("> read_file path=a"));
    }

    #[test]
    fn a_transcript_caps_what_a_tool_returned() {
        let long = "x".repeat(1000);
        let source = format!(
            "{{\"type\":\"tool_result\",\"name\":\"read_file\",\"ok\":true,\"output\":\"{long}\"}}"
        );
        let out = render_transcript(&source);
        assert!(out.starts_with("< read_file ok\n    xxx"), "{out}");
        assert!(
            out.contains("… 600 B not shown"),
            "the cap is stated rather than hidden: {out}"
        );
        assert!(out.len() < 600, "the rendering stays bounded: {}", out.len());
    }

    #[test]
    fn the_cap_never_splits_a_character() {
        // 300 two-byte characters = 600 bytes; a naive 400-byte slice would panic.
        let source = format!(
            "{{\"type\":\"tool_result\",\"name\":\"read_file\",\"ok\":true,\"output\":\"{}\"}}",
            "é".repeat(300)
        );
        let out = render_transcript(&source);
        assert!(out.contains('é'));
        assert!(out.contains("not shown"), "{out}");
    }

    #[test]
    fn map_frames_and_errors_survive_re_ingestion() {
        let source = concat!(
            "{\"type\":\"item\",\"i\":2,\"of\":3,\"input\":\"b.txt\"}\n",
            "{\"type\":\"error\",\"i\":2,\"of\":3,\"message\":\"timeout\"}\n"
        );
        assert_eq!(render_transcript(source), "─── item 2/3 ───\nb.txt\nerror: timeout");
    }

    #[test]
    fn ordinary_json_that_merely_has_a_type_field_is_not_a_trace() {
        // A tool definition, a commit record, any typed JSON: data, not events.
        let definition = r#"{"type":"function","function":{"name":"search"}}"#;
        assert!(!is_clank_jsonl(definition));
        let rendered = parse_evidence(definition).render();
        assert!(
            rendered.contains("function"),
            "it must reach the model as text: {rendered}"
        );
    }

    #[test]
    fn a_trace_with_no_answer_still_appears_as_evidence() {
        // Provenance only: no transcript exists, but the pipe delivered bytes,
        // so the model gets them rather than an empty context.
        let provenance = r#"{"type":"run","clank":"0.1.0","model":"stub"}"#;
        assert!(is_clank_jsonl(provenance));
        let tree = parse_evidence(provenance);
        let rendered = tree.render();
        assert!(rendered.contains("\"model\":\"stub\""), "{rendered}");
    }

    #[test]
    fn a_pipe_may_carry_any_json_shape() {
        // A JSON array of objects that are not context nodes is still evidence.
        let array = r#"[{"type":"function","function":{"name":"search"}}]"#;
        let rendered = parse_evidence(array).render();
        assert!(rendered.contains("search"), "{rendered}");
        // An object that is not a node, likewise.
        let object = r#"{"type":"commit","sha":"abc"}"#;
        let rendered = parse_evidence(object).render();
        assert!(rendered.contains("abc"), "{rendered}");
        // A file someone wrote on purpose is still checked.
        assert!(parse(r#"[{"txt":"typo"}]"#).is_err());
    }

    #[test]
    fn an_empty_tool_result_renders_no_indented_block() {
        let source = "{\"type\":\"tool_result\",\"name\":\"stat\",\"ok\":true,\"output\":\"\"}";
        assert_eq!(render_transcript(source), "< stat ok");
    }
}
