# clank with local models (oMLX) on this MacBook

`clank 0.1.0` (`~/.cargo/bin/clank`) driving **oMLX 0.7.0.dev4** on
`http://127.0.0.1:8000/v1` — MacBook Pro M1 Pro, 10 cores, 16 GB unified,
macOS 15.7.5. One model is resident at a time, so the server evicts and reloads
on a switch (~2 s for the 2B, ~5–7 s for the 9B, ~10 s for gemma): pick a model
per job, not per call.

```sh
source local/macbook/omlx-env.sh Qwen3.5-9B-MLX-4bit
```

Sets `CLANK_BASE_URL`, `CLANK_MODEL`, `CLANK_API_KEY` (key read from
`~/.omlx/settings.json`; nothing written to disk). Override the model per call
with `--model`.

## Which model for what

| model | use it for | it will not |
|---|---|---|
| `Qwen3.5-9B-MLX-4bit` (5.6 GB) | anything whose output must parse or that needs a citation: review, extraction, `--each`, `--tools` | be quick — 6–8 s for a short answer |
| `MiniCPM5-2B-MLX-8bit` (2.5 GB) | short mechanical text: rewrite, classify, label, one-line summaries. ~0.5 s warm | hold a format, respect a token budget when it rambles, cite a file truthfully |
| `gemma-4-E4B-it-MLX-4bit` (6.4 GB) | reading prose you piped | parse (`--json-schema` comes back fenced), obey `--thinking`, answer a `--tools` question |
| `Bonsai-2-27B-CRACK-1.75bit-JANG` | — | load at all: the runtime rejects it (409, *402 parameters not in model*) |

**2B for text, 9B for truth.** These are the jobs worth keeping, all run against
this endpoint; where output is quoted it is what came back, verbatim. The measured
version of that table — who invents what, who uses the context, whose code runs —
is at the end: *Hallucination, context, code*.

## Orienting in code you did not write

```sh
clank -q --thinking off -m 'What does this file do, what does it assume about its caller, and what would break it? Six lines max.' < src/context.rs
```

Pipe the function, not the file:

```sh
sed -n '100,150p' src/tools.rs | clank -q --thinking off \
  -m 'In one paragraph: what does this function guarantee, and what does it silently accept?'
```

Find the callers, then ask what they rely on — the search is the shell's:

```sh
rg -n -C3 'render_transcript' src/ | clank -q --thinking off \
  -m 'The context is every caller of render_transcript. One line per caller: what it passes and what it assumes about the return value.'
```

Onboarding over a whole tree — one process per file, file as context, stdin not
shared:

```sh
for f in src/*.rs; do
  clank -q --thinking off -c "$f" -m 'One line: what is this file responsible for?' </dev/null
  echo
done
```
```
This is a Rust file for `clank`, a minimal unix-style local-inference harness…
This is a Rust module defining read-only filesystem tools (read_file, list_dir, search, stat)…
This is a streaming chat client for the OpenAI API via `ureq`, handling SSE streams and reasoning control.
A tool for parsing and rendering structured text, tree, and file evidence.
```

`</dev/null` matters — clank reads stdin, so the first call eats the rest of the
list without it. `xargs -P2` instead of the loop is **2× slower** here (12.5 s vs
5.8 s over four files): the engine batches what you send it, and each extra
process pays a cold prefill.

## Reviewing a change before you commit

```sh
git diff | clank -q --thinking off --max-tokens 700 \
  -m 'The context is a git diff. One line per issue, most important first. If there is no issue, say none.'
```
```
none
```

Two things make this usable: the **budget is explicit**, and overflow is a
failure, not a truncated review. The same command with `--max-tokens 300` spent
26 s and exited 1 — `answer truncated at 300 tokens; raise it or narrow the
prompt` — with the partial review already on stdout. A `&&` chain stops; a human
sees why.

Review only what is staged, one line per hunk:

```sh
git diff --cached | clank -q --thinking off --max-tokens 600 \
  -m 'Review the staged diff. Per hunk: the risk it introduces, or "fine". No summary, no praise.'
```

