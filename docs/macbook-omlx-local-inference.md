# clank against oMLX on a 16 GB MacBook Pro

A demonstration record: clank driving local models served by **oMLX** (MLX runtime,
OpenAI-compatible endpoint) on the machine this was written on. Every number below
was measured on this box on **2026-09-19**, against `http://127.0.0.1:8000/v1`, and
the raw evidence is in `local/macbook/` (gitignored):

| evidence directory | what it holds |
|---|---|
| `omlx-runs-20260919T201041/` | the 17-test contract battery: one `.out`/`.err`/`.rc`/`.secs` per test |
| `omlx-matrix-20260919T201922/` | 4 models × 7 tests, with per-model `/health` snapshots and a 1 Hz memory sample |
| `omlx-extra-20260919T202643/` | raw schema answers, warm reasoning A/B, SIGPIPE, clank's own footprint, fan-out, endpoint throughput without clank |
| `omlx-uc-20260919T202951/`, `omlx-uc-20260919T203235/` | the use-case pass, one file of raw output per case |
| `omlx-env.sh`, `omlx-matrix.sh`, `omlx-extra.sh`, `omlx-usecases.sh` | the harnesses that produced all of it |

Claims are labelled: **[V]** = run and observed here, **[P]** = stated in this
repo's own docs, **[U]** = not verified.

---

## 1. The machine

```
Model Name:        MacBook Pro          Model Identifier:  MacBookPro18,1 (MK183KS/A)
Chip:              Apple M1 Pro         10 cores (8 performance + 2 efficiency)
GPU:               16 cores, Metal 3
Memory:            16 GB unified (17,179,869,184 bytes)
macOS:             15.7.5 (24G624)
Disk:              APFS, 57.5 GB free
```

Two facts about this spec drive everything below:

1. **16 GB is shared by CPU, GPU and every resident model.** There is no slab of
   VRAM to fill; a model that does not fit in unified memory does not fit at all.
2. **One model fits comfortably, two do not.** The four models served here are
   20.6 GB on disk between them — more than the machine has RAM — and the server's
   own admission ceiling measured 8.4–9.7 GB during this session **[V]**. So
   exactly one model is resident, and switching costs a reload.

On a machine like this the harness in front of the model is not free either: a
Python or Node agent stack with a plugin tree is hundreds of megabytes before it
says anything, and it holds state across calls that makes the resident model hard
to evict. That is the hole clank fits into.

## 2. The server: oMLX 0.7.0.dev4

oMLX is the Mac app serving the models; `omlx-server` is the process clank talks
to. Configuration read from `~/.omlx/settings.json` (mode 600) **[V]**:

| setting | value | what it means here |
|---|---|---|
| `server.host` / `server.port` | `127.0.0.1` / `8000` | the endpoint is `http://127.0.0.1:8000/v1` |
| `server.sse_keepalive_mode` | `chunk` | streaming keeps the SSE connection alive during long prefills |
| `auth.api_key` | set | every request needs `Authorization: Bearer …`; the same secret clank takes as `CLANK_API_KEY` |
| `memory.prefill_memory_guard` | `true`, tier `balanced` | prefill is throttled/evicted rather than allowed to push the box into swap |
| `memory.soft_threshold` / `hard_threshold` | `0.85` / `0.95` | admission and eviction targets as a fraction of what is available |
| `scheduler.max_concurrent_requests` | `8` | the server will accept 8 in flight; the engine still batches them |
| `cache.enabled` | `true`, SSD cache `auto`, hot cache `0` | prompt/KV caching is on, on SSD |
| `model.model_dir` | `/Users/jurip/.omlx/models` | 23 GB of weights across 8 model directories |
| `server.auto_start_on_launch` | `true` | no manual server start |

### What is being served

