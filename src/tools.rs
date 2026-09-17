//! Read-only filesystem tools exposed to the model.
//!
//! Deliberately no shell, no editing, no deletion: the harness only observes.
//! Output formats are unix-flavored and jq-friendly where they are structured.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// System prompt when no tools are offered (the default): the piped context is
/// the whole world, and telling the model so is what keeps the stage honest.
pub const SYSTEM_PROMPT: &str = "\
You are `clank`, a minimal unix-style inference harness. You talk to the user through stdout.
Your context is what was piped to you; you have no other channel to the world.
Rules:
- Answer from the context. If it does not contain the answer, say so in one line instead of guessing.
- Cite `file:line` when you refer to code.
- Be concise.
";

/// System prompt when `--tools` offers the read-only observers: they are for
/// the lookup the piped context did not cover, never a second source of truth.
pub const SYSTEM_PROMPT_WITH_TOOLS: &str = "\
You are `clank`, a minimal unix-style inference harness. You talk to the user through stdout.
Your context is what was piped to you, and that context is the evidence.
You can look up what it does not cover with these read-only tools (no shell, no editing):
- read_file(path, start_line?, end_line?) — read a text file; numbered lines
- list_dir(path) — list directory entries
- search(pattern, path?) — regex search under a file or directory; rg-style `file:line: text`
- stat(path) — metadata for a path (JSON)
Rules:
- You only observe: you never write, delete, or run commands.
- Prefer the piped context; when it and the filesystem disagree, the context is what you were asked about.
- Cite `file:line` when you refer to code.
- Be concise.
";

pub fn definitions() -> Value {
    json!([{
        "type": "function",
        "function": {
            "name": "read_file",
            "description": "Read a text file. Returns numbered lines; use start_line/end_line (1-based, inclusive) for large files.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path relative to cwd." },
                    "start_line": { "type": "integer", "minimum": 1 },
                    "end_line": { "type": "integer", "minimum": 1 }
                },
                "required": ["path"]
            }
        }
    }, {
        "type": "function",
        "function": {
            "name": "list_dir",
            "description": "List directory entries (name, d/-, size).",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path relative to cwd; '.' for cwd." }
                },
                "required": ["path"]
            }
        }
    }, {
        "type": "function",
        "function": {
            "name": "search",
            "description": "Line-based Rust regex search (single-line matches only). Output: `file:line: text`, context lines in the same format, blank line between groups. Skips binary files, symlinks, and .git directories. Caps: 500 matches, 4 MB per file.",
            "parameters": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Rust regex pattern." },
                    "path": { "type": "string", "description": "File or directory to search; default '.'." },
                    "ignore_case": { "type": "boolean", "description": "Case-insensitive matching; default false." },
                    "context_lines": { "type": "integer", "minimum": 0, "maximum": 10, "description": "Lines of context around each match; default 0." },
                    "glob": { "type": "string", "description": "Filename glob filter (*, ?, [...]); default '*'." }
                },
                "required": ["pattern"]
            }
        }
    }, {
        "type": "function",
        "function": {
            "name": "stat",
            "description": "Stat a path. Returns JSON: path, type, size, mtime (unix seconds), mode.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path relative to cwd." }
                },
                "required": ["path"]
            }
        }
    }])
}

/// Execute a tool. Returns (output, ok). Errors are returned as data, so the
/// model can react to them; only server-level failures kill the process.
pub fn execute(name: &str, args: &Value) -> (String, bool) {
    let res = match name {
        "read_file" => read_file(args),
        "list_dir" => list_dir(args),
        "search" => search(args),
        "stat" => stat(args),
        other => Err(format!("unknown tool: {other}")),
    };
    match res {
        Ok(out) => (out, true),
        Err(e) => (format!("error: {e}"), false),
    }
}

fn need<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

const DEFAULT_READ_LINES: usize = 400;

