# clank — minimal unix-style local-inference harness

## Disclaimer
This code is automatically generated, and text below as well. Though I read and curated it, you may feel annoyed of llm driven prose. So sorry for that if you are. This repo is a tool what I personally use right now, but eventually it's just a kind of "brainfart idea", of how I would like to interface with llms.

## Intro

A small Rust CLI that talks to a local llama-server (OpenAI-compatible
endpoint with tool calling) the unix way: prompt and context come in via
argv/stdin, data goes out on stdout, breadcrumbs and errors on stderr.

![clank in a shell](docs/images/clank-demo.gif)

*An actual session, recorded by `scripts/record-demo.sh`: VHS drives a real
terminal, so the answers, the latency and the exit codes in the recording are the
ones those commands produced. This run was `qwen3.8-27b-uncensored` on
`http://127.0.0.1:58777/v1`. Five beats — the four read-only observers; the pipe
as context; `--each` mapping one prompt over three items; a schema-constrained
answer through `jq`; and last, what failure looks like: a legible reason and
`exit 1`.*

- [CHEATSHEET.md](CHEATSHEET.md) — flags, one-liners, integrations
- [docs/use-cases.md](docs/use-cases.md) — real jobs, with the gate and the price
- [PROTOCOL.md](PROTOCOL.md) — the contract: invariants, request sequence, context doctrine, event set, exit codes
The demo traces in `local/demo-traces/` are the output of a local open-source
model served by llama.cpp, and the harness was built to run against that kind of
endpoint; the commits are the record of what changed, when. (An earlier version
of this line claimed every artifact in the repo was written by the local model —
that stopped being true and is corrected here.)

```sh
cargo build --release   # in clank/
./target/release/clank [OPTIONS] [PROMPT...]
```

## Why this instead of an agent framework

A persistent agent harness is a large program you delegate to and will never
read. The problem is not that its code is bad — it is that you cannot audit what
you did not write, so its tool surface is trusted by default. When that surface
includes writing files and running commands with your permissions, the trust
being extended is the whole machine.

Here the surface is text in on stdin, text out on stdout, diagnostics on stderr,
and a verdict in the exit code. No daemon, no session store, no memory you did
not hand over, no tool schemas in the context. Everything an "agent" would do
internally is a pipeline step you can read: `rg`, `git diff`, `jq`, `sed`, and
`xargs -P` when it has to fan out.

What that buys:

- **You can see the input.** The context is exactly what you piped — nothing else
  reached the model, and nothing else reached you.
- **You can see the cost.** One call is one call. No loop re-sending a growing
  history, no second request you did not ask for.
- **You can read the whole thing.** The stage is one binary; the workflow is a
  script you wrote. Both fit in your head at once.
- **The write stays yours.** clank observes and proposes; a gate someone can read
  decides, and the human or the shell applies. That boundary is the security
  property, not a missing feature.

Put it in a container with the tree mounted read-only and the boundary stops
being a promise clank keeps and becomes one the kernel keeps — which bounds what
the model can *write*, not what it can *reach*.

What it does not give you: memory across sessions, retrieval you did not
construct, or an unattended loop that edits your code. Those are the jobs a
harness is for. This is for the work where the artifact is the interface.

## What it is for

Ask a model about text you already have, and get back something your shell can
act on:

```sh
rg -n -C3 "userData" src/ | clank --thinking off -m "what does this do?"
git diff | clank --thinking off --max-tokens 400 -m "review this diff, one line per issue"
rg -l "TODO" src/ | clank --each --thinking off -m "one-line summary"        # map over items
clank --json-schema @schema.json -m "extract the findings" | jq -er .        # structured output
clank --jsonl -m "summarize" | clank -m "what did you say?"                  # continue a run
clank --system @prompts/review-sh.md -c script.sh -m "review the script"     # reusable skill
```

[docs/use-cases.md](docs/use-cases.md) is the full set — seven families of job,
each entry with the command, the gate that says whether it worked, and what it
costs.

## Unix contract

- **stdout = data only**: the final assistant text, framed `--each` answers, or
  `--jsonl` events (`run`, `item`, `tool_call`, `tool_result`, `assistant`,
  `error`). Nothing else. A `--jsonl` stream starts with a `run` event naming
  the model, the endpoint, the argv and an id for the effective system prompt
  (API keys redacted), so a trace says what produced it.