| model id | quantisation | on disk | resident when loaded | load time |
|---|---|---|---|---|
| `MiniCPM5-2B-MLX-8bit` | MLX 8-bit, group 64 | 2.5 GB | 2.62 GB | 2.0 s |
| `Qwen3.5-9B-MLX-4bit` | MLX 4-bit, group 64 | 5.6 GB | 5.82 GB | 5.0–7.2 s |
| `gemma-4-E4B-it-MLX-4bit` | MLX 4-bit (some layers 8-bit) | 6.4 GB | 6.68 GB | 9.5–11.6 s |
| `Bonsai-2-27B-CRACK-1.75bit-JANG` | 2-bit affine, JANG format | 6.1 GB | — | **never loads** (§6) |

Load times are from the server's own log; resident sizes from `/health`
(`engine_pool.current_model_memory`) **[V]**.

### The eviction you can watch happen

Twenty-one evictions were logged during the session. One line, verbatim **[V]**:

```
Evicting 'MiniCPM5-2B-MLX-8bit' to fit 'gemma-4-E4B-it-MLX-4bit'
under the admission soft target (9.57GB > 8.35GB)
Unloaded model: MiniCPM5-2B-MLX-8bit, freed=2.49GB (expected>=629.96MB)
Loading model: gemma-4-E4B-it-MLX-4bit
Loaded model: gemma-4-E4B-it-MLX-4bit (actual: 4.95GB, local estimate: 6.68GB)
```

This is the mechanism that makes a 16 GB Mac serve models that individually want
most of it. It only works if nothing above the server pins state across requests —
which is exactly clank's design (invariant 1: *one invocation = (prompt, context) →
answer; nothing persists* **[P]**).

## 3. Pointing clank at it

Three environment variables are the whole configuration; there is no config file
and no state on disk.

```sh
export CLANK_BASE_URL=http://127.0.0.1:8000/v1
export CLANK_MODEL=MiniCPM5-2B-MLX-8bit
export CLANK_API_KEY=$(jq -r .auth.api_key ~/.omlx/settings.json)   # not echoed, not written
```

`local/macbook/omlx-env.sh` does exactly this (optionally taking a model name):

```sh
source local/macbook/omlx-env.sh Qwen3.5-9B-MLX-4bit
clank --thinking off -m 'reply with exactly: pong'
```

First round trip, verbatim **[V]**:

```
$ time clank --thinking off -m 'reply with exactly: pong'
pong
real    0m2.535s
```

The 2.5 s is the model load; the same call warm is 0.48 s on the 2B, 1.37 s on
gemma and 5.92 s on the 4-bit 9B (§5).

## 4. What clank costs this machine, and what it does not

Measured while a call was in flight **[V]**:

| quantity | value |
|---|---|
| binary | 5,236,880 bytes (one Mach-O, six direct deps) |
| peak RSS during a request | **3,888 KB** |
| requests per invocation | one (two phases with `--tools`: unconstrained rounds, then one constrained answer **[P]**) |
| state after exit | none |
| `clank --list-tools` wall time from a shell | 0.19 s (dominated by process spawn; the README's own figure for clank's internal startup is 1 ms **[P]**) |

So the entire harness footprint on this box is under 4 MB resident, against
2.6–6.7 GB for the model. That asymmetry is the argument: **the model is the
constraint, so the tool in front of it should not be a second one.** A stage that
costs 4 MB and half a second is one you can put in a pipeline twenty times.

## 5. The matrix: which of these models clank can actually drive

`local/macbook/omlx-matrix.sh`, 7 tests per model, raw output per cell. `rc` is the
exit code clank returned — remember `rc=1` is often the *correct* answer (a
fabricated success would be the bug).

