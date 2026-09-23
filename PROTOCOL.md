# clank protocol

clank is one step in a pipeline: it reads what you pipe, answers once, and
writes the answer to stdout. The shell does the searching, the fan-out, the
checking and the iterating. The pipeline is the agent; clank is the model-shaped
stage inside it.

## The invariants

| # | invariant | why |
|---|---|---|
| 1 | one invocation = (prompt, context) → answer; nothing persists | no session files, no daemon, no cache clank owns; a pipeline re-runs and has no hidden state to inspect |
| 2 | the only inter-stage interface is text: stdout, a JSON context tree, a JSONL trace | anything that writes text can feed clank, anything that reads text can consume it |
| 3 | **the context is exactly what you piped**; the tools are the fallback only when nothing was piped | a stage that also reads the filesystem on its own has an input you cannot see in the pipeline. With no pipe there is no boundary to protect and no evidence to hand over |
| 4 | the capability boundary is observation: no writes, no shell, no network beyond the model endpoint | clank proposes; execution is the shell's or the human's. A read-only stage cannot damage the tree it reasons about |
| 5 | no concurrency inside the process | serial `--each` keeps stdout ordered and the mapping unambiguous; parallelism is `xargs -P`, above clank |
| 6 | stdout is data, stderr is diagnostics, exit codes are `0` / `1` / `2` | `clank … \| jq` and `clank … \| head` work because nothing else is on stdout |

## The lookups belong to the shell

| you want the model to | compose it |
|---|---|
| read a file | `cat f \| clank -m "…"`, `sed -n '10,40p' f \| clank -m "…"` |
| search | `rg -n "pat" \| clank -m "…"`, `rg -n "pat" . \| clank --each -m "one line: is this match a problem?"` |
| search the web | `clank-web "query" \| clank -m "summarize with citations"` |
| read one page | `clank-web --fetch URL \| clank -m "…"` |
| see what changed | `git diff \| clank -m "…"` |
| look twice / follow up | `clank --jsonl -m "…" \| clank -m "continue"` |
| fan out over N things | `rg -l … \| clank --each`, `find … -print0 \| clank --each -0` |
| act on the answer | `clank … \| jq -er .script > s.sh && bash -n s.sh`, then run it yourself |
| chain stages | the shell — see `demo.sh` |

An `--each` item is **text, not a file**: `rg -l … | clank --each` hands the
model path strings, so either ask something a path can answer, or let the shell
read the file (`-c` per item, `</dev/null`). Measured 2026-09-19 on oMLX: asked
to summarize paths, the models answered *"This file exists but its purpose is
not described in the provided context."*

`--tools` covers the one lookup the pipe did not, inside a single stage. A
search by the model's own tool is a worse `rg` with an invisible input, which is
what invariant 3 rules out: one stage is for one lookup, and more than that
wants a pipeline.

With nothing piped, no `-c` and no items there is no evidence to hand over, and
answering from priors means citing files that were never read: measured
2026-09-17, a no-context question produced a confident `src/renderer.ts:14` for
a file that does not exist — the format of a real answer, with a real line
number, exit 0. So with no evidence supplied the tools are offered by default,
and every lookup lands on stderr, so the model's input stays visible in the
two-channel contract. `--no-tools` forces the blind case, and then the prompt
carries one extra sentence: the context is empty, answer from what you know, and
never cite a file or line you were not given.

Neither mode is a sandbox: clank runs with your permissions and `read_file`
reaches any path you can read — an absolute path, `~/.ssh`, `/etc`. `--tools`
bounds what the model can *do* (observe, nothing else), not what it can *reach*.
The pipe bounds the input; for containment, run clank as a user that cannot read
what you are protecting.

## The request sequence

| mode | requests sent |
|---|---|
| default, something piped | **one** request, one answer, no tools |
| default, nothing piped or `-c`'d | one request, with the read-only tools offered |
| `--json-schema` | one request, carrying the schema |
| `--tools` | tool rounds until the model stops calling tools; that text is the answer |
| `--tools --json-schema` | tool rounds (no schema), then **exactly one** request carrying the schema and no tools |
| `--each` | the above, once per item, serially |

**Tools and the schema are never in the same request.** This server build
rejects the pair (400, verified with a bare `curl` probe), and the loop's
intermediate text is not an answer, so constraining it would constrain the wrong
thing. The constrained request is the only place the schema is enforced, by the
server: given a schema whose only legal value is `alpha` and an instruction to
reply `beta`, both endpoints answered `{"word":"alpha"}`. The grammar overrides
the instruction, which is also why the constrained request is not skipped when
the model has already produced JSON-looking text. While a schema is pending,
intermediate commentary is **not** emitted: stdout carries exactly the answer.

## Reasoning