- **stderr = diagnostics**: tool breadcrumbs (`> read_file ...`,
  `< read_file ok (N B)`), the model's reasoning with `--show-thinking`, and
  errors. Token accounting stays in the server's log, where it already lives.
- **prompt**: `-m TEXT`, positional text, piped stdin (stdin is the prompt
  only when no other prompt is given), or a single line from a TTY when
  nothing else is given.
- **tools**: with *nothing* piped — no stdin, no `-c`, no `--each` items — the model
  is offered the four read-only observers, because with no evidence to hand over the
  only honest answer about your workspace is one that looked at it. Every lookup is
  logged on stderr. When evidence *was* supplied it is the evidence, written by the
  shell's own tools, so the model gets that and nothing else; `--no-tools` forces it
  blind (and then the prompt forbids citing a file it was never given).
- **context**: piped stdin becomes a context node when a prompt is also given
  — a clank trace rendered as a transcript, otherwise the bytes as text, so any
  JSON shape survives the trip; `-c FILE` (repeatable) loads a saved context —
  JSON tree, plain text, or a clank JSONL transcript, validated strictly because
  a file is configuration. `-c` nodes come first, then the pipe, in document
  order: a pipe that reaches the process is never dropped.
- **items**: `--each` (with `-0` for NUL-separated input) runs the prompt once
  per stdin item and frames one answer each — see the map section below.
- **SIGPIPE restored** (Rust sets SIG_IGN by default), so `clank ... | head`
  dies cleanly.
- **line-buffered, per-delta flush** in text mode.
- **exit codes**: `0` ok; `1` failure — model/server/IO, a truncated answer
  (token budget), an empty answer, or any failed `--each` item; `2` usage.
- **no colour anywhere**: stdout and stderr are plain text, so there is nothing
  for `NO_COLOR` to disable. `--quiet` suppresses stderr breadcrumbs.

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
emitted as `─── path ───` + content. A `-c` file is validated strictly, because a
file is configuration someone wrote on purpose; the pipe is evidence, so its
bytes are handed over as-is — unless they are a clank trace, which is recognised
by its event types and rendered as a transcript.

## Map (`--each`)

`--each` runs one prompt over every item on stdin, serially, one conversation
per item — the items are the context, the prompt must be `-m`/positional:

```sh
rg -l "TODO" src/ | clank --each -m "one-line summary of this file"
find src -name '*.rs' -print0 | clank --each -0 -m "any overflow risk here?"
```

Text mode frames each answer with the item it belongs to, so the output maps
back to the input (`awk '/^─── item /{…}'` splits it):

```
─── item 1/2 ───
src/main.rs: parses args and drives the loop.

─── item 2/2 ───
src/tools.rs: four read-only observers.
```

`--jsonl` adds `i` (1-based) and `of` to every event and emits one `item` event
carrying the input before the work starts, so a consumer never has to guess:

```sh
# failures only, one line each
clank --each --jsonl -m "rate the risk 1-5" < files.txt | jq -c 'select(.type=="error")'

# item -> answer, joined on the shared index (jq -s waits for the stream)
clank --each --jsonl -m "rate the risk 1-5" < files.txt \
  | jq -s 'map(select(.type=="item" or .type=="assistant" or .type=="error"))
           | group_by(.i)
           | map({input: .[0].input, answer: (.[1].content // null),
                  error: (.[1].message // null)})'
```

A failed item is an `error` event and a stderr line, and does **not** stop the
run — fifty items should not be thrown away because the third one timed out.
The exit code is `1` if any item failed. An empty item list is not a failure:
stdout stays empty, exit code stays `0`.

Everything except the trailing item block is byte-identical from item to item,
which is what lets the server reuse the prompt prefix (see PROTOCOL.md). There
is no parallelism inside clank: `-P` belongs to `xargs`.


## Keeping it fast

The model is the constraint — clank's own share is 1 ms of startup and
milliseconds per megabyte — so the lever is the tokens you pay for:

| rule | effect (measured) |
|---|---|
| `--thinking off` for mechanical work | 1.35 s → 0.25 s |
| keep the prompt head stable, vary the tail | 10.4 s → 0.21 s (cached prefix) |
| stop reading when you have enough (`clank … \| head -1`) | 7.1 s → 0.6 s |
| one call holding many items beats many small calls | 1.5× on four items |

Per call: ≈0.25 s fixed, ≈0.02 s per output token, prompt tokens at 1.3 ms cold
and 0.02 ms cached. `docs/research-harness-constraints.md` §5.2 has the raw
numbers, §5.3 the comparison against headless pi.

## Tools (`--tools`, opt-in)

By default the model sees **only the context you piped** — the stage boundary is
the input you can see in the pipeline. `--tools` offers it four read-only
filesystem observers for the lookup the pipe did not cover — no shell, no
editing:

| tool | description |
|---|---|
| `read_file(path, start_line?, end_line?)` | numbered file content |
| `list_dir(path)` | directory entries (name, type, size) |
| `search(pattern, path?, ignore_case?, context_lines?, glob?)` | regex search, `file:line: text`; context lines in the same format, blank line between groups |
| `stat(path)` | file/directory metadata |

**A tool call is the wrong way to feed a stage.** It is a worse `rg` with an
input nobody can see in the pipeline; the composable form of the same lookup is
a pipe — `rg -n "pattern" | clank -m "…"` — and the composable form of looking
twice is a second stage (`clank --jsonl | clank -m "continue"`). Reach for
`--tools` when one lookup inside one stage is genuinely cheaper than a second
stage, not by default. See PROTOCOL.md.

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

- **Composition is the design.** The shell does the searching, the fan-out,
  the checking and the iterating; clank is the model-shaped stage inside it.
  The default is one prompt, one request, one answer, and the context is
  exactly what you piped — a stage whose input you cannot see in the pipeline
  is not a stage you can trust.
- **Stateless per invocation**: context goes in (pipe or file), data comes
  out (stdout). No session files, no state on disk.
- **Tools are data, not code paths**: each tool is one JSON schema entry plus
  one `fn(&Value) -> String` dispatch; `--jsonl` events are the module
  boundary, so `clank --jsonl | jq` and `clank --jsonl | clank` compose.
- **The capability boundary is the design**: no shell, no editing, no
  deletion. Anything beyond read-only observation composes outside the pipe.
- **A schema is never sent with tools** (this server build rejects the pair,
  and an intermediate round is not an answer): with `--tools`, the rounds run
  unconstrained and then one final request carries the schema and no tools.
- **`--each` is serial and framed** so stdout maps back to stdin; parallel
  fan-out is `xargs -P`, above clank. PROTOCOL.md has the full doctrine and the
  admission rules.

## Configuration

Flags override `$CLANK_*` environment variables, which override built-in
defaults:

| flag | env | default |
|---|---|---|
| `-m` / `--message` / positional | — | — |
| `-c` / `--context` | — | — |
| `--each` | — | false |
| `-0` / `--null` | — | false |
| `--tools` | — | on only when nothing was piped |
| `--no-tools` | — | false |
| `--thinking` | — | server default |
| `--show-thinking` | — | false |
| `--model` | `CLANK_MODEL` | `qwen3.8-27b-gsq-rco-iq3xxs` |
| `--base-url` | `CLANK_BASE_URL` | `http://127.0.0.1:40583/v1` |
| `--api-key` | `CLANK_API_KEY` | none |
| `--timeout` | `CLANK_TIMEOUT` | 600 s per request |
| `--max-tokens` | — | 8192 |
| `--json-schema` | — | none |
| `--system` | `CLANK_SYSTEM` | — |
| `--list-tools` | — | — |
| `--max-rounds` | — | 12 tool rounds per prompt (with `--tools`) |
| `--jsonl`, `-q`/`--quiet`, `-h`/`--help` | — | — |

## Demo