## Writing the commit message from the diff

```sh
git show HEAD | clank -q --thinking off --max-tokens 300 \
  -m 'The context is a git diff. Write the commit message: a subject line under 60 chars, then a short body if the change needs one. Output the message only.'
```

Then keep it if it is right. There is no `--commit` flag, by design: clank
proposes, the shell applies.

## Triage: the failing test, the compiler, the log

```sh
cargo test 2>&1 | tail -40 | clank -q --thinking off \
  -m 'The context is test output. In three lines: what failed, the smallest hypothesis for why, and the next command to run.'
```
```sh
cargo build 2>&1 | clank -q --thinking off \
  -m 'The context is compiler output. One line per error: the cause in plain words, then the fix. Skip suggestions that repeat the error text.'
```
```sh
grep -aE 'ERROR|Traceback|failed' app.log | tail -30 | clank -q --thinking off \
  -m 'The context is lines from a log. One line: what went wrong, and the first line that shows it.'
```

`tail` first: the 9B is 6–8 s per call, and 40 lines of a stack trace answer the
question as well as 4,000.

## Extracting what you need to act on

Schema-constrained, gated by `jq`, only on the 9B:

```sh
git diff --cached | clank -q --thinking off \
  --json-schema '{"type":"object","properties":{"breaking":{"type":"array","items":{"type":"string"}},"files":{"type":"array","items":{"type":"string"}}},"required":["breaking","files"]}' \
  -m 'The context is a git diff. JSON only: the files touched, and the breaking changes, as empty arrays if there are none.' \
  | jq -er .
```

Schema enforcement is per **model**, measured on this endpoint with a
one-property enum schema:

| model | what came back | exit |
|---|---|---|
| Qwen3.5-9B-4bit | `{ "verdict": "risk" }` | 0 |
| MiniCPM5-2B-8bit | `verdict: risk` | 1 — *not valid JSON* |
| gemma-4-E4B-4bit | a sentence about the context not containing an anonymizer | 1 |

## Sweeping a class of comments or items

```sh
rg -n 'TODO|FIXME|XXX' src/ | clank --thinking off --each \
  -m 'One line: is this actionable now? If yes, the smallest next step. If no, "not now".'
```

Framed per item on stdout, so `awk '/^─── item /{…}'` splits it back; `--jsonl`
carries `i`/`of` on every event. Same shape for rewriting a list in place:

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

**Items are text, not files.** `printf 'src/main.rs\n' | clank --each -m 'what does this file do?'`
answers *"src/main.rs is the main source file for a Rust application"* — the model
never saw the file. If the items have to be read, the shell reads them: use the
`-c` loop above, or `--each` over content you already gathered (`rg -n … | clank --each`).

## Asking for the test, then the command that verifies it

```sh
sed -n '1,80p' src/context.rs | clank -q --thinking off \
  -m 'The context is a function. Propose the cases a test must cover — the boundary, the error path, the surprising input — as a list. Do not write the test.'
```

Then let it propose the verification, and gate the proposal mechanically:

```sh
clank -q --thinking off \
  --json-schema '{"type":"object","properties":{"cmd":{"type":"string"}},"required":["cmd"]}' \
  -m 'The context is a Rust crate. JSON only: a single bash one-liner that runs the test suite and prints OK only if it passes.' \
  < README.md | jq -er .cmd > verify.sh
bash -n verify.sh && sh verify.sh     # check, then decide to run it yourself
```

Nothing in this loop executes automatically: `jq -er` refuses a malformed answer,
`bash -n` refuses a broken script, and the last step is you.

## A second pass over your own reasoning

A `--jsonl` trace is context for the next stage — same model, fresh context, no
stored session:

```sh
clank --thinking off --jsonl --no-tools -m 'Name three grep options, one per line, option then a five-word description.' > s1.jsonl
clank --thinking off -m 'The context is a transcript of an earlier stage. Which option is the one for case-insensitive matching? One line.' < s1.jsonl
```
```
The option for case-insensitive matching is -i.
```