Thinking is on by default in both local templates, and its tokens are billed
whether or not anyone looks at them. clank makes it controllable and visible,
and does nothing else with it:

| flag | what it sends | what it does |
|---|---|---|
| *(none)* | nothing | the server's default (today: thinking on) |
| `--thinking off` | `chat_template_kwargs: {"enable_thinking": false}` | disables thinking through the template |
| `--thinking <level>` | `reasoning_effort: <level>` | `minimal`, `low`, `medium`, `high`, `xhigh`, `max`; the template decides whether it can honour the level, and an unsupported level is the server's error to report, not clank's to guess at |
| `--show-thinking` | — | streams the model's reasoning to **stderr** as it arrives; stdout keeps exactly the answer |

`--thinking off` is the portable switch: it disables thinking through the
template on both local endpoints. An effort level is advisory — one endpoint
honours it, the other accepts the field and ignores it, so a level is never a
guarantee.

clank does not parse, store or replay reasoning: a trace is answers and tool
calls, not thoughts.

### The system prompt is a contract, not a style guide

It states what the model may assume (the context is everything it has) and what
it may not do (run commands, change files, narrate actions it did not take),
plus how to answer (from the context, or say in one line that the answer is not
there). It does **not** attempt to shape tone, length or formatting: every added
rule is harness influence the reader can no longer attribute, and wording
effects are not measurable through clank. Sampling belongs to the serving stack
(these endpoints run at temperature 1.0 and 0.7, with no per-request knob), and
an A/B of one brevity directive came out with a different sign in different
samples. Task-specific instructions belong in `--system @file`, where they are
per-invocation, versioned as files, and visible in the trace through
`run.prompt`.

## Context doctrine

| channel | authority | says what |
|---|---|---|
| pipe (`rg … \| clank`) or `-c FILE` | **evidence** | exactly what this stage was given; may be a transformation that no longer matches the disk |
| the prompt (`-m` / positional) | the question | what to do with the evidence |
| `--tools` | exploration, opt-in | the lookup the evidence did not cover — logged on stderr, so it is visible in the two-channel contract |

