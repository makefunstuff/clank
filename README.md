# clank — minimal unix-style inference harness

A Rust CLI that is one stage in a pipeline: prompt and context in on argv/stdin,
the answer out on stdout, diagnostics on stderr, a verdict in the exit code. It
speaks to any OpenAI-compatible endpoint — built against llama.cpp's server, since
verified against a remote gateway and an MLX one too.

```sh
cargo build --release
./target/release/clank [OPTIONS] [PROMPT...]
```

![clank in a shell](docs/images/clank-demo.gif)

Recorded against a live server by `scripts/record-demo.sh`: the four read-only
observers, the pipe as context, `--each` mapping one prompt over three items, a
schema-constrained answer through `jq`, and what failure looks like — a legible
reason and `exit 1`.

- [CHEATSHEET.md](CHEATSHEET.md) — flags, one-liners, integrations
- [docs/use-cases.md](docs/use-cases.md) — the job families, each with its gate and its price
- [PROTOCOL.md](PROTOCOL.md) — the contract: invariants, request sequence, context doctrine, event set, exit codes
- [docs/macbook-omlx-local-inference.md](docs/macbook-omlx-local-inference.md) — it running against local models on a 16 GB Mac: which model per job, the programming workflow as one-liners, hallucination probes, what to take from Jev to make it less hallucinatory, what does not work

## Install

Not on crates.io — the crate named `clank` there is an unrelated project — and no
prebuilt binaries. Two binaries — `clank` and its decision-stage sibling `clank-jev` —
six direct dependencies, and nothing system-provided
beyond a C compiler (`ring`, for TLS; no OpenSSL to find):

```sh
cargo install --locked --git https://github.com/makefunstuff/clank   # -> ~/.cargo/bin/clank
```

That directory is on `PATH` if you have installed anything with cargo before; if
`clank: command not found` is your first result, it is not. `--locked` holds the
dependency graph to the committed `Cargo.lock`; `--rev <sha>` pins clank itself,
which is what you want when wiring it into something else.

From a checkout instead, if you would rather read it first — `src/` is 2.7k lines:

```sh
git clone https://github.com/makefunstuff/clank && cd clank
cargo build --release     # -> ./target/release/clank
cargo test                # no model and no network: a stub SSE server and a stub
                          # JSON one, driven by the real binaries
```

At run time it needs an OpenAI-compatible endpoint that does tool calling.
llama.cpp's `llama-server` is what it was built against, and the only piece here
that wants a GPU. Point it at one and check the round trip:

```sh
export CLANK_BASE_URL=http://127.0.0.1:8080/v1
export CLANK_MODEL=$(curl -s "$CLANK_BASE_URL/models" | jq -r '.data[0].id')
clank -m 'reply with exactly: pong'    # -> pong, exit 0
```

