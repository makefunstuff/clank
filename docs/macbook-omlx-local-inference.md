# clank with local models (oMLX) on this MacBook

`clank 0.1.0` (`~/.cargo/bin/clank`) driving **oMLX 0.7.0.dev4** on
`http://127.0.0.1:8000/v1` — MacBook Pro M1 Pro, 10 cores, 16 GB unified,
macOS 15.7.5. One model is resident at a time (the server's ceiling measured
8.4–9.7 GB, so it evicts and reloads on a switch: ~2 s for the 2B, ~5–7 s for
the 9B, ~10 s for gemma).

```sh
source local/macbook/omlx-env.sh Qwen3.5-9B-MLX-4bit
```

That sets `CLANK_BASE_URL`, `CLANK_MODEL` and `CLANK_API_KEY` (the key is read
from `~/.omlx/settings.json`; nothing is written to disk).

## Which model

| model | size | use it for | it will not |
|---|---|---|---|
| `Qwen3.5-9B-MLX-4bit` | 5.6 GB | anything whose output must parse or that needs a citation: schema-constrained JSON, `--each` framing, `--tools` lookups | be quick — 6–8 s for a short answer |
| `MiniCPM5-2B-MLX-8bit` | 2.5 GB | short mechanical text: rewrite, classify, one-sentence summaries, ~0.5 s warm | hold a format, stay inside a token budget when it rambles, cite a file truthfully |
| `gemma-4-E4B-it-MLX-4bit` | 6.4 GB | reading prose you piped | parse (`--json-schema` comes back fenced), obey `--thinking` (no reasoning mode), answer a `--tools` question |
| `Bonsai-2-27B-CRACK-1.75bit-JANG` | 6.1 GB | — | load at all: the runtime rejects it (409, *402 parameters not in model*) |

**2B for text, 9B for truth.** Everything below was run here.

## 1. Ask about text you already have

```sh
clank --thinking off -m 'in one sentence: what does the email anonymizer keep?' < fixtures/notes.md
```

```
The email anonymizer keeps only the domain part of the email address.
```

The model sees exactly what you piped and nothing else, so there is no question
about its input. The 2B answers the same, shorter, in 0.5 s.

## 2. Review a diff before you commit

```sh
git show HEAD | clank -q --thinking off --max-tokens 700 \
  -m 'The context is a git diff. One line per issue, most important first. If there is no issue, say none.'
```

```
none
```

Same input with `--max-tokens 300` took 26 s and **`exit 1`**:
`answer truncated at 300 tokens (--max-tokens); raise it or narrow the prompt`.
The partial review was already on stdout; the exit code said out loud that it
was unfinished, which is what stops a `&&` chain from shipping it.

## 3. Structured output you gate on

```sh
clank --thinking off --json-schema @fixtures/verdict.schema.json \
  -m 'the anonymizer keeps the domain part but loses the local part. verdict ok or risk? JSON only.' \
  | jq -er .verdict
```

| model | what came back | exit |
|---|---|---|
| Qwen3.5-9B-4bit | `{ "verdict": "risk" }` | 0 |
| MiniCPM5-2B-8bit | `verdict: risk` | 1 — *not valid JSON* |
| gemma-4-E4B-4bit | *"The provided context does not contain information about…"* | 1 |

A top-level `json_schema` is the **endpoint's** grammar to enforce, not clank's.
On this box oMLX honours it for the 9B and ignores it for the other two, which is
why the gate is `jq -er` and the model name in the command matters. clank's half
always holds: a violation is `exit 1`, never a best-effort parse.

## 4. Map one prompt over a list

```sh
git log --format=%s -3 | clank --thinking off --each \
  -m 'one line: rewrite this commit subject in the imperative mood'
```

```
─── item 1/3 ───
readme: add install section and fix name clash

─── item 2/3 ───
use-cases §9: implement loop and goal in eight lines of shell

─── item 3/3 ───
readme: third shorter and remove essay voice
```

The framing is the interface: `awk '/^─── item /{…}'` splits stdout back into
per-item results, and `--jsonl` puts `i`/`of` on every event.

**The items are text, not files.** Piping paths gives the model nothing to read:

```sh
printf 'src/main.rs\nsrc/context.rs\n' | clank --thinking off --each -m 'one line: what does this file do?'
```
```
src/main.rs is the main source file for a Rust application.
This file is a Rust source file.
```

If the items have to be read, the shell reads them — see below.

## 5. Summarize every file in a directory

One process per file, the file passed as context, the loop's stdin not shared:

```sh
for f in src/*.rs; do
  clank -q --thinking off -c "$f" -m 'one line: what does this file do?' </dev/null
  echo
done
```