fn read_file(args: &Value) -> Result<String, String> {
    let path = need(args, "path").ok_or_else(|| "missing required arg: path".to_string())?;
    let text = fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let lines: Vec<&str> = text.split('\n').collect();
    let total = lines.len();
    let start = args.get("start_line").and_then(|v| v.as_u64()).unwrap_or(1).max(1) as usize;
    let end = args
        .get("end_line")
        .and_then(|v| v.as_u64())
        .map(|e| (e as usize).min(total))
        .unwrap_or(total.min(DEFAULT_READ_LINES));
    let end = end.max(start - 1).min(total);
    if start > total {
        return Ok(format!("{path}: file has {total} lines; start_line {start} is out of range"));
    }
    let mut out = format!("{path} ({total} lines, showing {start}..{end})\n");
    for (n, line) in lines.iter().enumerate().take(end).skip(start - 1) {
        out.push_str(&format!("{:4} | {}\n", n + 1, line));
    }
    if end < total {
        out.push_str(&format!("... ({} more lines; use start_line/end_line)\n", total - end));
    }
    Ok(out)
}

fn list_dir(args: &Value) -> Result<String, String> {
    let path = need(args, "path").ok_or_else(|| "missing required arg: path".to_string())?;
    let dir = Path::new(path);
    if !dir.is_dir() {
        return Err(format!("{path}: not a directory"));
    }
    let mut entries: Vec<(String, bool, u64)> = fs::read_dir(dir)
        .map_err(|e| format!("cannot list {path}: {e}"))?
        .filter_map(|e| e.ok())
        .map(|e| {
            let md = e.metadata().ok();
            (
                e.file_name().to_string_lossy().into_owned(),
                md.as_ref().map(|m| m.is_dir()).unwrap_or(false),
                md.map(|m| m.len()).unwrap_or(0),
            )
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = format!("{path}/\n");
    for (name, is_dir, size) in entries {
        out.push_str(&if is_dir {
            format!("  d  {name}/\n")
        } else {
            format!("  -  {name} ({size} B)\n")
        });
    }
    Ok(out)
}

const SEARCH_CAP: usize = 500;
const MAX_FILE_BYTES: u64 = 4_000_000;

fn search(args: &Value) -> Result<String, String> {
    let pattern = need(args, "pattern")
        .ok_or_else(|| "missing required arg: pattern".to_string())?;
    let ignore_case = args.get("ignore_case").and_then(Value::as_bool).unwrap_or(false);
    let context_lines = args
        .get("context_lines")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(10) as usize;
    let glob = need(args, "glob").unwrap_or("*");
    let mut b = regex::RegexBuilder::new(pattern);
    b.case_insensitive(ignore_case);
    let re = b.build().map_err(|e| format!("bad regex {pattern:?}: {e}"))?;
    let g = glob_to_regex(glob).map_err(|e| format!("bad glob {glob:?}: {e}"))?;
    let path = need(args, "path").unwrap_or(".");
    let root = Path::new(path);

    let mut out = String::new();
    let mut count = 0usize;
    let mut truncated = false;
    if root.is_file() {
        walk_one(root, &re, &g, context_lines, &mut out, &mut count, &mut truncated);
    } else if root.is_dir() {
        walk_dir(root, &re, &g, context_lines, &mut out, &mut count, &mut truncated);
    } else {
        return Err(format!("{path}: no such file or directory"));
    }
    if count == 0 {
        out = "no matches\n".to_string();
    }
    if truncated {
        out.push_str("... (truncated at 500 matches)\n");
    }
    Ok(out)
}

/// Simple glob (* ? [...]) to a full-match regex over the file name.
fn glob_to_regex(glob: &str) -> Result<regex::Regex, regex::Error> {
    let chars: Vec<char> = glob.chars().collect();
    let mut re = String::from("^");
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '!' && chars.get(i + 1) == Some(&'[') {
            // negated class: ![abc] -> [^abc]
            let mut j = i + 2;
            while j < chars.len() && chars[j] != ']' {
                j += 1;
            }
            if j >= chars.len() {
                return Err(regex::Error::Syntax("unclosed '[' in glob".to_string()));
            }
            let body: String = chars[i + 2..j].iter().collect();
            re.push_str("[^");
            re.push_str(&body);
            re.push(']');
            i = j + 1;
            continue;
        }
        match chars[i] {
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            '[' => {
                let start = i;
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '!' || chars[j] == '^') {
                    j += 1;
                }
                while j < chars.len() && chars[j] != ']' {
                    j += 1;
                }
                if j >= chars.len() {
                    return Err(regex::Error::Syntax("unclosed '[' in glob".to_string()));
                }
                let mut cls: String = chars[start..=j].iter().collect();
                if cls.get(1..2) == Some("!") {
                    cls.replace_range(1..2, "^");
                }
                re.push_str(&cls);
                i = j + 1;
                continue;
            }
            c => re.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    re.push('$');
    regex::Regex::new(&re)
}

fn is_binary(b: &[u8]) -> bool {
    b.iter().take(8000).any(|&b| b == 0)
}

fn walk_one(
    p: &Path,
    re: &regex::Regex,
    g: &regex::Regex,
    ctx: usize,
    out: &mut String,
    count: &mut usize,
    truncated: &mut bool,
) {
    let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if !g.is_match(name) {
        return;
    }
    let Ok(meta) = fs::metadata(p) else { return };
    if meta.len() > MAX_FILE_BYTES {
        return;
    }
    let Ok(bytes) = fs::read(p) else { return };
    if is_binary(&bytes) {
        return;
    }
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let n = lines.len();
    let mut last_end: usize = 0;
    let mut first = true;
    for (idx, line) in lines.iter().enumerate() {
        if !re.is_match(line) {
            continue;
        }
        *count += 1;
        if *count >= SEARCH_CAP {
            *truncated = true;
            return;
        }
        let lo = idx.saturating_sub(ctx);
        let hi = (idx + ctx).min(n - 1);
        if !first && lo > last_end + 1 {
            out.push('\n');
        }
        for k in lo..=hi {
            out.push_str(&format!("{}:{}: {}\n", p.display(), k + 1, lines[k].trim_end()));
        }
        last_end = hi;
        first = false;
    }
}

fn walk_dir(
    dir: &Path,
    re: &regex::Regex,
    g: &regex::Regex,
    ctx: usize,
    out: &mut String,
    count: &mut usize,
    truncated: &mut bool,
) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut files: Vec<PathBuf> = Vec::new();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for e in entries.filter_map(|e| e.ok()) {
        let p = e.path();
        let ft = e.file_type();
        if ft.as_ref().map(|t| t.is_symlink()).unwrap_or(false) {
            continue;
        }
        if ft.as_ref().map(|t| t.is_dir()).unwrap_or(false) {
            if e.file_name() == std::ffi::OsStr::new(".git") {
                continue;
            }
            dirs.push(p);
        } else if p.is_file() {
            files.push(p);
        }
    }
    files.sort();
    for p in files {
        if *truncated {
            return;
        }
        walk_one(&p, re, g, ctx, out, count, truncated);
    }
    dirs.sort();
    for d in dirs {
        if *truncated {
            return;
        }
        walk_dir(&d, re, g, ctx, out, count, truncated);
    }
}

fn stat(args: &Value) -> Result<String, String> {
    let path = need(args, "path").ok_or_else(|| "missing required arg: path".to_string())?;
    let md = fs::metadata(path).map_err(|e| format!("cannot stat {path}: {e}"))?;
    let kind = if md.is_dir() {
        "dir"
    } else if md.is_symlink() {
        "symlink"
    } else {
        "file"
    };
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut v = json!({
        "path": path,
        "type": kind,
        "size": md.len(),
        "mtime": mtime,
    });
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Some(obj) = v.as_object_mut() {
            // Permission bits only: `mode()` also carries the file type
            // (0o100644), which reads like a permission mask but is not one.
            obj.insert(
                "mode".into(),
                Value::String(format!("{:04o}", md.permissions().mode() & 0o7777)),
            );
        }
    }
    Ok(v.to_string())
}

