# clank — minimal unix-style local-inference harness

A small Rust CLI that talks to a local llama-server (OpenAI-compatible
endpoint with tool calling) the unix way: prompt and context come in via
argv/stdin, data goes out on stdout, breadcrumbs and errors on stderr.

```sh
cargo build --release   # in clank/
./target/release/clank [OPTIONS] [PROMPT...]
```

## Usage

```sh
# plain prompt
clank -m "reply with exactly: pong"

# pipe context in, ask a question (the pipe is the context)
rg "userData" src/ | clank -m "what does this code do?"

# context from a saved tree file (JSON, paths relative to cwd)
clank -c ctx.json -m "summarize the context"

# JSONL events on stdout for jq
echo "list the fixtures dir" | clank --jsonl | jq -c
# schema-constrained JSON output (use --no-tools with this server build)
clank -m 'Reply with JSON: {"ping":"pong"}' \
  --json-schema '{"type":"object","properties":{"ping":{"type":"string"}},"required":["ping"]}' \
  --no-tools | jq
```

## Unix contract

- **stdout = data only**: final assistant text, or `--jsonl` events
  (`tool_call`, `tool_result`, `assistant`). Nothing else.
- **stderr = diagnostics**: tool breadcrumbs (`> read_file ...`,
  `< read_file ok (N B)`) and errors.
- **prompt**: `-m TEXT`, positional text, or piped stdin (stdin is the
  prompt only when no other prompt is given).
- **context**: piped stdin becomes a raw-text context node when a prompt is
  also given; `-c FILE` loads a saved context (JSON tree or plain text) and
  takes precedence over stdin.
- **SIGPIPE restored** (Rust sets SIG_IGN by default), so `clank ... | head`
  dies cleanly.
- **line-buffered, per-delta flush** in text mode.
- **exit codes**: `0` ok, `1` failure (model/server/IO), `2` usage error.
- **NO_COLOR** honored; `--quiet` suppresses stderr breadcrumbs.

## Context tree

A context file is either plain text (one text node) or a JSON tree:

```json
[
  { "text": "raw text node" },
  { "file": "path/relative/to/cwd" },
  { "children": [ { "file": "a.md" }, { "text": "more" } ] }
]
```

Nodes render in document order; file leaves are read at render time and
emitted as `─── path ───` + content. Piped stdin is always a single raw-text
node, so `rg "x" | clank -m "..."` composes like any unix pipeline.

## Tools (deliberately read-only)

The model can call four filesystem observers and nothing else — no shell,
no editing:

| tool | description |
|---|---|
| `read_file(path, start_line?, end_line?)` | numbered file content |
| `list_dir(path)` | directory entries (name, type, size) |
| `search(pattern, path?, ignore_case?, context_lines?, glob?)` | regex search, `file:line: text`; context lines in the same format, blank line between groups |
| `stat(path)` | file/directory metadata |

Tool errors (missing path, bad args) are returned to the model as data, not
crashes; a bad context file or a server error is a process failure (exit 1).

**`search` is deliberately weaker than ripgrep/grep.** It is line-based,
single-line Rust-regex matching (no multi-line patterns), capped at 500
matches and 4 MB per file, and it skips binary files, symlinks, and `.git`
directories. Flags: `ignore_case`, `context_lines` (0–10), `glob` (filename
filter). For heavy or multi-line searches, compose with the real tool
instead of rebuilding it — `rg "pattern" -g '*.rs' | clank -m "what does
this do?"` — or point `search` at a narrow directory.

## Design notes

- **Stateless per invocation**: context goes in (pipe or file), data comes
  out (stdout). No session files, no state on disk.
- **Tools are data, not code paths**: each tool is one JSON schema entry plus
  one `fn(&Value) -> String` dispatch; `--jsonl` events are the module
  boundary, so `clank --jsonl | jq` and `clank --jsonl | clank` compose.
- **The capability boundary is the design**: no shell, no editing, no
  deletion. Anything beyond read-only observation composes outside the pipe.

## Configuration

Flags override `$CLANK_*` environment variables, which override built-in
defaults:

| flag | env | default |
|---|---|---|
| `-m` / positional | — | — |
| `-c` / `--context` | — | — |
| `--model` | `CLANK_MODEL` | `qwen3.8-27b-gsq-rco-iq3xxs` |
| `--base-url` | `CLANK_BASE_URL` | `http://127.0.0.1:40583/v1` |
| `--api-key` | `CLANK_API_KEY` | none |
| `--timeout` | `CLANK_TIMEOUT` | 600 s per call |
| `--max-tokens` | — | 8192 |
| `--json-schema` | — | none |
| `--no-tools` | — | false |
| `--max-rounds` | — | 12 tool rounds |
| `--jsonl`, `-q`/`--quiet`, `-h`/`--help` | — | — |

## Demo

`demo.sh` runs a four-stage chain against a local model (default: the MoE
task model, overridable via `CLANK_MODEL` / `CLANK_BASE_URL`): generate a
bash script that renders `fixtures/notes.md` as HTML → critique it (the
model verifies its claims with the read-only tools) → finalize it
(`--json-schema` emits the corrected script as JSON) → one-liner (the model
proposes a single bash one-liner that runs and verifies the script;
`demo.sh` executes it). One `--jsonl` trace per stage lands in
`local/demo-traces/`.

## Verified

* Models: qwen 3.8 27b (Qwen3.8-27B GSQ-RCO IQ3_XXS) on
  `http://127.0.0.1:40583/v1` and qwen 3.6 35B-A3B (Qwen3.6-35B-A3B-UD-Q5_K_S)
  on `http://127.0.0.1:37313/v1` — both via the OpenAI-compatible endpoint,
  SSE streaming, tool calling.
* `clank -m "reply with exactly: pong"` → `pong` on stdout, exit 0.
* `rg "userData" fixtures | clank -m "what does this code do?"` → full tool
  loop (3× `read_file`, breadcrumbs on stderr), answer on stdout, exit 0 —
  verified on both models.
* Tree context: `clank -c fixtures/ctx.json -m "..."` renders text, file,
  and nested nodes in document order, exit 0.
* `--jsonl` emits `tool_call` / `tool_result` / `assistant` events parseable
  by `jq`, exit 0.
* `--json-schema` + `--no-tools` forces the final answer to schema-valid
  JSON (`{"ping":"pong"}`, `jq`-parseable, exit 0); a bad schema exits 2.
  This server build rejects tools + `json_schema` together (400, verified
  by curl probe), so use `--no-tools` whenever `--json-schema` is set.
* `clank -m "..." | head -1` → exit 0 (SIGPIPE restored, no Rust panic).
* No prompt with empty stdin → exit 2; unreadable context file → exit 1;
  `--help` → exit 0.