```
This is a Rust file for `clank`, a minimal unix-style local-inference harness…
This is a Rust module defining read-only filesystem tools (read_file, list_dir, search, stat)…
This is a streaming chat client for the OpenAI API via `ureq`, handling SSE streams and reasoning control.
A tool for parsing and rendering structured text, tree, and file evidence.
```

Two details that cost an afternoon otherwise: **`</dev/null`** — clank reads
stdin, so the first call eats the rest of the list — and **`echo`** — a
single-answer clank writes no trailing newline, so the answers run together.
`xargs -P2` instead of the loop is **2× slower** here (12.5 s vs 5.8 s over four
files): one local engine batches what you send it, and each extra process pays a
cold prefill.

## 6. Feed a run into the next one

```sh
clank --thinking off --jsonl --no-tools -m 'Name three grep options, one per line, option then a five-word description.' > s1.jsonl
clank --thinking off -m 'The context is a transcript of an earlier stage. Which option is the one for case-insensitive matching? One line.' < s1.jsonl
```
```
The option for case-insensitive matching is -i.
```

A `--jsonl` trace re-ingests as a rendered transcript, so a second stage needs no
intermediate format. `--no-tools` on stage 1 matters: with nothing piped clank
offers its read-only observers, and a general-knowledge question then gets
*"I can't answer this request yet because I don't know what grep file to look at."*

## 7. Keep a repo instruction block

```sh
clank --thinking off --system @local/macbook/sys.md -m 'what is 2+2?'
```
with `sys.md` = `Answer only with the word: banana.` → `banana`.

One flag, one file in the repo, no stored session.

## 8. Ask the workspace (nothing piped)

```sh
clank --thinking off --tools -m 'what is the per-file output cap in the transcript renderer? cite file:line.'
```

| model | answer | verdict |
|---|---|---|
| Qwen3.5-9B-4bit | `400` at `src/context.rs:136` | correct |
| MiniCPM5-2B-8bit | `8000` at `src/renderer.ts:14` | **invented** — no such file |
| gemma-4-E4B-4bit | `500 matches and 4 MB per file` | the *search* tool's caps, not the question |

Every lookup lands on stderr, so you can count them — this is the 2B's entire
investigation before it answered `src/renderer.ts:14`:

```
> search path=. pattern=per-file.*cap|cap.*per-file|output.*cap|cap.*output
< search ok (39864 B)
> read_file end_line=320 path=./README.md start_line=290
< read_file ok (2750 B)
```

A broad search and a README slice cannot support that citation, and the wrong
answer still exits 0 — text came back, so clank has nothing to fail. The
breadcrumbs are the only thing that catches it. Below ~9B, treat `--tools` as
unreliable and pipe the evidence instead.

## 9. Stop reading when you have enough

```sh
clank --thinking off -m 'count from one to fifty, one number per line' | head -1
```

0.8 s and pipeline `rc=141` (SIGPIPE) instead of 6.8 s of tokens nobody reads.
The shell's control flow works because clank restores the default SIGPIPE
disposition.

## 10. Four stages, every one gated

`./demo.sh` is the reference pipeline: generate a script from a fixture, critique
it against the fixture **and the shell's own measurements**, finalize it under
`--json-schema`, then have the model propose the command that verifies the
result. Model output is syntax-checked and never executed.

| model | result |
|---|---|
| Qwen3.5-9B-MLX-4bit | passes end to end in ~35 s |
| gemma-4-E4B-it-MLX-4bit | stops at stage 3: the schema request came back fenced |
| MiniCPM5-2B-MLX-8bit | stops at stage 1: `answer truncated at 8192 tokens` |

Three models, three different failures, no false "passed". On small models the
gates are the product, not the answer.

## What does not work

- **`--json-schema` on the 2B and gemma.** Fenced or prose answers, `exit 1`.
  Use the 9B, or check the format yourself with `jq -er` and retry.
- **`--tools` below ~9B.** One correct answer, one invention, one misread
  question in the runs here. Pipe the file instead.
- **`--each` over bare paths.** The items are text; the model cannot read them.
- **`xargs -P` fan-out against one engine.** Slower than serial, and the answers
  interleave on stdout.
- **`--thinking` on gemma-4-E4B.** No delta at all: that quantisation has no
  reasoning mode. It is a real 1.8× on the 2B and a small win on the 9B.
- **`Bonsai-2-27B-CRACK-1.75bit-JANG`.** Listed by `/v1/models`, unloadable
  here — clank passes the server's 409 through verbatim, including the 402
  parameter names, which is the whole diagnosis.
- **A `while read` loop without `</dev/null`.** The first call consumes the list.
- **`clank --tools` as a sandbox.** It bounds what the model can *do*
  (observe), not what it can *reach*: with your permissions it can read any
  path you can read.

Raw output for every example above is under `local/macbook/` (gitignored).