| test | Bonsai-27B-1.75bit | MiniCPM5-2B-8bit | Qwen3.5-9B-4bit | gemma-4-E4B-4bit |
|---|---|---|---|---|
| A1 round trip, `--thinking off` | rc=1, 0.35 s (409, never loads) | rc=0, 3.48 s (cold) | rc=0, 15.04 s (cold) | rc=0, 25.43 s (cold) |
| A2 round trip, default reasoning | rc=1 | rc=0, 1.05 s | rc=0, 7.94 s | rc=0, 2.11 s |
| A3 `--json-schema` + `jq -er .verdict` | rc=4 | **rc=5 — not valid JSON** | **rc=0 — honoured** | **rc=5 — not valid JSON** |
| A4 pipe as context (one-sentence question) | rc=1 | rc=0, 0.86 s | rc=0, 2.11 s | rc=0, 4.08 s |
| A5 `--each` over three items | rc=1 | rc=0, 1.86 s | rc=0, 5.30 s | rc=0, 5.84 s |
| A6 `--tools`, cite a `file:line` | rc=1 | rc=0, 42.73 s — **fabricated** | rc=0, 133.45 s — **correct** | rc=0, 65.28 s — wrong |
| A7 truncation is a failure | rc=1 | rc=1 (correct) | rc=1 (correct) | rc=1 (correct) |

Cold rows (A1) include the server's model load, and A1/A2 for each model are
therefore *not* comparable with each other — see the warm A/B below.

**Warm reasoning A/B** (`omlx-extra.sh` E2: one call to warm the model, then two
calls per side) **[V]**:

| model | default | `--thinking off` | reading |
|---|---|---|---|
| MiniCPM5-2B-8bit | 0.84 s | 0.48 s | 1.8× — the flag does something |
| Qwen3.5-9B-4bit | 7.23 s | 5.92 s | 1.2× |
| gemma-4-E4B-4bit | 1.35 s | 1.37 s | no-op: this quantisation has no reasoning mode |

`--thinking off` sends `chat_template_kwargs.enable_thinking=false` **[P]**; oMLX
accepted it on all three models. On gemma-4-E4B it changes nothing, which is the
honest reading: a flag that the template ignores shows up as *no delta*, not as an
error.

### The only model that passed everything is the 5.6 GB one

`Qwen3.5-9B-MLX-4bit` is the practical default on this machine: it honoured the
JSON schema, followed the `--each` framing, used the tools, and got the citation
right. The 2B is 12× faster on a short answer (0.48 s vs 5.92 s warm) and 2–3× faster
on a real one, and is the right choice for mechanical work
where you can check the answer — it follows formats less reliably and invents
citations (§9). The practical split on a 16 GB Mac is exactly that: **a small fast
model for extraction and rewriting, a 4-bit 9B for anything that must be right.**

## 6. Use cases, each with what actually came back

### 6.1 Ask about text you already have (the pipe)

```sh
clank --thinking off -m 'in one sentence: what does the email anonymizer keep?' \
  < fixtures/notes.md
```

- MiniCPM5-2B-8bit, 0.86 s **[V]**: `The email anonymizer keeps only the domain part.`
- Qwen3.5-9B-4bit, 2.11 s **[V]**: `The email anonymizer keeps only the domain part of the email address.`
- gemma-4-E4B-4bit, 4.08 s **[V]**: `The email anonymizer keeps only the domain part of the email address in \`anonymize()\` within the \`userData\` struct (Notes).`

The input is exactly what was piped — invariant 3 **[P]** — so there is no question
about what the model saw, and nothing to re-create later.

### 6.2 Structured output you can gate on

`fixtures/verdict.schema.json` is a one-property schema with an enum. Raw answers,
no `jq` in the way **[V]**:

| model | stdout | clank exit |
|---|---|---|
| Qwen3.5-9B-4bit | `{ "verdict": "risk" }` | **0** |
| MiniCPM5-2B-8bit | `verdict: risk` | 1 — `final output is not valid JSON; --json-schema requested` |
| gemma-4-E4B-4bit | `The provided context does not contain information about an anonymizer's behavior …` | 1 |

An earlier gemma run returned the right JSON wrapped in a Markdown fence, and clank
failed it too. This is the caveat the README already records **[P]**: *a top-level
`json_schema` is the endpoint's grammar to enforce, not clank's.* On this box oMLX
honoured it for `Qwen3.5-9B-MLX-4bit` and ignored it for the other two. clank's
half of the contract held in every case: a violation is `exit 1`, never a
best-effort parse.