The built-in defaults (`http://127.0.0.1:40583/v1`,
`qwen3.8-27b-gsq-rco-iq3xxs`) are one machine's. Those two variables plus the flags
in [Configuration](#configuration) are the whole configuration — no config file, no
state on disk — and a model the server does not serve is the usual first failure. It
is not silent: the server's reason goes to stderr and the exit code is 1.

Container instead of a toolchain: [In a container](#in-a-container).

## Why not an agent framework

A persistent agent harness is a large program whose tool surface you trust by
default, because you will not read it. When that surface can write files and run
commands with your permissions, what you are trusting is the whole machine.

The surface here is stdin, stdout, stderr and an exit code. No daemon, no session
store, no memory you did not hand over. Everything an "agent" does internally is a
pipeline step you can read — `rg`, `git diff`, `jq`, `sed`, `xargs -P`.

So the input is visible: the context is exactly what you piped. The cost is
visible: one call is one call, with no loop re-sending a growing history. And the
write stays yours — clank observes and proposes, a gate decides, you apply. In a
container with the tree mounted read-only that boundary belongs to the kernel
rather than to clank's promises; it bounds what the model can *write*, not what it
can *reach*.

What it does not give you: memory across sessions, retrieval you did not construct,
or a loop that edits your code while you are away. Those are what a harness is for.

## What it is for

Ask a model about text you already have, and get back something your shell can act on:

```sh
rg -n -C3 "userData" src/ | clank --thinking off -m "what does this do?"
git diff | clank --thinking off --max-tokens 400 -m "review this diff, one line per issue"
rg -n "TODO" src/ | clank --each --thinking off -m "one line: actionable now, or not?"
clank --json-schema @schema.json -m "extract the findings" | jq -er .
clank --jsonl -m "summarize" | clank -m "what did you say?"
clank --system @prompts/review-sh.md -c script.sh -m "review the script"
```

## Unix contract

- **stdout is data.** The answer, framed `--each` answers, or `--jsonl` events —
  `run`, `item`, `tool_call`, `tool_result`, `assistant`, `error` — and nothing
  else. A `--jsonl` stream opens with a `run` event naming the model, the endpoint
  and the argv (API keys redacted), so a trace says what produced it.
- **stderr is diagnostics.** Tool breadcrumbs, reasoning under `--show-thinking`,
  errors. Token accounting stays in the server's log, where it already lives.
- **exit codes.** `0` ok; `1` failure — model, server, IO, a truncated answer, an
  empty answer, or any failed `--each` item; `2` usage. Truncation and emptiness
  are failures, not results: a stage that reports a success it cannot back is worse
  than one that fails.
- **prompt** comes from `-m TEXT`, positional text, piped stdin (stdin is the prompt
  only when no other prompt is given), or one line from a TTY.
- **context** is piped stdin, which becomes a context node when a prompt is also
  given: a clank trace renders as a transcript, anything else as text. `-c FILE`
  (repeatable) loads a saved context — JSON tree, text, or a trace. `-c` nodes come
  first, then the pipe; a pipe that reaches the process is never dropped.
- **tools** are offered only when nothing was piped — no stdin, no `-c`, no `--each`
  items — because with no evidence to hand over, the only honest answer about your
  workspace is one that looked at it. Every lookup lands on stderr. When evidence
  was supplied, that is the evidence and nothing else. `--no-tools` forces the blind
  case, and the prompt then forbids citing what it never saw.
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

File leaves are read at render time and emitted as `─── path ───` plus content. A
`-c` file is validated strictly, because a file is configuration someone wrote on
purpose; the pipe parses leniently, because it is evidence.

## Map (`--each`)

One prompt over every item on stdin, serially, one conversation per item. The items
are the context, so the prompt has to come from `-m`/positional.

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

**An item is text, not a file.** The item is the context, so `rg -l "TODO" src/ |
clank --each -m "summarize this file"` hands the model a filename and nothing else,
and it answers accordingly — measured on oMLX (2026-09-19): *"This file exists but
its purpose is not described in the provided context."* To summarize files, let the
shell read them:

```sh
for f in src/*.rs; do
  clank -c "$f" -m "one line: what is this file responsible for?" </dev/null
  echo
done
```

`</dev/null` because clank reads stdin — without it the first call eats the rest of
the list. Content you already gathered maps fine: `rg -n "TODO" src/ | clank --each
-m "one line: actionable now, or not?"`.

`--jsonl` adds `i` (1-based) and `of` to every event and emits one `item` event
carrying the input before the work starts, so a consumer never has to guess. A
failed item is an `error` event and a stderr line, and it does not stop the run —
fifty items should not be thrown away because the third one timed out. The exit
code is `1` if any item failed. An empty item list is not a failure.

Everything except the trailing item block is byte-identical from item to item,
which is what lets the server reuse the prompt prefix. There is no parallelism
inside clank: `-P` belongs to `xargs`.

## Keeping it fast

The model is the constraint — clank's own share is 1 ms of startup — so the lever
is the tokens you pay for:

| rule | measured |
|---|---|
| `--thinking off` for mechanical work | 1.35 s → 0.25 s |
| keep the prompt head stable, vary the tail | 10.4 s → 0.21 s (cached prefix) |
| stop reading when you have enough (`clank … \| head -1`) | 7.1 s → 0.6 s |
| one call holding many items beats many small calls | 1.5× on four items |

Per call: ≈0.25 s fixed, ≈0.02 s per output token, prompt tokens at 1.3 ms cold and
0.02 ms cached. `docs/research-harness-constraints.md` §5.2 has the raw numbers and
§5.3 the comparison against headless pi.

## Tools (`--tools`, opt-in)

| tool | description |
|---|---|
| `read_file(path, start_line?, end_line?)` | numbered file content |
| `list_dir(path)` | directory entries (name, type, size) |
| `search(pattern, path?, ignore_case?, context_lines?, glob?)` | regex search, `file:line: text` |
| `stat(path)` | file/directory metadata |

A tool call is the wrong way to feed a stage: it is a worse `rg` with an input
nobody can see in the pipeline. The composable form of a lookup is a pipe, and the
composable form of looking twice is a second stage. Reach for `--tools` when one
lookup inside one stage is genuinely cheaper than a second stage.

`search` is deliberately weaker than ripgrep: line-based, single-line patterns,
capped at 500 matches and 4 MB per file, skipping binaries, symlinks and `.git`.
Flags: `ignore_case`, `context_lines` (0–10), `glob`. For anything heavier, compose
with the real tool.

Tool errors go back to the model as data. A bad context file or a server error is a
process failure.

## Decisions for routing (`clank-jev`)

A second binary in this crate, for one job: ask typed questions about a state and
get answers a shell can branch on. `clank` writes prose; `clank-jev` picks one of
*your* options and says how sure it is.

```sh
route=$(printf '%s' "$task" | clank-jev --ask 'What kind of task is this?' \
          --choice code,prose,math --min-prob 0.7) || route=unclear
case "$route" in
  code) clank --model local-code -c src/context.rs -m "$task" ;;
  *)    clank --model local-fast -m "$task" ;;
esac
```

**Why a separate binary rather than a `clank` flag.** clank's contract is one
prompt, one request, one answer, no second wire protocol (invariants 1–3). A
decision stage also stands on its own — a git hook, a Makefile, a cron job — and a
script that wants a decision should not have to carry a chat client to get one.

**Providers.** Credentials come from the environment, never from argv:

| `--provider` | endpoint | credential | default model |
|---|---|---|---|
| `auto` *(default)* | whichever credential is set | — | — |
| `typesafe` | `api.typesafe.ai/v1/systemone` | `TYPESAFE_API_KEY` (or `JEV_API_KEY`, `JEV_CLI_API_KEY`) | `jev-latest` |
| `openrouter` | `openrouter.ai/api/alpha/decisions` | `OPENROUTER_API_KEY` | `typesafe/jev-1.13` |
| `kev` | `127.0.0.1:8009/v1/systemone` (`--base-url` to move it) | none | `kev-latest` |

The `kev` provider is a local System One server — [kev](https://github.com/jaredpalmer/kev)
is a trained Jev-family model (LoRA + pointer readout head on Qwen, one prefill
pass) that speaks the same request and response shapes, so it needs no
credentials and no network:

```sh
git clone https://github.com/jaredpalmer/kev && cd kev && uv sync --extra serve
KEV_DTYPE=fp32 uv run --extra serve python -m kev.serve --run jaredpalmer/kev-0.6b --port 8009
```

Measured on the same 18 typed decisions as the table in
[docs/decision-readout.md](docs/decision-readout.md): `kev-0.6b` **16/18 = 89%**,
83 ms median, control 17% (hosted Jev: 94%, 591 ms, $0.000015). It passes the
shuffled-context control, so the accuracy comes from the state. `kev-4b` is the
checkpoint they recommend and it does not fit here — it serves bf16 only, ~8.5 GB
against 16 GB shared with a resident oMLX model.

**A closed-choice reason rides along with the value**, decided in the same request
— a judgment, not just a score. `fixtures/checks-verification.json` asks the two
questions a shadow watchdog asks, each with its own reason set:

```sh
printf '%s\n' "USER: run the tests, do not touch the config" \
  "TOOL cargo test -> FAILED" "ASSISTANT: all tests pass, config updated" \
  | clank-jev --checks fixtures/checks-verification.json --json --min-prob 0.6
```

```json
{"answers":{"verification":{"type":"noul","value":true,"probability":0.98,
  "reason":"verification_contradiction","reason_probability":1.0}}, ...}
```

**Every question shape is available on the command line**, so a script needs no
file: `--ask` with `--choice A,B,C` (unordered options), `--boolean` (yes/no), or
`--score low,mid,high` (ordered levels). `--checks FILE` takes the full set, Jev's
own JSON shape.

**Gates, and the exit codes a script branches on.** `--min-prob` fails a decision
you asked not to trust; `--expect` and `--expect-min` fail one that is not the
value you needed. A question the provider skipped, or an answer carrying no
probability, fails the gate instead of passing by default. `--print-reason` puts
the closed-choice reason on stdout instead of the value, for a script that routes
on *why* rather than *what*.

`--min-prob` compares against **confidence in the decision**, not the probability of
"yes". For a yes/no question those differ: a decisive *no* has `probability` near 0
and `confidence` near 1, so a gate on the raw probability would reject the model for
being certain. The JSON carries both — `probability` is P(true) for `noul` and the
winning option's share otherwise, `confidence` is `max(p, 1-p)` for `noul` and the
provider's own normalized margin for `choice`/`score` when it sends one.

### Combinations

`clank` and `clank-jev` are both stages, so they compose in both orders. Each of
these was run; the observed behaviour is quoted.

**Route, then generate** — the decision picks the model, clank does the work:

```sh
route=$(printf '%s' "$task" | clank-jev --ask 'What kind of task is this?' \
          --choice code,prose,math --min-prob 0.7) || route=unclear
case "$route" in
  code) clank --model local-code -c src/context.rs -m "$task" ;;
  *)    clank --model local-fast -m "$task" ;;
esac
```

**Generate, then validate** — clank writes, jev checks it against a rubric, and a
failed check stops the pipeline. `fixtures/checks-commit.json` asks whether the
message describes the diff and what shape its subject line has:

```sh
msg=$(git show HEAD | clank -q --thinking off -m 'Write the commit message for this diff.')
{ git show --stat HEAD; printf 'MESSAGE:\n%s\n' "$msg"; } \
  | clank-jev --checks fixtures/checks-commit.json --min-prob 0.6
```

*Observed:* `describes` = true, reason `no_conflict`, p=0.89 — and the gate still
fired at `shape` (p=0.500), because `README.md: update …` is a path prefix rather
than a typed one. That is the gate doing its job on a genuinely ambiguous answer.

**Audit a trace after the fact** — a `--jsonl` trace is evidence, so the watchdog
questions can be asked of it afterwards:

```sh
clank --jsonl -m 'where is the transcript cap defined? cite file:line' > trace.jsonl
clank-jev --checks fixtures/checks-verification.json --min-prob 0.6 < trace.jsonl
```

*Observed:* `verification` = false, reason `no_conflict` (the citation was real),
and `instruction` at p=0.23 — no user instruction exists in a trace, so the check
correctly reports it cannot decide, and the gate fails the run rather than
reporting a clean bill of health.

**Fan out, act only on confident decisions** — the loop is the shell's:

```sh
while read -r subject; do
  v=$(printf '%s' "$subject" | clank-jev -q --ask 'Could this break an existing caller?' \
        --boolean --min-prob 0.7) || { echo "unclear: $subject"; continue; }
  [ "$v" = true ] && echo "check: $subject"
done < <(git log --format=%s -8)
```

*Observed:* 7 of 8 doc-only subjects decided `false` at confidence ≥ 0.83, and one
came back `unclear` at 0.68 — the run that found the `--min-prob` semantics above.

**Escalate: local first, hosted only when the local answer is not confident** — a
credential ladder, the same shape as a model ladder:

```sh
ask() { printf '%s' "$1" | clank-jev -q --provider "$2" --ask 'Which team owns this?' \
          --choice BILLING,TECHNICAL,ACCOUNT --min-prob "$3"; }
v=$(ask "$state" kev 0.9) || v=$(ask "$state" openrouter 0.5)
```

*Observed:* the local `kev-0.6b` decided `TECHNICAL` at confidence 0.84, below the
0.9 gate, so the hosted Jev was asked and agreed — 83 ms and $0 spent before
reaching the network.

**Break a tie between two answers** — two models, one judge:

```sh
a=$(clank -q --model local-fast -c src/context.rs -m "$task One line.")
b=$(clank -q --model local-code -c src/context.rs -m "$task One line.")
printf 'A: %s\n\nB: %s\n' "$a" "$b" \
  | clank-jev --ask 'Which answer names the exact file:line and the correct value?' --choice A,B --min-prob 0.6
```

*Observed:* it picked B at p=0.75, confidence 0.51 — and **the gate fired**, because
neither answer had the right line number. A tie-break that can say "both of these
are wrong" is the reason to use one.

| code | meaning |
|---|---|
| `0` | decided, and every gate passed |
| `1` | a gate failed — **the decision is still printed**, because the caller asked not to trust it, not to lose it |
| `2` | usage: no question shape, an empty state, a malformed checks file |
| `3` | provider, network or credentials |

stdout is the bare value for one question (`code`, `true`, `2`), a JSON object for
several; diagnostics and every gate failure go to stderr; `-q` silences them.

## Configuration

Flags override `$CLANK_*` environment variables, which override built-in defaults:

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
shell's own measurements; stage 3 finalizes it under `--json-schema`; stage 4 has
the model propose a command that verifies the result. The fixture is piped at every
stage, so no stage asks the model to read the filesystem for itself; every stage is
gated, and a non-zero exit, an answer that is not the expected JSON, or a script
that fails `bash -n` stops the run. One `--jsonl` trace per stage lands in
`local/demo-traces/`.

Its first version did print "passed" after producing a broken artifact. That is what
the gates are for.

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

With nothing piped, the read-only observers look around inside the container, each
lookup on stderr:

```
> search context_lines=0 glob=* ignore_case=true path=. pattern=exit.?code|EXIT_|exit\(2\)|return 2
< search ok (3019 B)
> list_dir path=.
< list_dir ok (326 B)
```

and the answer cited `src/main.rs:10`.

Four details are not optional. `-i` is what lets the pipe reach it. `--network=host`
is what makes a model on localhost reachable, and it is also the limit of the
sandbox. `:ro` is not decoration: mounted read-only, `touch /w/pwned` returns
`Read-only file system`. And not alpine — clank is glibc-dynamic, and musl has no
loader for it; `ubuntu:24.04` and `debian:stable-slim` both work.

## Verified

* Endpoints: qwen 3.8 27b (Qwen3.8-27B GSQ-RCO IQ3_XXS) on `:40583` and qwen 3.6
  35B-A3B (Qwen3.6-35B-A3B-UD-Q5_K_S) on `:37313`, both via the OpenAI-compatible
  endpoint with SSE streaming and tool calling; plus a remote gateway on `:4000`.
  On 2026-09-19, oMLX 0.7.0 on `:8000` serving MLX models on a 16 GB M1 Pro:
  `Qwen3.5-9B-MLX-4bit`, `MiniCPM5-2B-MLX-8bit` and `gemma-4-E4B-it-MLX-4bit` all
  answer, stream and expose reasoning; the fourth listed model,
  `Bonsai-2-27B-CRACK-1.75bit-JANG`, loads on no request at all — the runtime
  answers 409 and clank prints the server's reason verbatim, 402 parameter names
  included. Details and raw output in `docs/macbook-omlx-local-inference.md`.
* **A top-level `json_schema` is the endpoint's grammar to enforce, not clank's.**
  Against the local endpoints it held: asked to reply `beta` under a schema whose
  only legal value was `alpha`, clank printed `{"word":"alpha"}`. Through the
  gateway (2026-09-19) it did not: a fenced block came back with the wrong key, and
  clank exited 1 with *final output is not valid JSON; --json-schema requested*.
  The contract held either way — a violation is a failure, not a result — but assume
  the grammar holds only where it has been measured. A schema and tools never share
  a request; with `--tools` the rounds run unconstrained and one final request
  carries the schema. This server build rejects the pair (400, by curl probe).
  Measured per model on oMLX the same day: `Qwen3.5-9B-MLX-4bit` returns bare,
  valid JSON; `gemma-4-E4B-it-MLX-4bit` wraps it in a Markdown fence and
  `MiniCPM5-2B-MLX-8bit` answers in prose, both of them `exit 1`. One schema,
  three models, two of them ignoring it — which model carries the grammar is a
  property to measure, not to assume.
* The tools fallback exists because of a measured failure. With nothing piped and no
  tools, `clank -m "which file defines the transcript rendering, and what is the
  output cap? cite file:line"` answered *"`src/renderer.ts`, cap 8000 at
  `src/renderer.ts:14`"* — a file that does not exist, in the citation format of a
  real answer, exit 0. The same question with the tools offered answered
  `src/context.rs:68` and cap `400` at `src/context.rs:136`, with the test that
  asserts it. The fallback is still a fallback: on oMLX the 2B reproduced that
  same invented citation *with* the tools on — two lookups in the stderr
  breadcrumbs, then `src/renderer.ts:14` again. What survives a model that
  fabricates is the exit code and the observable channel, not the feature.
* `--thinking off` arrives as `chat_template_kwargs.enable_thinking=false` and a
  level as `reasoning_effort`, with no field at all by default and exit 2 for an
  unknown level. It removes reasoning on both local endpoints; an effort level is
  honoured by one and silently ignored by the other. Reasoning reaches stderr only
  under `--show-thinking`, and stdout stays the answer alone.
* `cargo test` drives the real binary against a stub SSE server that records every
  request body, and asserts what this README and PROTOCOL.md claim: the default is
  one request with no tools; a schema and tools never share a request; the schema'd
  round carries the tool results it read; piped evidence is still context when `-c`
  is used; `--each` frames one answer per item (byte-exact) and shares the prompt
  prefix; a failed item is an `error` event plus exit 1 while the others are still
  answered; an empty item list exits 0; a truncated or empty answer exits 1 rather
  than reporting success; the `run` event's argv has any `--api-key` redacted; a
  trace with a `run` header still reads back as context.
* `clank-jev` (the decision stage) is verified live two ways: against hosted Jev
  through OpenRouter's Decisions endpoint — `fixtures/checks-verification.json`
  returned `instruction_conflict` and `verification_contradiction`, both at p ≥ 0.93,
  in one request — and against a local `kev-0.6b` on `:8009`, where 18 typed
  decisions scored 16/18 with a 17% shuffled-context control at an 83 ms median and
  no credentials. Its contract is guarded the same way clank's is: `tests/docs.rs`
  reads its `--help` and requires every flag to appear in README.md and
  CHEATSHEET.md, and requires PROTOCOL.md to document exit code 3.
* `demo.sh` ran end to end on 2026-09-17 against `:37313`: four stages, 61 s, all
  gates passed. Its first run found a real defect — an ungated stage wrote a broken
  candidate and the script still announced success. On 2026-09-19 against oMLX it
  passed on `Qwen3.5-9B-MLX-4bit` in 35 s, and stopped at a different gate on each
  of the other two: gemma on the fenced schema at stage 3, the 2B on a truncated
  answer at stage 1.
* Live: tree context renders text, file and nested nodes in document order;
  `--jsonl` parses with `jq`; repeatable `-c` concatenates files in order (checked
  through the `CLANK_DEBUG` request dump); `--system` appends the directive to the
  system prompt; `clank --jsonl | clank` and `clank -c trace.jsonl` render the prior
  run as a tagged transcript; `clank … | head -1` exits 0; `--list-tools` prints the
  four definitions without a model call; no prompt with empty stdin exits 2, an
  unreadable context file exits 1, `--help` exits 0.
* `--tools` is no longer unexercised: the oMLX runs sent it through the round
  loop end to end, with the lookups on stderr as promised. Still unexercised
  against a live model: the schema-after-tools sequence — the wire tests cover
  that request and output contract only. TTY-with-no-prompt reads one line, in code; it is not exercisable headless.

## Provenance

Code and docs are largely LLM-written and then read and curated. The commits are the
record of what changed, when, and each of the claims under *Verified* was run.