`demo.sh` is the reference pipeline: every stage is a pipe and the shell
gathers the evidence, so no stage asks the model to read the filesystem for
itself. Generate a bash script that renders `fixtures/notes.md` as HTML (the
fixture is piped in) → critique it against the fixture and the shell's own
measurements (piped in) → finalize it (`--json-schema`, one request, grammar
enforced by the server; the candidate and stage 2's trace are the context) →
one-liner (the model proposes a command that runs and verifies the script;
`demo.sh` only syntax-checks it). One `--jsonl` trace per stage lands in
`local/demo-traces/`.

Every stage is **gated**: a non-zero exit, an answer that is not the expected
JSON, or a script that fails `bash -n` stops the demo with a non-zero exit code.
A demo that prints "passed" after producing a broken artifact is worse than no
demo — that is not a hypothetical, it is what the first version of this script
did. All stages run with `--thinking off`: the schema'd ones would otherwise
spend the budget thinking inside the grammar.

The image at the top of this file is a second, shorter take on the same idea:
`scripts/demo.tape` (a VHS tape) holds five beats, each one shell command with
its own exit-status gate, and `scripts/record-demo.sh` resolves the binary, the
endpoint and the model, warms the model, records the tape against a live server,
and fails if the GIF it produced is missing or empty. Re-recording is one
command; nothing in the GIF is drawn or replayed from a canned transcript.

## In a container

One binary, a base image, and whatever you mount. `Dockerfile` builds it; the
image is 32 MB and holds nothing else.

```sh
docker build -t clank .

git log -1 --stat | docker run --rm -i --network=host \
  -v "$PWD":/w:ro -w /w \
  -e CLANK_BASE_URL=http://127.0.0.1:4000/v1 \
  -e CLANK_MODEL=deepseek/deepseek-v4-flash \
  clank --thinking off -m "The context is git log --stat. One line: what changed and the risk it carries."
```

*Verified* 2026-09-19 against the DeepSeek gateway, the repo mounted read-only:

> `docs/use-cases.md` gained §8 (87 lines) and `PROTOCOL.md` one line,
> documenting (not installing) a timer-driven use case; the risk is that the
> units are unverified in-place — only hand-tested once, where a truncated run
> left a `run` event with no `assistant`.

With nothing piped, the read-only observers look around inside the container, and
every lookup still lands on stderr:

```
> search context_lines=0 glob=* ignore_case=true path=. pattern=exit.?code|EXIT_|exit\(2\)|return 2
< search ok (3019 B)
> list_dir path=.
< list_dir ok (326 B)
```

*Verified:* the same question answered `` `src/main.rs:10` defines them — "exit
codes: 0 ok, 1 failure …, 2 usage" ``.

Four things that are not optional:

- **`-i`** is what lets the pipe reach it. No `-i`, no context.
- **`--network=host`** is what makes a model on localhost reachable — and it is
  also the limit of the sandbox. The container bounds what the model can *write*,
  not what it can *reach*; PROTOCOL.md says the same thing about `--tools`.
- **`:ro`** is not decoration. Mounted read-only, `touch /w/pwned` returns
  `Read-only file system`. The capability boundary stops being a promise clank
  keeps and becomes one the kernel keeps.
- **Not alpine.** clank is glibc-dynamic — `exec /usr/local/bin/clank: no such
  file or directory` under musl. `ubuntu:24.04` and `debian:stable-slim` both work.

What this covers is the reading and the proposing: the search, the lookups, the
diff, the answer — with no daemon, no session store, no memory, and no tool
schemas in the context. What it does not cover is the write. clank proposes and
something else applies, which is the containment rather than a gap in it.

## Verified

* Through a remote gateway (`http://127.0.0.1:4000/v1`, `deepseek/deepseek-v4-flash`,
  2026-09-19) a top-level `json_schema` was **not** grammar-enforced: asked for
  `{"script": …}` the model returned a fenced block whose key was `command`, and
  clank exited 1 with *final output is not valid JSON; --json-schema requested*.
  The contract held — a violation is a failure, not a result — but the grammar
  guarantee is the endpoint's to keep, and this one did not. Assume `--json-schema`
  is enforced only where it has been measured.
* Models: qwen 3.8 27b (Qwen3.8-27B GSQ-RCO IQ3_XXS) on
  `http://127.0.0.1:40583/v1` and qwen 3.6 35B-A3B (Qwen3.6-35B-A3B-UD-Q5_K_S)
  on `http://127.0.0.1:37313/v1` — both via the OpenAI-compatible endpoint,
  SSE streaming, tool calling.
* `clank -m "reply with exactly: pong"` → `pong` on stdout, exit 0.
* `rg "userData" fixtures | clank -m "what does this code do?"` → the answer
  on stdout, exit 0 — verified on both models (before the default changed to
  piped-context-only; the same invocation is now a single request).
* Tree context: `clank -c fixtures/ctx.json -m "..."` renders text, file,
  and nested nodes in document order, exit 0.
* `--jsonl` emits events parseable by `jq`, exit 0.
* `--json-schema` forces the final answer to parse as JSON
  (`{"ping":"pong"}`, `jq`-parseable, exit 0); a bad schema exits 2, a
  non-JSON final answer exits 1. It is **one request** on its own; with
  `--tools` the tool rounds run unconstrained first and then one final request
  carries the schema and no tools. The server build rejects tools +
  `json_schema` in one request (400, verified by curl probe), so clank never
  sends them together.
* Observed 2026-09-17, and the reason the tools default as they do: with nothing
  piped and no tools, `clank -m "which file defines the transcript rendering, and what
  is the output cap? cite file:line"` answered *"`src/renderer.ts`, cap 8000 at
  `src/renderer.ts:14`"* — a file that does not exist, in the citation format of a
  real answer, exit 0. The same question with the tools offered answered
  `src/context.rs:68`, cap `400` at `src/context.rs:136`, with the test that asserts it.
* Offline, no model (`cargo test`): `tests/wire.rs`
  drives the real binary against a stub SSE server that records every request
  body, and asserts what this README and PROTOCOL.md claim — **the default is
  one request, with no tools and a system prompt that does not advertise any**;
  a schema and tools never share a request; the schema'd round carries the tool
  results it read; the piped evidence is still context when `-c` is used;
  `--each` frames one answer per item (byte-exact), shares the prompt prefix,
  and tags events with `i`/`of`; a failed item is an `error` event plus exit 1
  while the other items are still answered; an empty item list exits 0;
  `--each` without a prompt exits 2; a **truncated** answer and an **empty**
  answer each exit 1 rather than reporting success; `--thinking off` arrives as
  `chat_template_kwargs.enable_thinking=false` and a level as `reasoning_effort`
  (with no field at all by default, and exit 2 for an unknown level); reasoning
  reaches stderr only under `--show-thinking` while stdout stays the answer
  alone; the `--jsonl` stream opens
  with a `run` event whose argv has any `--api-key` value redacted; a trace with
  a `run` header still reads back as context.
* Live, against both local endpoints (probe + `clank` itself, 2026-09-17):
  a top-level `json_schema` **is** enforced — instructed to reply `beta` under a
  schema whose only legal value is `alpha`, clank printed `{"word":"alpha"}`;
  `--thinking off` removes reasoning on both endpoints, while an effort level is
  honoured by one endpoint and silently ignored by the other; reasoning reaches
  stderr with `--show-thinking` and stdout stays the answer alone. Server-side
  prompt-cache reuse was measured while probing (a repeat request cached 6318 of
  6322 prompt tokens, cutting prompt processing to 129 ms) — clank does not
  report it; it belongs to the server's log.
* `demo.sh` ran live end to end on 2026-09-17 against `:37313`: four stages,
  61 s, all gates passed (`local/demo-traces/` refreshed). Its first run found a
  real defect — an ungated stage wrote a broken candidate and the script still
  announced success — which is why every stage now exits non-zero on failure.
* `--tools` and the schema-after-tools sequence are still unexercised against a
  live model; the wire tests cover the request and output contract only.
* `clank -m "..." | head -1` → exit 0 (SIGPIPE restored, no Rust panic).
* No prompt with empty stdin → exit 2; unreadable context file → exit 1; `--help` → exit 0.
* Transcript continuation: `clank --jsonl | clank` and `clank -c trace.jsonl`
  render the prior run as a tagged transcript (`assistant:`, `> tool`,
  `< result`); a live model call answered `pong` when asked what the
  assistant said last.
* `--system` appends the directive to the system prompt (verified live:
  the model answered exactly `BYE`).
* Repeatable `-c` concatenates files in order (verified via the
  `CLANK_DEBUG` request dump: both files appear in document order).
* `--list-tools` prints the four tool definitions, `jq`-parseable, no model
  call; TTY with no prompt reads one line (code path; not exercisable
  headless).