The gate is the shell's:

```sh
clank --thinking off --json-schema @fixtures/verdict.schema.json -m '…' \
  | jq -er .verdict && echo gate-passed
```

### 6.3 Map one prompt over many items (`--each`)

Items are the evidence, one conversation per item, serially, one framed answer
each **[P]**:

```sh
git log --format=%s -3 | clank --thinking off --each \
  -m 'one line: rewrite this commit subject in the imperative mood'
```

Observed **[V]** (Qwen3.5-9B-4bit, 5.30 s):

```
─── item 1/3 ───
readme: add install section and fix name clash

─── item 2/3 ───
use-cases §9: implement loop and goal in eight lines of shell

─── item 3/3 ───
readme: third shorter and remove essay voice
```

The framing is what makes this composable: `awk '/^─── item /{…}'` splits stdout
back into per-item results, and `--jsonl` carries `i`/`of` on every event **[P]**.

**The honest limit:** an item is *text*, not a file. Piping bare paths gives the
model nothing to read, and the answers are vacuous or a refusal — measured **[V]**:

```
$ printf 'src/main.rs\nsrc/tools.rs\nsrc/client.rs\nsrc/context.rs\n' \
    | clank --thinking off --each -m 'one line: what does this file do?'      # 1.93 s
─── item 1/4 ───
src/main.rs is the main source file for a Rust application.
─── item 2/4 ───
This file exists but its purpose is not described in the provided context.
─── item 3/4 ───
This file contains client code.
─── item 4/4 ───
This file is a Rust source file.
```

The same prompt with `find src -name '*.rs' -print0 | clank --each -0` (1.57 s)
produced the same vacuous class of answer. **If the items need to be read, the
shell reads them** — see 6.4.

### 6.4 Fan out for real — and measure it before you believe the fan-out

Four files, three shapes, same 2B model, same prompt **[V]**:

| shape | wall time | notes |
|---|---|---|
| `while read f; do clank -c "$f" …; done` (4 calls) | **5.84 s** | correct per-file answers, but concatenated without separators |
| `xargs -P2 -I{} clank -c {} …` (4 calls, 2 at a time) | **12.50 s** | 2.1× *slower*; output interleaves across items |
| one `clank --each` call with paths as items | **1.93 s** | fastest, and useless: the model never saw the files |

Two lessons from this box:

- **Parallelism above a single local engine makes it slower, not faster.** The
  engine batches what you send it; two clank processes just contend, and each one
  pays a cold prefill for its own prompt prefix. 12.50 s of parallel against
  5.84 s of serial is the measured cost of that mistake. (On a bigger machine or a
  server with separate instances, the arithmetic changes — measure, do not assume.)
- **Framing is a property of `--each`.** The loop's four answers ran together into
  one paragraph because a single-answer clank writes no trailing newline; the
  parallel run interleaved two answers mid-line. If a machine has to consume the
  output, use `--each`, or one process per output file.

One shell detail that costs an afternoon if you miss it: **clank reads stdin**, so a
`while read … < file` loop must give each call its own stdin (`</dev/null`) or the
first call eats the rest of the list.

### 6.5 Continue a run: a trace is context

```sh
clank --thinking off --jsonl --no-tools -m 'Name three grep options, one per line, option then a five-word description.' \
  > stage1.jsonl
clank --thinking off -m 'The context is a transcript of an earlier stage. Which of the options it listed is the one for case-insensitive matching? Answer in one line.' \
  < stage1.jsonl
```

Both stages, 3.84 s on Qwen3.5-9B **[V]**; stage 2 answered `The option for
case-insensitive matching is -i.` A `--jsonl` stream is not just a log: it re-ingests
as a rendered transcript **[P]**, so a two-stage pipeline needs no intermediate
format.

