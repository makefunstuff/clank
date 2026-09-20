# clank — minimal unix-style inference harness

A Rust CLI that is one stage in a pipeline: prompt and context in on argv/stdin,
the answer on stdout, diagnostics on stderr, a verdict in the exit code. It
speaks to any OpenAI-compatible endpoint; built against llama.cpp's server,
since verified against a remote gateway and an MLX one.

```sh
cargo build --release
./target/release/clank [OPTIONS] [PROMPT...]
```

![clank in a shell](docs/images/clank-demo.gif)

Recorded against a live server by `scripts/record-demo.sh`: the four read-only
observers, the pipe as context, `--each` over three items, a schema-constrained
answer through `jq`, and a failure with its reason and `exit 1`.

- [CHEATSHEET.md](CHEATSHEET.md) — flags, one-liners, integrations
- [PROTOCOL.md](PROTOCOL.md) — the contract: invariants, request sequence,
  context doctrine, event set, exit codes
- [docs/use-cases.md](docs/use-cases.md) — the job families, each with its gate
  and its price
- [docs/clank-jev.md](docs/clank-jev.md) — typed decisions for routing:
  providers, gates, six composed examples
- [docs/macbook-omlx-local-inference.md](docs/macbook-omlx-local-inference.md) —
  local models on a 16 GB Mac: which model per job, hallucination probes, what
  does not work
- [docs/history/](docs/history/README.md) — closed research records, cited by
  the docs above
- [.jev/README.md](.jev/README.md) — the repository's own design invariants, as
  rules Jev enforces in an editor
- [STATUS.md](STATUS.md) — where the project is, what is open, how to check it
- [.omp/rules/](.omp/rules/) — the same conventions as stream rules, which steer
  a model *while* it writes

## Install

Not on crates.io: the crate named `clank` there is an unrelated project. Two
binaries, `clank` and its decision-stage sibling `clank-jev`, six direct
dependencies, and nothing system-provided beyond a C compiler (`ring`, for TLS;
no OpenSSL to find).

