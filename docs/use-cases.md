# Use cases

clank is a stage, not an assistant: you hand it text, it hands back an answer on
stdout, and the shell decides what happens to that answer. Everything below is a
*task* built out of that, with the gate that tells you whether it worked and the
price you pay for it.

Every command with a concrete input was run against the local endpoints on
2026-09-17 (`:40583` qwen3.8-27b unless noted, `--thinking off`), and the
observations quoted are its output. Blocks containing `…` placeholders illustrate
a rule rather than a runnable line; the herdr block is marked "syntax verified,
not executed" with the reason.

Prices come from the measured cost model (`research-harness-constraints.md` §5.2):

```
one call ≈ 0.25 s + 0.02 s × output tokens
         + prompt tokens at 1.3 ms cold / 0.02 ms cached
```

## Nothing to pipe

The pipe is the evidence — but sometimes the question *is* about the workspace, and
there is nothing to hand over. clank treats that as its own case: with no stdin, no
`-c` and no `--each` items, the model is offered the four read-only observers, and
every lookup it makes lands on stderr where you can see it.

```sh
clank --thinking off -m "where is the transcript cap defined, and what enforces it? cite file:line"
```

*Verified:* that question, with nothing piped, answered `src/context.rs:68`,
`TRANSCRIPT_OUTPUT_CAP = 400` at `src/context.rs:136`, and the call site at `:112`,
after three logged lookups. The same question **without** the tools answered
`src/renderer.ts:14`, cap 8000 — a file that does not exist, in the exact citation
format of a real answer, exit 0. That is why the fallback exists.

When you *want* the blind case — a self-contained question, a deterministic request,
a benchmark — force it, and the prompt stops inviting citations it cannot back:

```sh
clank --no-tools -m "write a regex that matches ISO-8601 dates"
# → No context was provided, so I cannot cite a `file:line`.
clank --no-tools --json-schema @extract.schema.json -m "pull the fields out of this" < input.txt
```

The rule in one line: **something piped → that is the evidence and nothing else;
nothing piped → it may look, and the lookups are visible.**

## 1. Ask about text you already have

**Explain what a search matched** — the pipe is the evidence, so the model never
opens a file you did not hand it.

```sh
rg -n -C3 "userData" src/ | clank --thinking off -m "what does this code do? cite file:line"
```

*Cost:* 1 call, ~0.3 s. *Gate:* you read it.

**Review a diff before committing.**

```sh
git diff HEAD~1 -- src/context.rs | head -70 \
  | clank --thinking off --max-tokens 400 \
      -m "Review this diff. One line per issue as file:line: problem. If none, output exactly: no issues."
```

*Cost:* 1 call; **give a review real budget** — this one truncated at 160 tokens
and exited 1, which is the contract working: a truncated review is not a shorter
review, it is a failed run. *Gate:* exit 0 and a human reads the lines.

**Explain a failure from the command that produced it.**

```sh
make 2>&1 | tail -40 | clank --thinking off --max-tokens 200 -m "root cause, then the one-line fix"
```

*Cost:* 1 call. *Gate:* exit 0. The model cannot run anything — no `--tools`, no
shell — so a fix is a proposal, which is the next section.

## 2. Structured output you can act on

**Extract fields from evidence.** The schema is enforced by the server's grammar,
not by hope: in testing, a schema whose only legal value was `alpha` overrode an
explicit instruction to answer `beta`.

```sh
rg -n "unwrap|expect\(" src/ \
  | clank --thinking off --json-schema @schema-findings.json -m "Extract the findings." \
  | jq -er '.findings[] | "\(.file):\(.line): \(.issue)"'
```

*Cost:* 1 call. *Gate:* `jq -er` fails the pipeline if the shape is wrong, and
clank exits 1 if the answer is not JSON at all.

**Triage a list, one schema'd answer per item.**

```sh
printf 'src/context.rs\nsrc/tools.rs\nREADME.md\n' \
  | clank --each --thinking off --max-tokens 60 \
      --json-schema '{"type":"object","properties":{"kind":{"type":"string","enum":["code","docs"]}},"required":["kind"]}' \
      -m "Classify this path as code or docs. Answer with JSON."
```

```
─── item 1/3 ───
{"kind": "code"}
─── item 2/3 ───
{"kind": "code"}
─── item 3/3 ───
{"kind": "docs"}
```