Note `--no-tools` on stage 1. With nothing piped, clank offers its read-only
observers by default **[P]** — and a general-knowledge question then gets answered
with *"I can't answer this request yet because I don't know what grep file to look
at"* **[V]**. When there is nothing to inspect, say so: `--no-tools` adds the
sentence that forbids citing what it never saw. Both modes are honest; they are
honest about different things.

### 6.6 A reusable instruction block

```sh
clank --thinking off --system @local/macbook/sys.md -m 'what is 2+2?'
```

with `sys.md` containing `Answer only with the word: banana.` → `banana`, 3.16 s,
`exit 0` **[V]**. One flag, one file in the repo, no framework, no stored session.

### 6.7 Let it look around (and read the breadcrumbs)

With nothing piped, the four read-only observers (`read_file`, `list_dir`,
`search`, `stat`) are offered, and every lookup lands on stderr **[P]** — the input
is still visible in the two-channel contract:

```sh
clank --thinking off --tools -m 'what is the per-file output cap in the transcript renderer? cite file:line and state the number.'
```

| model | time | answer | verdict |
|---|---|---|---|
| Qwen3.5-9B-4bit | 133 s | `400` at `src/context.rs:136`, quoting `const TRANSCRIPT_OUTPUT_CAP: usize = 400;` | **correct** (checked against the file: `src/context.rs:136` **[V]**) |
| MiniCPM5-2B-8bit | 43 s | `8000` at `src/renderer.ts:14` | **fabricated** — there is no `src/renderer.ts` |
| gemma-4-E4B-4bit | 65 s | `500 matches and 4 MB per file` (the *search* tool's caps) | wrong answer to the question asked |

The 2B's fabrication is the exact failure mode `PROTOCOL.md` records from an earlier
session, down to the invented `src/renderer.ts:14` **[P]** — the format of a real
answer with a plausible line number. What the harness gives you is not accuracy; it
is **auditability**: the stderr breadcrumbs show how many lookups happened, and two
lookups cannot support a citation the model made up. The answer was wrong, the run
was still cheap, and nothing in it pretended to be verified.

A second run of the same 2B with the same question did answer correctly
(`400`, `src/context.rs:136`) in 38 s over ten tool calls **[V]** — the failure
is stochastic, which is the reason to keep the exit code and the trace, not to trust
one sample.

### 6.8 Stop reading when you have enough (SIGPIPE)

```sh
clank --thinking off -m 'count from one to fifty, one number per line' | head -1
```

Measured **[V]**: full answer 6.83 s / 49 lines; with `head -1`, **0.80 s**, pipeline
`rc=141` (SIGPIPE). clank restores the default SIGPIPE disposition **[P]**, so it
dies with the pipe instead of generating tokens nobody will read. On a machine
where every token costs 20–50 ms, this is one of the two or three genuinely useful
optimisations — the shell's own control flow, not a feature the harness had to add.

### 6.9 Review a diff (and see the budget rule fire)

```sh
{ git show --stat HEAD; git show HEAD; } | clank --thinking off --max-tokens 300 \
  -m 'The context is a git diff. One line per issue, most important first. If there is no issue, say none.'
```

- `--max-tokens 300`: 26.59 s, **`exit 1`** —
  `clank: answer truncated at 300 tokens (--max-tokens); raise it or narrow the prompt`.
  The partial answer on stdout was already useful; the exit code said out loud that
  it was not finished.
- `--max-tokens 700`: 8.69 s, `exit 0`, answer `none`.

Same input, same model, 3× the time for the truncated run. A stage that reports
success it cannot back is worse than one that fails **[P]** — and here "success"
would have been a partial review that a `&&` chain would have happily shipped.

### 6.10 The four-stage pipeline (`demo.sh`)

`demo.sh` is the repo's reference pipeline: generate a script, critique it against
the fixture *and the shell's own measurements*, finalize it under `--json-schema`,
then have the model propose a command that verifies the result. Every stage is
gated; model output is syntax-checked and never executed. Against this box **[V]**:

| model | result |
|---|---|
| `Qwen3.5-9B-MLX-4bit` | **all four stages passed in 34.7 s** (`bash -n` clean on both generated scripts; the final one-liner: `bash local/demo-traces/final.sh … \| grep -q '<html.*Notes' && echo VERIFIED`) |
| `gemma-4-E4B-it-MLX-4bit` | failed at stage 3: the schema-constrained request came back fenced, clank `exit 1`, demo stopped. Stages 1–2 had already produced a one-line candidate that the critique flagged |
| `MiniCPM5-2B-MLX-8bit` | failed at stage 1: `answer truncated at 8192 tokens` — the 2B rambles until the budget is gone. clank `exit 1`, nothing downstream ran |

Three models, three different failures, zero false "passed" lines. That is the
whole thesis in one table: on a small machine with small models, **the gates are
the product**.

## 7. The failure contract, as observed here

Every failure below is a real run from this session **[V]**:

| command | exit | stderr |
|---|---|---|
| `--model no/such-model` | 1 | `model returned HTTP 404 Not Found: … Available models: MiniCPM5-2B-MLX-8bit, Bonsai-2-27B-CRACK-1.75bit-JANG, Qwen3.5-9B-MLX-4bit, gemma-4-E4B-it-MLX-4bit` |
| `--model Bonsai-2-27B-CRACK-1.75bit-JANG` | 1 | `model returned HTTP 409 Conflict: … unavailable after a previous load failure: LM load failed (force_lm=True): Received 402 parameters not in model: language_model.lm_head.signs, …` |
| `--max-tokens 1` with a counting prompt | 1 | `answer truncated at 1 tokens (--max-tokens); raise it or narrow the prompt` |
| `--thinking sometimes` | 2 | usage (unknown level) |
| model emits nothing after a tool result | 1 | `no answer: the model returned no text and called no tool` |
| `--json-schema` with a fenced answer | 1 | `final output is not valid JSON; --json-schema requested` |

The 409 is worth reading twice: **the server's own words are passed through
verbatim.** clank did not translate a broken MLX model into "model error"; it gave
the operator the 402 parameter names, which is what you need to fix the model.
`Bonsai-2-27B-CRACK-1.75bit-JANG` is listed by `/v1/models` and cannot be loaded by
this runtime at all — a trap for any harness that trusts the model list, and a
one-second diagnosis here.

## 8. Why this makes local models usable on a memory-constrained Mac

1. **The harness is 4 MB; the model is 2.6–6.7 GB.** Nothing this side of the
   endpoint competes for the 16 GB.
2. **Nothing persists, so the server owns residency.** clank holds no session, no
   cache, no daemon **[P]**, so oMLX is free to evict, reload and throttle between
   calls (§2). The cost is visible where it should be — in that call's latency:
   ~2 s to swap the 2B, ~5–7 s for the 4-bit 9B, ~9.5–11.6 s for gemma.
3. **One model at a time is a fine workflow when every call is a stage.** Small,
   checkable calls to the 2B (0.48 s warm) for mechanical work; the 9B (5.9–7.2 s
   warm) when the answer has to be right. No routing layer, no orchestration
   framework; the model name is a flag.
4. **Failures are cheap and loud.** Truncation, emptiness, a bad schema, a wrong
   model: all `exit 1` with the reason on stderr. On a machine where a 2B model
   *will* invent `src/renderer.ts:14`, an honest exit code is the difference
   between a pipeline and a wish.
5. **Composition replaces capability.** The four-stage demo got a usable artifact
   out of a 9B 4-bit model because the shell gathered the evidence, gated each
   stage, and never trusted the previous answer. That is the trade a constrained
   Mac makes: more stages, smaller stages, every one of them checkable.
6. **The interfaces are the same ones the rest of the machine speaks.** Pipes,
   exit codes, `jq`, `xargs`, `head`. Nothing here needs a plugin, a session file,
   or a service kept alive to hold a conversation open.

## 9. What did not work, and the limits of this record

- **`Bonsai-2-27B-CRACK-1.75bit-JANG` is unusable on this runtime** — the server
  refuses to load it (402 extra parameters). Not a clank problem, but on this box
  that model is listed and dead **[V]**.
- **Two of three models ignore `--json-schema`.** On this oMLX build the grammar is
  honoured for `Qwen3.5-9B-MLX-4bit` and ignored for `gemma-4-E4B-it-MLX-4bit` and
  `MiniCPM5-2B-MLX-8bit` **[V]**. Treat schema enforcement as a property to
  *measure per endpoint*, exactly as the README already warns **[P]**.
- **`--tools` is unreliable below ~9B.** One correct answer, one fabrication, one
  misread question (§6.7). `--tools` is documented as a convenience, never a
  substitute for a pipe **[P]**; these numbers are the reason.
- **Parallel fan-out is a pessimisation against one local engine** (12.50 s vs
  5.84 s, §6.4). Local-first hardware does not reward `xargs -P` the way a remote
  API does.
- **`--each` items are text, not files.** Bare paths produce vacuous answers
  (§6.3). The shell must hand over what it wants summarised.
- **Memory guard throttling is visible at the edge.** Server log, during this
  session **[V]**: `Paused request … for prefill LRU eviction (reason=adaptive_prefill_throttle)` →
  `No idle model evicted …; scheduler will fall back to throttling`. Long prompts on
  a machine already at 7 % free can be slowed deliberately rather than fail. It is
  the server's behaviour, not clank's, but a pipeline's latency budget has to
  include it.
- **Single machine, single session, one quantisation each.** Every number here is
  one run on one M1 Pro with oMLX 0.7.0.dev4 and clank 0.1.0. The A6-style failures
  are stochastic (the same 2B answered the same question correctly on another run),
  so treat individual cells as samples, not as model properties.
- **No sandbox.** `--tools` bounds what the model can *do* (observe), not what it
  can *reach*: with your permissions it can read any path you can read **[P]**.

## 10. Reproducing this

```sh
cd clank
cargo install --locked --path .        # or: cargo build --release
cargo test                             # 24 tests, no model, no network

source local/macbook/omlx-env.sh Qwen3.5-9B-MLX-4bit

./local/macbook/run-tests.sh                    # 17-test contract battery
./local/macbook/omlx-matrix.sh                  # every served model × 7 tests
./local/macbook/omlx-extra.sh                   # A/B, SIGPIPE, footprint, fan-out, endpoint throughput
./local/macbook/omlx-usecases.sh                # the pipelines in §6
CLANK_MODEL=Qwen3.5-9B-MLX-4bit ./demo.sh       # the four-stage reference pipeline

./local/macbook/measure-server.sh               # endpoint throughput WITHOUT clank
```

Each script writes raw out/err/rc/secs evidence under `local/macbook/` and prints
where it landed. `measure-server.sh` exists so the throughput numbers can be
attributed to the machine and the engine rather than to the harness in front of it:
measured on this box, without clank **[V]** —

| measurement | result |
|---|---|
| decode, 2B 8-bit, 300 tokens | **54.4 tok/s** |
| prefill, 4,531 prompt tokens | **660 tok/s** (6.86 s wall) |

## Verification

- **[V]** Everything in this document marked as measured: 17-test battery, 4×7
  model matrix, raw schema answers, warm reasoning A/B, SIGPIPE, fan-out timings,
  `/health` and log-derived residency/load/eviction numbers, `measure-server.sh`
  throughput, the four-stage demo runs, and the exit codes in §7.
- **[P]** Taken from this repo's docs rather than re-derived: the invariants,
  the exit-code contract, `--thinking off`'s wire form, the `--tools` doctrine, the
  `json_schema`-is-the-endpoint's caveat, the recorded `src/renderer.ts:14`
  hallucination, and the "no sandbox" statement.
- **[U]** Not verified: behaviour on other Apple silicon, other oMLX versions, other
  quantisations; whether any of these failures reproduce on a second run (the 2B's
  did not); and load-time numbers under a warmer or busier machine.