Prebuilt archives are attached to each
[release](https://github.com/makefunstuff/clank/releases) — linux x86_64, macOS
arm64 and macOS x86_64 — each holding both binaries, this README, the LICENSE,
the cheatsheet and the protocol, with a `SHA256SUMS`-style `.sha256` beside it:

```sh
gh release download --pattern '*x86_64-unknown-linux-gnu.tar.gz*'
sha256sum -c clank-*.tar.gz.sha256      # macOS: shasum -a 256 -c
tar xzf clank-*.tar.gz
```

From source, which is what you want if you are wiring it into something else:

```sh
cargo install --locked --git https://github.com/makefunstuff/clank   # -> ~/.cargo/bin/clank
```

`--locked` holds the dependency graph to the committed `Cargo.lock`;
`--rev <sha>` pins clank itself. That directory is on `PATH` if you have
installed anything with cargo before. From a checkout instead — `src/` is 2.8k
lines:

```sh
git clone https://github.com/makefunstuff/clank && cd clank
cargo build --release     # -> ./target/release/clank
cargo test                # no model and no network: a stub SSE server and a stub
                          # JSON one, driven by the real binaries
```

At run time it needs an OpenAI-compatible endpoint that does tool calling.
llama.cpp's `llama-server` is what it was built against, and the only piece here
that wants a GPU:

```sh
export CLANK_BASE_URL=http://127.0.0.1:8080/v1
export CLANK_MODEL=$(curl -s "$CLANK_BASE_URL/models" | jq -r '.data[0].id')
clank -m 'reply with exactly: pong'    # -> pong, exit 0
```

The built-in defaults (`http://127.0.0.1:40583/v1`, `qwen3.8-27b-gsq-rco-iq3xxs`)
are one machine's. Those two variables plus the flags in
[Configuration](#configuration) are the whole configuration: no config file, no
state on disk. A model the server does not serve is the usual first failure, and
it is not silent — the server's reason goes to stderr and the exit code is 1.
Container instead of a toolchain: [In a container](#in-a-container).

## Design

The surface is stdin, stdout, stderr and an exit code. No daemon, no session
store, no memory you did not hand over, so everything a persistent agent harness
would do internally is a pipeline step you can read: `rg`, `git diff`, `jq`,
`sed`, `xargs -P`. The input is visible (the context is exactly the pipe), the
cost is visible (one call is one call, with no loop re-sending a growing
history), and the write stays yours: clank observes and proposes, a gate
decides, you apply. In a container with the tree mounted read-only that boundary
belongs to the kernel rather than to clank's promises, and it bounds what the
model can *write*, not what it can *reach*.

What it does not give you: memory across sessions, retrieval you did not
construct, or a loop that edits your code while you are away.

## What it is for

Ask a model about text you already have, and get back something your shell can
act on:

```sh
rg -n -C3 "userData" src/ | clank --thinking off -m "what does this do?"
git diff | clank --thinking off --max-tokens 400 -m "review this diff, one line per issue"
rg -n "TODO" src/ | clank --each --thinking off -m "one line: actionable now, or not?"
clank --json-schema @schema.json -m "extract the findings" | jq -er .
clank --jsonl -m "summarize" | clank -m "what did you say?"
clank --system @prompts/review-sh.md -c script.sh -m "review the script"
```

## Unix contract

- **stdout is data.** The answer, framed `--each` answers, or `--jsonl` events
  (`run`, `item`, `tool_call`, `tool_result`, `assistant`, `error`), and nothing
  else. A `--jsonl` stream opens with a `run` event naming the model, the
  endpoint and the argv (API keys redacted), so a trace says what produced it.
- **stderr is diagnostics.** Tool breadcrumbs, reasoning under `--show-thinking`,
  errors. Token accounting stays in the server's log, where it already lives.
- **exit codes.** `0` ok; `1` failure — model, server, IO, a truncated answer,
  an empty answer, or any failed `--each` item; `2` usage. Truncation and
  emptiness are failures, not results: a stage that reports a success it cannot
  back is worse than one that fails.
- **prompt** comes from `-m TEXT`, positional text, piped stdin (stdin is the
  prompt only when no other prompt is given), or one line from a TTY.
- **context** is piped stdin, which becomes a context node when a prompt is also
  given: a clank trace renders as a transcript, anything else as text. `-c FILE`
  (repeatable) loads a saved context — JSON tree, text, or a trace. `-c` nodes
  come first, then the pipe; a pipe that reaches the process is never dropped.
- **tools** are offered only when nothing was piped — no stdin, no `-c`, no
  `--each` items — because with no evidence to hand over, the only honest answer
  about your workspace is one that looked at it. Every lookup lands on stderr.
  When evidence was supplied, that is the evidence and nothing else.
  `--no-tools` forces the blind case, and the prompt then forbids citing what it
  never saw.
- **no colour**, plain text on both channels. `--quiet` drops the breadcrumbs.
- **SIGPIPE restored** (Rust sets SIG_IGN by default), so `clank … | head` dies
  cleanly; output is line-buffered and flushed per delta.

## Context tree

A context file is plain text (one text node) or a JSON tree:

```json
[
  { "text": "raw text node" },
  { "file": "path/relative/to/cwd" },
  { "children": [ { "file": "a.md" }, { "text": "more" } ] }
]
```

File leaves are read at render time and emitted as `─── path ───` plus content.
A `-c` file is validated strictly, because a file is configuration someone wrote
on purpose; the pipe parses leniently, because it is evidence.

## Map (`--each`)

One prompt over every item on stdin, serially, one conversation per item. The
items are the context, so the prompt has to come from `-m`/positional.

```sh
git log --format=%s -3 | clank --each -m "one line: rewrite in the imperative mood"
find src -name '*.rs' -print0 | clank --each -0 -m "one line: what is this path for?"
```

Text mode frames each answer with the item it belongs to, so stdout maps back to
stdin (`awk '/^─── item /{…}'` splits it):

```
─── item 1/3 ───
readme: add install section and fix name clash

─── item 2/3 ───
use-cases §9: implement loop and goal in eight lines of shell

─── item 3/3 ───
readme: third shorter and remove essay voice
```

**An item is text, not a file.** The item is the context, so
`rg -l "TODO" src/ | clank --each -m "summarize this file"` hands the model a
filename and nothing else, and it answers accordingly — measured on oMLX
(2026-09-19): *"This file exists but its purpose is not described in the
provided context."* To summarize files, let the shell read them:

```sh
for f in src/*.rs; do
  clank -c "$f" -m "one line: what is this file responsible for?" </dev/null
  echo
done
```

`</dev/null` because clank reads stdin: without it the first call eats the rest
of the list. Content you already gathered maps fine:
`rg -n "TODO" src/ | clank --each -m "one line: actionable now, or not?"`.

`--jsonl` adds `i` (1-based) and `of` to every event and emits one `item` event
carrying the input before the work starts, so a consumer never has to guess. A
failed item is an `error` event and a stderr line, and it does not stop the run;
the exit code is `1` if any item failed, and an empty item list is not a
failure. Everything except the trailing item block is byte-identical from item
to item, which is what lets the server reuse the prompt prefix. There is no
parallelism inside clank: `-P` belongs to `xargs`.

## Keeping it fast

The model is the constraint (clank's own share is 1 ms of startup), so the lever
is the tokens you pay for:

| rule | measured |
|---|---|
| `--thinking off` for mechanical work | 1.35 s → 0.25 s |
| keep the prompt head stable, vary the tail | 10.4 s → 0.21 s (cached prefix) |
| stop reading when you have enough (`clank … \| head -1`) | 7.1 s → 0.6 s |
| one call holding many items beats many small calls | 1.5× on four items |

Per call: ≈0.25 s fixed, ≈0.02 s per output token, prompt tokens at 1.3 ms cold
and 0.02 ms cached.
[docs/history/research-harness-constraints.md](docs/history/research-harness-constraints.md)
§5.2 has the raw numbers and §5.3 the comparison against headless pi.

## Tools (`--tools`, opt-in)

| tool | description |
|---|---|
| `read_file(path, start_line?, end_line?)` | numbered file content |
| `list_dir(path)` | directory entries (name, type, size) |
| `search(pattern, path?, ignore_case?, context_lines?, glob?)` | regex search, `file:line: text` |
| `stat(path)` | file/directory metadata |

A tool call is the wrong way to feed a stage: it is a worse `rg` with an input
nobody can see in the pipeline. The composable form of a lookup is a pipe, and
the composable form of looking twice is a second stage. Reach for `--tools` when
one lookup inside one stage is cheaper than a second stage.

`search` is deliberately weaker than ripgrep: line-based, single-line patterns,
capped at 500 matches and 4 MB per file, skipping binaries, symlinks and `.git`.
Flags: `ignore_case`, `context_lines` (0–10), `glob`. For anything heavier,
compose with the real tool. Tool errors go back to the model as data; a bad
context file or a server error is a process failure.

## Decisions for routing (`clank-jev`)

A second binary in this crate: ask typed questions about a state, get answers a
shell can branch on. `clank` writes prose; `clank-jev` picks one of *your*
options and reports how sure it is, in the exit code as well as the answer.

```sh
route=$(printf '%s' "$task" | clank-jev --ask 'What kind of task is this?' \
          --choice code,prose,math --min-prob 0.7) || route=unclear
case "$route" in
  code) clank --model local-code -c src/context.rs -m "$task" ;;
  *)    clank --model local-fast -m "$task" ;;
esac
```

Providers are `typesafe`, `openrouter` and a local `kev` (no credential, no
network); exit codes `0`/`1`/`2`/`3` separate "decided", "a gate failed",
"usage" and "could not ask". [docs/clank-jev.md](docs/clank-jev.md) has the
provider table, the closed-choice reason sets, the `--min-prob` semantics, and
six composed examples with their observed answers.

| flag | meaning |
|---|---|
| `--ask TEXT` | one question; needs exactly one of the three shapes below |
| `--choice A,B,C` | unordered options, printed by name |
| `--boolean` | yes/no, printed as `true` or `false` |
| `--score low,mid,high` | ordered levels, lowest first; prints the level number |
| `--checks FILE` | a JSON file of questions: `{id: {type, instructions, criteria, reasons?}}` |
| `--min-prob P` | exit 1 unless the decision is at least this confident |
| `--expect VALUE` | exit 1 unless the decision equals this (one question) |
| `--expect-min N` | exit 1 unless an ordered decision is at least this level |
| `--print-reason` | print the closed-choice reason instead of the value |
| `--provider NAME` | `auto`, `typesafe`, `openrouter`, `kev` |
| `--model ID` | defaults per provider: `jev-latest`, `typesafe/jev-1.13`, `kev-latest` |
| `--base-url URL` | move the endpoint (a local `kev`, a proxy, a stub) |
| `--timeout SECS` | per-request timeout, default 60 |
| `--json` | the full result object instead of the bare value |
| `-q` / `--quiet` | no diagnostic line on stderr |

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

`demo.sh` is the reference pipeline. Stage 1 writes a script that renders
`fixtures/notes.md` as HTML; stage 2 critiques it against the fixture and the
shell's own measurements; stage 3 finalizes it under `--json-schema`; stage 4
has the model propose a command that verifies the result. The fixture is piped
at every stage, so no stage asks the model to read the filesystem; every stage
is gated, and a non-zero exit, an answer that is not the expected JSON, or a
script that fails `bash -n` stops the run. One `--jsonl` trace per stage lands
in `local/demo-traces/`. Its first version printed "passed" after producing a
broken artifact — what the gates are for.

## In a container

`Dockerfile` builds a 32 MB image (33,063,772 bytes by `docker image inspect`)
holding both binaries and nothing else.

```sh
docker build -t clank .

git log -1 --stat | docker run --rm -i --network=host \
  -v "$PWD":/w:ro -w /w \
  -e CLANK_BASE_URL=http://127.0.0.1:4000/v1 \
  -e CLANK_MODEL=deepseek/deepseek-v4-flash \
  clank --thinking off -m "The context is git log --stat. One line: what changed and the risk it carries."
```

Verified 2026-09-19 with the repo mounted read-only; it answered:

> `docs/use-cases.md` gained §8 (87 lines) and `PROTOCOL.md` one line, … the risk is
> that the units are unverified in-place — only hand-tested once, where a truncated
> run left a `run` event with no `assistant`.

With nothing piped, the read-only observers look around inside the container,
each lookup on stderr:

```
> search context_lines=0 glob=* ignore_case=true path=. pattern=exit.?code|EXIT_|exit\(2\)|return 2
< search ok (3019 B)
> list_dir path=.
< list_dir ok (326 B)
```

and the answer cited `src/main.rs:10`.

Four details are not optional. `-i` is what lets the pipe reach it.
`--network=host` makes a model on localhost reachable, and is also the limit of
the sandbox. `:ro` is not decoration: mounted read-only, `touch /w/pwned`
returns `Read-only file system`. And not alpine: clank is glibc-dynamic, and
musl has no loader for it; `ubuntu:24.04` and `debian:stable-slim` both work.

## Verified

Each claim was run; the dated record behind it, with the endpoint, model and
quoted output, is in
[docs/history/verification-log.md](docs/history/verification-log.md).

- **Endpoints.** llama.cpp on `:40583` and `:37313`, a gateway on `:4000`, and
  oMLX 0.7.0 on `:8000` serving three MLX models: streaming, tool calling and
  reasoning channels on all of them; a fourth model the runtime refuses with a
  409 that clank prints verbatim.
- **`json_schema`.** Enforced by the server where it has been measured, not
  guaranteed elsewhere: the gateway returned a fenced block and clank exited 1.
  A schema and tools never share a request (400, curl-probed).
- **The tools fallback.** A no-context question invented `src/renderer.ts:14` at
  exit 0; with the observers offered, the same question answered
  `src/context.rs:68` and cap `400`. A model that fabricates still fabricates —
  the exit code and the two channels are what survive.
- **`--thinking`.** `off` sends `chat_template_kwargs.enable_thinking=false`, a
  level sends `reasoning_effort`, an unknown level is exit 2, and reasoning
  reaches stderr only under `--show-thinking`.
- **`cargo test`.** The real binaries against a stub server: one request by
  default, the schema/tools split, the pipe as context, `--each` framing and
  prefix sharing, failed items, truncation and emptiness as exit 1, argv
  redaction, a trace as context. `tests/docs.rs` guards the docs against the
  code; `tests/rules.rs` guards `.jev/rules`.
- **`clank-jev`.** Hosted Jev (both watchdog questions ≥ 0.93, one request) and
  a local `kev-0.6b` (16/18 typed decisions, 17% shuffled control, 83 ms, no
  credentials).
- **`demo.sh`.** Four gated stages end to end: 61 s on `:37313`, 35 s on
  `Qwen3.5-9B-MLX-4bit`, and a different gate firing on each of the other two
  models.
- **Unexercised.** The schema-after-tools sequence against a live model, and the
  TTY-with-no-prompt path.


## Provenance

Code and docs are largely LLM-written and then read and curated. The commits are
the record of what changed, when, and each of the claims under *Verified* was
run.

## License

MIT. See [LICENSE](LICENSE).