*Cost:* one call per item (0.25 s each). *Gate:* frames map answers to inputs;
`--jsonl` + `jq` for machine use.

**Grow the schema with clank itself.** Ask for a schema, check it is JSON, then
feed it back from disk.

```sh
clank --thinking off --max-tokens 400 \
  -m 'Design a JSON Schema, draft-07 without $schema, for extracting review findings:
      an object with "findings", an array of {file, line, issue}. Output ONLY the schema.' \
  > schema-custom.json
jq -e . schema-custom.json                # gate: it must be valid JSON
rg -n "TODO" src/ | clank --json-schema @schema-custom.json -m "Extract the findings."
```

*Verified:* produced a 237-byte schema that then extracted 5 findings from real
`rg` output. Note the `@` — `--json-schema @file` and `--system @file` read their
payload from disk, which is what makes this loop possible without editing clank.

## 3. Map over many things

**Summarize everything that changed.**

```sh
git diff --name-only | clank --each --thinking off --max-tokens 120 -m "one-line summary of this file"
```

*Cost:* 0.25 s + output per item; 20 items ≈ 8 s serially.

**Tolerate failures and get a retry list.** A failed item does not stop the run,
and it does not disappear either.

```sh
printf 'a\nb\nc\n' | clank --each --jsonl --max-tokens 1 -m "Say something long." > run.jsonl
echo $?                                   # 1 → something failed, the run still finished
jq -r 'select(.type=="error") | .i' run.jsonl   # 1 2 3 → exactly what to retry
```

*Verified:* exit 1 with three `error` events and `1 2 3` recoverable from the
trace. *Gate:* the exit code for "did all of it work", the events for "which part
did not".

**Batch instead of mapping when the model is slow.** One call with a list beats N
calls when each item is small: the prompt is processed once and the fixed
per-call cost is paid once.

```sh
printf 'src/context.rs\nsrc/tools.rs\nREADME.md\n' | clank --thinking off --max-tokens 120 \
  --json-schema '{"type":"object","properties":{"items":{"type":"array","items":{"type":"object",
     "properties":{"path":{"type":"string"},"kind":{"type":"string"}},"required":["path","kind"]}}},"required":["items"]}' \
  -m "The context is a list of paths. Return one entry per path with its kind."
```

*Verified:* 3 entries from 1 call. Measured at four items: serial `--each` 2.4 s,
`xargs -P2` 1.5 s, one batched call 1.6 s — and the gap widens with N.

## 4. Keep it fast

The model is the constraint, so the lever is the number of tokens you pay for:

| rule | why | measured |
|---|---|---|
| `--thinking off` for mechanical work | thinking is billed and generated before any answer | 1.35 s → 0.25 s |
| keep the prompt head stable, vary the tail | cached prefixes are ~60× cheaper to process than cold ones | 10.4 s → 0.21 s |
| stop reading when you have enough (`\| head -1`) | the call dies with the pipe instead of finishing | 7.1 s → 0.6 s |
| prefer one call with many items over many small calls | the 0.25 s fixed cost is paid per call | 1.5× on 4 items |

```sh
rg -l "TODO" src/ | clank --each --thinking off -m "one-line summary"     # rule 1
clank -c big-cached-context.json -m "…"                                   # rule 2 (same file each time)
clank -m "the first line only matters" | head -1                          # rule 3
printf '%s\n' a b c | clank --json-schema @list-schema.json -m "…"        # rule 4
```

## 5. Wire it into the environment

**Write atomically, gate on the answer.** A redirect makes a failed run look like
a file of results, so check before you keep it.

```sh
clank --thinking off --json-schema '{"type":"object","properties":{"verdict":{"type":"string"}},"required":["verdict"]}' \
  -m "Verdict on this text, one word." <<< "the build is green" > out.tmp \
  && jq -e .verdict out.tmp \
  && mv out.tmp out.json
```

*Verified:* `"pass"`, kept as `{"verdict": "pass"}`; on failure nothing is kept.

**An append-only run log.** Every `--jsonl` stream opens with a `run` event
(model, endpoint, argv), so a file of many runs stays readable.

```sh
clank --thinking off --jsonl -m "Reply with exactly: one" >> log.jsonl
clank --thinking off --jsonl -m "Reply with exactly: two" >> log.jsonl
jq -c 'select(.type=="run") | .clank' log.jsonl          # two runs, delineated
clank -c log.jsonl -m "what have I asked so far?"        # the log is context
```