`--no-tools` on stage 1 matters: with nothing piped clank offers its read-only
observers, and a general question then comes back as *"I can't answer this request
yet because I don't know what grep file to look at."* Say what you mean.

## A repo-specific reviewer, kept in the repo

```sh
clank --thinking off --system @docs/review.md -m 'Review the diff in the context.' < <(git diff)
# docs/review.md: a file you keep in the repo — what to flag, what to ignore, the format back
```

The flag appends the file to the system prompt; the file is where house rules
belong (what to flag, what to ignore, the format you want back). Tested with a
nonsense directive — `Answer only with the word: banana.` answered `banana` to
`what is 2+2?` — so the block is a contract, not a hint.

## When you do not know where to look

With nothing piped, clank offers four read-only observers (`read_file`,
`list_dir`, `search`, `stat`) and every lookup lands on stderr:

```sh
clank --thinking off --tools -m 'where is the transcript output cap defined? cite file:line and the value.'
```

| model | answer | verdict |
|---|---|---|
| Qwen3.5-9B-4bit | `400` at `src/context.rs:136` | correct |
| MiniCPM5-2B-8bit | `8000` at `src/renderer.ts:14` | invented — no such file |
| gemma-4-E4B-4bit | `500 matches and 4 MB per file` | the *search* tool's caps, not the question asked |

The 2B's entire investigation, before it invented that citation:

```
> search path=. pattern=per-file.*cap|cap.*per-file|output.*cap|cap.*output
< search ok (39864 B)
> read_file end_line=320 path=./README.md start_line=290
< read_file ok (2750 B)
```

A broad search and a README slice cannot support `src/renderer.ts:14`, and the run
still exits 0 — text came back, so clank has nothing to fail. The breadcrumbs are
the only thing that catches it. Below ~9B, pipe the evidence instead of trusting
the fallback. It is also not a sandbox: the observers reach any path you can read.

## Two habits that pay for themselves

```sh
clank --thinking off -m 'count from one to fifty, one number per line' | head -1
```

0.8 s and `rc=141` (SIGPIPE) instead of 6.8 s of tokens nobody reads.

```sh
clank --thinking off --each -m '…' < list      # one call, N items, shared prefix
```

`--each` is serial by design and shares the prompt prefix; `xargs -P` above a
single local engine only adds cold prefills. Fan out when you have several
endpoints or several machines, not because the loop looks parallelisable.

## Four stages, every one gated

`./demo.sh` is the reference pipeline: generate a script from a fixture, critique
it against the fixture **and the shell's own measurements**, finalize it under
`--json-schema`, then have the model propose the command that verifies the result.
Model output is syntax-checked and never executed.

| model | result |
|---|---|
| Qwen3.5-9B-MLX-4bit | passes end to end in ~35 s |
| gemma-4-E4B-it-MLX-4bit | stops at stage 3: the schema request came back fenced |
| MiniCPM5-2B-MLX-8bit | stops at stage 1: `answer truncated at 8192 tokens` |

Three models, three different gates firing, no false "passed".

## Hallucination, context, code: which model to trust offline

Three families of probe, run twice against this endpoint. Grading is mechanical:
keyword rules for "did it deny what it cannot know", and an actual `bash` run plus
a `rustc --test` compile for the code. `local/macbook/omlx-probes.sh` runs them,
`omlx-probes-grade.py` grades them, raw answers land beside the verdicts.

| probe | the ask | 2B | 9B | gemma |
|---|---|---|---|---|
| H1 a file that does not exist (observers on) | deny it | pass | pass | pass |
| H1b the same file, seeing nothing | say so | pass | pass | pass |
| H2 a flag that does not exist | deny it | pass | pass | pass |
| H3 a port the piped note does not mention | decline | pass | pass | pass |
| H4 that file again, with unrelated context piped | decline | pass | pass | pass |
| H5 `--max-rounds`'s default, never supplied | say unknown | **"10"** | "None" | pass |
| X1 retry window + error code, only in the piped spec | 7, E42 | pass | pass | pass |
| X2 which function drops the local part | | pass | pass | pass |
| C1 a one-liner over a csv | must run and match | pass | **header row** | pass |
| C2 `pub fn count_words` in Rust | must compile and pass | pass | pass | pass (dropped `pub`) |