/// Compact one-line rendering of tool args for stderr breadcrumbs.
pub fn compact_args(v: &Value) -> String {
    match v {
        Value::Object(m) => m
            .iter()
            .map(|(k, val)| format!("{k}={}", short(val)))
            .collect::<Vec<_>>()
            .join(" "),
        other => other.to_string(),
    }
}

fn short(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "-".into(),
        other => format!("…{}chars", other.to_string().len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("clank-test-{}-{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_file(d: &Path, name: &str, content: &str) -> PathBuf {
        let p = d.join(name);
        fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn search_ignore_case() {
        let d = sandbox("case");
        let p = write_file(&d, "a.txt", "UserData\nother\n");
        let out = search(&json!({"pattern": "userdata", "path": p.to_string_lossy().to_string(), "ignore_case": true})).unwrap();
        assert!(out.contains("a.txt:1: UserData"));
        let out = search(&json!({"pattern": "userdata", "path": p.to_string_lossy().to_string()})).unwrap();
        assert_eq!(out, "no matches\n");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn search_context_separates_groups() {
        let d = sandbox("ctx");
        let p = write_file(&d, "b.txt", "a match\nx\ny\nz\nb match\n");
        let path = p.to_string_lossy().to_string();
        let out = search(&json!({"pattern": "match", "path": &path, "context_lines": 1})).unwrap();
        assert_eq!(out, format!("{path}:1: a match\n{path}:2: x\n\n{path}:4: z\n{path}:5: b match\n"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn search_glob_filters_files() {
        let d = sandbox("glob");
        let _a = write_file(&d, "a.txt", "hit\n");
        let _b = write_file(&d, "b.log", "hit\n");
        let out = search(&json!({"pattern": "hit", "path": d.to_string_lossy().to_string(), "glob": "*.txt"})).unwrap();
        assert!(out.contains("a.txt:1: hit"));
        assert!(!out.contains("b.log"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn glob_to_regex_shapes() {
        let g = glob_to_regex("a?.txt").unwrap();
        assert!(g.is_match("a1.txt"));
        assert!(!g.is_match("abc.txt"));
        let g = glob_to_regex("*.txt").unwrap();
        assert!(g.is_match("x.txt"));
        assert!(!g.is_match("x.rs"));
        let g = glob_to_regex("[abc].md").unwrap();
        assert!(g.is_match("c.md"));
        let g = glob_to_regex("![abc].md").unwrap();
        assert!(g.is_match("d.md"));
        assert!(!g.is_match("a.md"));
    }

    #[test]
    fn read_file_shows_the_range_and_what_is_left() {
        let d = sandbox("read");
        let body = (1..=10).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let p = write_file(&d, "ten.txt", &body);
        let path = p.to_string_lossy().to_string();

        let all = read_file(&json!({"path": &path})).unwrap();
        assert!(all.starts_with(&format!("{path} (10 lines, showing 1..10)")), "{all}");
        assert!(all.contains("   1 | line 1"), "{all}");
        assert!(!all.contains("more lines"), "nothing was left out: {all}");

        let window = read_file(&json!({"path": &path, "start_line": 3, "end_line": 4})).unwrap();
        assert!(window.contains("showing 3..4"), "{window}");
        assert!(window.contains("   3 | line 3") && window.contains("   4 | line 4"), "{window}");
        assert!(window.contains("... (6 more lines; use start_line/end_line)"), "{window}");

        let past = read_file(&json!({"path": &path, "start_line": 99})).unwrap();
        assert!(past.contains("out of range"), "{past}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn list_dir_names_what_is_there() {
        let d = sandbox("list");
        let _ = write_file(&d, "a.txt", "x");
        fs::create_dir(d.join("sub")).unwrap();
        let out = list_dir(&json!({"path": d.to_string_lossy().to_string()})).unwrap();
        assert!(out.contains("a.txt"), "{out}");
        assert!(out.contains("sub"), "{out}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn stat_reports_size_and_mode_as_json() {
        let d = sandbox("stat");
        let p = write_file(&d, "s.txt", "hello");
        let out = stat(&json!({"path": p.to_string_lossy().to_string()})).unwrap();
        let v: Value = serde_json::from_str(&out).expect("stat returns JSON");
        assert_eq!(v["size"], 5, "{out}");
        assert!(v["mode"].as_str().unwrap_or("").len() == 4, "{out}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_bad_tool_call_is_data_not_a_panic() {
        // Unknown tool, missing argument, missing file: all model errors.
        let (out, ok) = execute("nope", &json!({}));
        assert!(!ok && out.starts_with("error: unknown tool"), "{out}");
        let (out, ok) = execute("read_file", &json!({}));
        assert!(!ok && out.contains("missing required arg: path"), "{out}");
        let (out, ok) = execute("read_file", &json!({"path": "/definitely/not/here"}));
        assert!(!ok && out.contains("cannot read"), "{out}");
    }
}