**A socket, a FIFO, a file** — clank writes to stdout, so the destination is
yours:

```sh
socat UNIX-LISTEN:/tmp/out.sock - > received.txt &        # verified: "pong"
clank --thinking off -m "Reply with exactly: pong" | socat - UNIX-CONNECT:/tmp/out.sock

mkfifo /tmp/p && clank --thinking off -m "Reply with exactly: fifo" > /tmp/p &   # verified
cat /tmp/p
```

A FIFO is a rendezvous, not a buffer: the writer blocks until a reader opens.
Writes below 4 KB are atomic; keep to **one writer per FIFO**.

**Whatever is on a tmux pane.**

```sh
tmux capture-pane -p | clank --thinking off -m "one line: what is wrong?"
```

*Verified* against a live pane: *"ERROR: disk quota exceeded at /var/log"* →
*"The disk quota at /var/log has been exceeded."*

**A neovim buffer as the pipe.** `%!` replaces the buffer with the command's
stdout, so the answer lands in the file.

```vim
:%!clank --thinking off --max-tokens 60 -m "Add a comment line after the shebang. Output only the script."
:'<,'>!clank --thinking off -m "what does this do?"
```

*Verified* headlessly (`nvim --headless -c '%!…' -c wq`): the buffer gained
`# Print the word "hello"` after the shebang. The answer replaces the buffer — no
other file is touched.

**A herdr pane.** Syntax verified with `herdr pane --help` / `agent --help`;
**not executed here**, because it would create panes in your session.

```sh
herdr pane list | jq -r '.result.panes[].pane_id'          # pane ids
herdr pane split --current --direction right --ratio 0.4   # then read the id from the JSON
herdr pane run <PANE_ID> 'rg "userData" src/ | clank --thinking off -m "what does this do?"'
herdr pane read <PANE_ID> --source recent
herdr agent prompt <TARGET> "review the diff in src/" --wait
```

Note: `pane run` sends text and Enter (`send-text`'s help); there is no
`pane wait-output` — waiting is `herdr agent wait <TARGET> --until <status>` for
agent panes. (An earlier version of the cheatsheet invented `wait-output`.)

## 6. Propose, then gate

clank never executes what the model says. The pattern is always: model writes
data, a mechanical check decides, a human or the shell runs it.

**The model writes a jq filter; jq runs it.**

```sh
clank --thinking off --json-schema '{"type":"object","properties":{"filter":{"type":"string"}},"required":["filter"]}' \
  -m 'a jq filter that keeps elements with a "children" key' | jq -r .filter > proposed.jq
cat proposed.jq                                   # read it: filters can read files
jq -c -f proposed.jq data.json || clank -c proposed.jq -m 'that filter failed; fix it'
```

*Verified:* the first proposal used a non-existent `@compact` format; jq exited 3
with a legible error; feeding that error back produced a working filter. jq's own
exit code is the gate.

**A script, checked before a human runs it.**

```sh
clank --json-schema '{"type":"object","properties":{"script":{"type":"string"}},"required":["script"]}' \
  -m "emit a bash script that renders fixtures/notes.md as HTML, as JSON" | jq -er .script > s.sh
bash -n s.sh && bash s.sh
```

**A reusable skill.** Ask clank to write an instruction block, keep it, then pass
it as the system directive whenever that job comes up.

```sh
clank --thinking off --max-tokens 300 \
  -m 'Write a reusable instruction block for reviewing a bash script. Markdown, no preamble:
      one finding per line as file:line: issue, no praise, end with a single VERDICT line.' \
  > prompts/review-sh.md

clank --thinking off --max-tokens 300 --system @prompts/review-sh.md -c demo.sh -m "Review the script."
```

*Verified:* the generated skill produced exactly the required format, including
`VERDICT:`, on a second, different subject.

## 7. Continue a run

```sh
clank --jsonl -m "summarize fixtures/notes.md" | clank -m "what did you say?"   # pipe the trace
clank -c trace.jsonl -m "what did you read?"                                    # or the file
```

A trace is rendered as a transcript: `assistant:` lines, `> tool args`,
`< tool ok` with up to 400 bytes of what the tool returned, `─── item i/n ───`
frames and `error: …` lines. Tool output beyond the cap is elided with a
`… N B not shown` marker — the trace file itself keeps everything.

*Verified:* with `--tools`, a trace of a `read_file` call re-fed into a new run
let the model quote the heading it had read.