**Nothing invented a file, a flag or a missing detail.** All three declined H1–H4,
including the trap with unrelated context piped, and all three answered the context
probes exactly — `7, E42` while the retired `3` and `E00` sat in the same file as
distractors half a sentence later.

The single fabrication was a **number nobody could check**. Asked for
`--max-rounds`'s default with nothing supplied, the 2B answered `10`; the flag
defaults to 12, and clank exited 0 because text came back. The 9B said `None`. If
you take one rule from this page: a local model's confident number is the one thing
its own honesty rules cannot catch.

The 2B's denial is worth reading in full, because it is what good looks like — it
refused the premise and then did the lookup:

```
The file `src/renderer.rs` does not exist in this repository. The `src/` directory
currently contains only `client.rs`, `context.rs`, `main.rs`, and `tools.rs`.
There is no `src/renderer.rs` file. Rendering logic is spread across
`src/context.rs` and `src/main.rs`. Here are the relevant parts:
- `src/context.rs:68` — `render_transcript` function …
- `src/context.rs:239` — `Context::render` method.
- `src/main.rs:304` — Calls `t.render()` on a context/tree for display.
- `src/tools.rs:402` — Compact one-line rendering of tool args for breadcrumbs.
```

Every one of those four citations is correct, checked against the files.

**Code generation is where the failures are — semantic, not syntactic.** All the
code compiled and ran; the wrong answers ran fine and printed the wrong thing:

- 9B, C1: `cut -d, -f2 data.csv | sort -u` → `dev ken mira owner`. It forgot the
  header row and included `owner` as a value. (First run it wrote
  `awk -F, 'NR>1{print $2}' data.csv | sort -u | uniq` and passed.)
- 2B, C1 first run: `awk -F, '{print $2}' …` — the same header bug; second run it
  wrote `NR>1` and passed.
- gemma, C2: returned `fn count_words(…)` without the requested `pub`. Compiles
  inside a crate, fails the signature as written in the prompt.

So: never trust the answer, run the artifact. `bash -n` catches syntax, executing it
catches the rest, and that check costs less than the model call did.

**Offline verdict on this machine.** gemma-4-E4B-it is the most dependable of the
three on these ten probes (6/6, 2/2, 2/2) and the least usable for anything
structured — it cannot hold a `--json-schema` and answered the wrong question under
`--tools`. Qwen3.5-9B is the one to use when output must parse or carry a citation;
its code still needs running. MiniCPM5-2B is honest about absence and 4–12× faster,
and its answers need checking: one guessed number, one self-contradiction and one
header-row bug in two passes.

**Read the cells as failure modes, not rates.** Each is n=1–2; the 2B contradicted
itself on X2 (`The local part is kept, and it's dropped by the anonymize()
function.`) in the first pass and answered cleanly in the second. The value of the
probe is that it tells you *what* to check, not how often it breaks.

## What does not work

- **`--json-schema` on the 2B and gemma.** Fenced or prose answers, `exit 1`. Use
  the 9B, or check the shape yourself with `jq -er`.
- **`--tools` below ~9B.** One correct answer, one invention, one misread question
  in the runs here.
- **`--each` over bare paths.** Items are text; the model cannot read them.
- **`xargs -P` against one engine.** Slower than serial, and the answers interleave.
- **`--thinking` on gemma-4-E4B.** No delta — no reasoning mode in that
  quantisation. Real 1.8× on the 2B, small win on the 9B.
- **`Bonsai-2-27B-CRACK-1.75bit-JANG`.** Listed by `/v1/models`, unloadable here;
  clank passes the server's 409 through verbatim, 402 parameter names included.
- **A `while read` loop without `</dev/null`.** The first call consumes the list.
- **Long generations on the 2B.** It repeats until the budget is gone; that is
  `exit 1`, not a long answer.

Raw output for the runs quoted here is under `local/macbook/` (gitignored).