1. If the pipe *is* the object of the request, say so in the prompt ("the
   context is the output of …").
2. When the pipe and the disk disagree, the pipe wins: it may be a `sed`, a `jq`,
   or another model's output.
3. Claims about code cite `file:line`, so the next stage can check them with the
   same evidence.
4. A pipe that reaches the process is never dropped: `-c` nodes come first, then
   the piped text as the last node. If both are present, both are sent.
5. **A pipe may carry any shape.** A clank trace is recognised exactly — every
   line an object whose `type` is one of the six events — and rendered as a
   transcript; *anything else*, including JSON that happens to have a `type`
   field of its own, is handed to the model as text. The pipe is evidence, so it
   parses leniently and cannot fail. `-c FILE` is configuration someone wrote on
   purpose, so a node-shaped mistake there is an error instead of a silent
   fall-back.

Where stdin goes:

| invocation | stdin is |
|---|---|
| `clank` alone (no `-m`, no positional) | the prompt |
| `clank -m TEXT` with a pipe | a context node, after any `-c` nodes |
| `clank --each` | the item list — and then the prompt must come from `-m`/positional |

## Output contract

Text mode, one prompt: the answer, and nothing else.

Text mode, `--each`: each frame is newline-terminated, frames are separated by
one blank line, and there is no blank line after the last one, so
`awk '/^─── item /{…}'` splits it even when the model's own answer did not end
its line. The item header is the same string the model was shown, so an answer
always maps back to its input.

```
─── item 1/3 ───
<answer>

─── item 2/3 ───
<answer>
```

`--jsonl` replaces deltas and frames with one event per line. The event set is
closed, and every event of an item additionally carries `i` (1-based) and `of`:

| event | fields |
|---|---|
| `run` | the **first line**: `clank`, `prompt`, `model`, `base_url`, `tools`, `thinking`, `argv` (an API key on the command line is redacted). `prompt` is a short id for the *effective* system prompt — the built-in one alone, or one with a `--system` skill appended — so two traces stay comparable across a prompt change |
| `item` | `input` — emitted once, before an item's work |
| `tool_call` | `name`, `arguments` (with `--tools`; parsed object, `null` if the model emitted invalid JSON) |
| `tool_result` | `name`, `ok`, `output` (with `--tools`) |
| `assistant` | `content` — the answer |
| `error` | `message` — that item failed; the run continues |

Framing is **LF-only**: one JSON object per line, split on `\n` and nothing
else. Do not read these streams with a line reader that also splits on Unicode
separators — Node's `readline` breaks on `U+2028`/`U+2029` inside JSON payloads
and corrupts the stream; `jq -c` and Rust's `lines()` are correct. A trace that
begins with a `run` event feeds back in as context unchanged: provenance is not
part of the transcript.

Exit codes:

| code | meaning |
|---|---|
| `0` | every prompt answered, every item succeeded |
| `1` | model/server/IO failure, a **truncated** answer (the server hit the token budget), an **empty** answer, a non-JSON answer under `--json-schema`; with `--each`, **any** item failed, even though the others were answered; an **internal** failure, where a value clank built itself did not serialise |
| `2` | usage: bad flags, bad schema, no prompt, `--each` without a prompt |

Truncation and emptiness are failures, not results: a pipeline stage that
reports success it cannot back is worse than one that fails. A model that spends
its whole budget thinking emits neither text nor a tool call, and that is not an
empty answer — it is no answer.

A failed `--each` item does not stop the run, so its failure has to be loud
somewhere: it goes to stderr, it becomes an `error` event, and it sets the exit
code. An empty item list is not a failure: `rg -l … | clank --each` legitimately
matches nothing, so stdout stays empty and the exit code stays `0`.

## Item placement and prompt reuse

With `--each`, every message except the trailing item block is byte-identical
from item to item, and the item block goes last for exactly that reason: the
server's prompt-prefix cache can then reuse the whole shared prefix, and only
the item itself is new work.

## The proposal boundary

clank never executes what the model says. The pattern that makes that useful is
two stages with a mechanical check in between:

```sh
clank -m 'emit a bash script as JSON' \
  --json-schema '{"type":"object","properties":{"script":{"type":"string"}},"required":["script"]}' \
  | jq -er .script > candidate.sh
bash -n candidate.sh &&  # syntax gate
  bash candidate.sh      # a human decided to run it
```

`jq -e` and `bash -n` are the parts a model cannot talk its way past. Never
`eval $(clank …)` and never `clank … | sh`: the model's output is data, and the
only thing between data and an executed command must be a gate someone can read.

The same pattern generalises: the model can **write a jq filter** and the shell
runs it.

```sh
clank -m 'write a jq filter that keeps elements with a "children" key' \
  --json-schema '{"type":"object","properties":{"filter":{"type":"string"}},"required":["filter"]}' \
  | jq -r .filter > proposed.jq
cat proposed.jq           # read it: a filter can read files
jq -c -f proposed.jq data.json || clank -c proposed.jq -m 'that filter failed; fix it'
```

jq's own exit code and stderr are the check, and the retry feeds the failure
back as context. No tool schema, no code execution inside clank: a proposal, a
gate, and a loop the shell owns.

## Why the decision stage is a separate binary

`clank-jev` (in this crate, a second `[[bin]]`) asks typed questions about a
state and returns typed answers with probabilities, for routing in scripts. It
is deliberately *not* a clank flag:

| rule | why a `--use-jev` flag would break it | why the sibling binary does not |
|---|---|---|
| R1 composition first | it would add a mode to a tool whose value is being one stage | it is its own stage, composable with clank and without it |
| R3 two channels | the same, unchanged — stdout data, stderr diagnostics, exit codes | unchanged: value on stdout, diagnostics on stderr, 0/1/2/3 |
| R4 flags not memory | credentials would sit in clank's flag surface | credentials come from the environment only, never argv |
| R6 output readable without clank | a Jev answer is only useful to a caller that knows the contract | the JSON is the contract, and one question prints a bare value |
| no second wire protocol | clank speaks `/v1/chat/completions` and nothing else; a second provider inside it is a second source of truth | the second protocol lives in a binary whose entire job is that protocol |

The sibling inherits clank's shape: one request per invocation, no state on
disk, a closed answer space, exit codes that never lie, and a failure (a skipped
question, a missing probability) that fails the gate instead of passing by
default. The reason pattern — a closed-choice reason decided in the same request
as the value — is taken from `seanperkins/omp-jev-watchdog`; the vocabulary
lives in the checks file, not in the binary.

## Why web search is a separate binary

`clank-web` (a third `[[bin]]` in this crate) runs one search query, or fetches
one URL, and writes result lines. It is deliberately not a clank tool, for the
same reasons the decision stage is not a flag:

| rule | why a web tool inside clank would break it | why the sibling binary does not |
|---|---|---|
| R1 composition | the shell already composes a search with a summary | `clank-web "query" \| clank -m "summarize with citations"` is the whole feature |
| invariant 3 | a tool round would read the network on an input the pipe cannot show | the query is the invocation, and the results are what the next stage reads |
| invariant 4 | clank's network is the model endpoint | the search APIs live in the binary whose job is those APIs |
| R4 no memory | a search key would become a clank tool setting | `.clank/config.toml` is optional and shared. `clank` and `clank-jev` read `[clank]`. `clank-web` reads `[web]`. A missing file in the working directory leaves flags and the environment in charge |

Discovery is the working directory. Each binary loads `./.clank/config.toml`
when that file exists, and does not search parent directories. `--config PATH`
names a file. `CLANK_CONFIG` names one when the flag is absent. An explicit path
that is missing is usage (exit `2` on `clank`, `clank-jev`, and `clank-web`). A
missing `./.clank/config.toml` is not an error. Value precedence stays flags,
then the environment, then the file, then built-ins. The key is the environment
variable named by `api_key_env`. The sample at `fixtures/clank.config.toml`
names variables and does not carry a key.

`clank --list-tools` stays the four read-only filesystem observers. One
invocation is one request: no crawl, no JavaScript, no second fetch of a link
the page named. `--fetch` is that one GET.

Exit codes are `0` / `1` / `2`, the same classes as clank, and not `clank-jev`'s
`3`:

| code | meaning |
|---|---|
| `0` | the search or fetch completed and the results were written, including an empty result set |
| `1` | the request did not complete: network, an HTTP error, a missing key, or a body that is not the provider's JSON |
| `2` | usage: bad flags, an empty query, an unknown provider, a config file that does not parse |

An empty result set is a completed search: the stage's job was to return what
the provider returned. stdout is JSONL (`title`, `url`, `snippet`), or
`--format text` lines of those three fields separated by tabs. The key is the
environment variable named by `[web.brave]` or `[web.tavily]`. It is never an
argument and never printed. The shape is in
[docs/clank-web.md](docs/clank-web.md).

## Admission rules

Composition is the criterion. Before adding anything, answer these in order:

| rule | question | if yes/no |
|---|---|---|
| R1 composition | could the shell do this by running two things? | then it is not clank's job |
| R2 single shot | does it survive as (prompt, context) → answer? | if not, it is a different tool |
| R3 two channels | is it stdout data, a stderr breadcrumb, or an exit code? | if it needs a third channel or an interaction, no |
| R4 no memory | is it per-invocation intent? | then a flag; if clank would have to remember it, no |
| R5 never lie | does its absence make clank claim a success it cannot back? | then it is contract, not a feature, and it costs what it costs |
| R6 substitutable | can the output be read without clank? | if not, no |

## What does not belong in clank

| want | where it goes |
|---|---|
| searching, reading, listing for the model | the pipe — `rg`, `cat`, `sed`, `git diff` (see the table above) |
| web search, or fetching one URL | `clank-web`; clank's tools stay on the local filesystem |
| fan-out over many inputs, in parallel | `xargs -P` / `parallel`, above clank |
| retries | `until` / a wrapper in the shell, or the `error` events as a retry list |
| state across runs | re-feed the trace (`-c trace.jsonl`) or a file the shell keeps |
| writing, editing, running commands | a different process with a real sandbox |
| multi-stage orchestration | the shell (`demo.sh` is the reference) |
| scheduling, running unattended | a timer (systemd, or cron) above clank |
| context compaction | a pipeline stage of its own, or a `-c` tree you curate by hand |

## Cost

Every request has its own `--timeout`, and a stage makes as many requests as its
mode needs (see the request sequence), so wall clock is
`requests × per-request time`. `--tools` costs a round trip per lookup; the
default costs exactly one.

Per-request token accounting is deliberately **not** clank's: the server already
reports it (its log, and `--log-jsonl` for machines), and duplicating a number
that already has a home is how a second source of truth starts. `--quiet`
silences the breadcrumbs. `CLANK_DEBUG=req.json` writes the exact body of the
first request, which is how you see what the model was actually told. Counting
rounds from a `--jsonl` trace is the cheap cost model:
`clank --jsonl … | jq -s 'map(select(.type=="tool_call")) | length'`.

## How this is checked

`tests/wire.rs` runs the real binary against a stub server and asserts on the
request bodies and on stdout: that the default is one request with no tools,
that a schema and tools never share a request, that the pipe is still context
when `-c` is used, that a truncated or empty answer exits `1`, that `--each`
frames one answer per item and shares the prompt prefix, that a failed item is
reported without hiding the others, and that the exit codes match the table
above. `tests/web.rs` drives `clank-web` against a stub and checks the request,
the JSONL, and exits `0`, `1` and `2`. `tests/docs.rs` guards this document
against drift: every `clank` flag must appear in `README.md` and `CHEATSHEET.md`,
every `clank-jev` flag in `README.md` and `docs/clank-jev.md`, every `clank-web`
flag in `README.md` and `docs/clank-web.md`, every event type listed in
`src/context.rs` must have a row in the event table, and every file this repo's
docs link to must exist. Live model behaviour is recorded under
[docs/history/](docs/history/README.md).
