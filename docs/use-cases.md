# Use cases

clank is a stage, not an assistant: you hand it text, it hands back an answer on
stdout, and the shell decides what happens to that answer. Everything below is a
*task* built out of that, with the gate that tells you whether it worked and the
price you pay for it.

Every command with a concrete input was run against the local endpoints on
2026-09-17 (`:40583` qwen3.8-27b unless noted, `--thinking off`), and the
observations quoted are its output. §8 and §9 were run on 2026-09-19 against the
DeepSeek gateway on `:4000`. Blocks containing `…` placeholders illustrate
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

## 8. Run it on a timer

Everything above assumes you are there. Nothing in clank requires that: the timer
is the control loop, clank is the stage, and the answer goes to a file you read
when you feel like it. This is the shape where neither a conversation nor your
attention is the clock.

The script is the interesting part. A timer cannot see your editor, so the shell
assembles the evidence — that is the job, and everything else is plumbing:

```sh
#!/bin/sh
# ~/.local/bin/clank-sweep — one stage, one answer, appended to a drain file.
set -eu
out=${CLANK_SWEEP_OUT:-$HOME/.local/state/clank/sweep.jsonl}
mkdir -p "$(dirname "$out")"
cd "$HOME/Work/clank"

ctx=$(git log --since='24 hours ago' --stat --format='%h %s' | head -200)
[ -n "$ctx" ] || exit 0          # nothing changed: no call, no cost

printf '%s\n' "$ctx" \
  | clank --thinking off --max-tokens 400 --jsonl \
      -m "The context is the output of git log --stat for the last day.
For each commit: one line, what changed and the risk it carries.
No praise, no preamble, no summary paragraph." \
      >> "$out"
```

Two `systemd --user` units run it. `Persistent=true` is why a timer beats a
crontab line: a run missed because the machine was off still happens.

```ini
# ~/.config/systemd/user/clank-sweep.service
[Service]
Type=oneshot
Environment=CLANK_BASE_URL=http://127.0.0.1:4000/v1
Environment=CLANK_MODEL=deepseek/deepseek-v4-flash
ExecStart=%h/.local/bin/clank-sweep
```

```ini
# ~/.config/systemd/user/clank-sweep.timer
[Timer]
OnCalendar=daily
Persistent=true
RandomizedDelaySec=15m

[Install]
WantedBy=timers.target
```

```sh
systemctl --user daemon-reload && systemctl --user enable --now clank-sweep.timer
journalctl --user -u clank-sweep -n 20                       # the timer's exit status
jq -c 'select(.type=="assistant") | .content' ~/.local/state/clank/sweep.jsonl
```

*Cost:* one call per run, and none at all on a day where nothing changed.
*Gate:* the exit status in the journal, and the events in the file — `--jsonl` is
what stops a failed run from looking like a quiet day.

*Verified* on 2026-09-19 against the DeepSeek gateway (`:4000`,
`deepseek/deepseek-v4-flash`), the script run by hand twice; **the units
themselves are not installed**. At `--max-tokens 300` it truncated on a
two-commit day and exited 1 — the contract of §1 — and the file held a lone
`run` event with no `assistant`, which is exactly how the failure stays visible.
At 400 it answered. The `run` event carries the model, the endpoint, the prompt
id and the argv, so the queue explains itself months later. The model also
ignored "one line" and wrote a paragraph per commit; clank does not police
length, by design. One of those lines is the argument for the whole shape: it
noticed that the demo renderer added in one commit was deleted in the next.

Three things this shape needs that an interactive one does not:

**A stable prompt head.** A cached prefix measured 10.4 s → 0.21 s (§4). The
context varies every run; the instructions must not.

**A story for contention.** The endpoint has finite slots and a nightly job can
land on top of a live session. `flock` on a lockfile, or check the server first.

**Proposals, never applications.** The file is a queue. Nothing a timer produces
is applied by the timer — the gate is still a human, just later.

## 9. The loop and the goal, without a framework

What a framework sells as an agent loop and goal-directed behaviour is an `until`
and an exit code. The model proposes; the shell decides when the work is done,
because the shell is the only party that can check.

**The goal is a check, not a claim.** The shell knows the answer, so the model
never gets to announce success:

```sh
truth=$(find src -name '*.rs' | wc -l)      # the goal, computed by the shell
until [ "$(sh candidate.sh 2>/dev/null)" = "$truth" ]; do
  { echo "attempt produced: [$(sh candidate.sh 2>&1 | head -2)]"
    echo "it must print exactly: $truth"; } \
    | clank --thinking off --max-tokens 120 \
        -m "Write a one-line POSIX sh command that prints the number of .rs files under src/. Output only the command." \
    > candidate.sh
done
```

*Verified:* converged on the first attempt, with `find src -name '*.rs' | wc -l`.
*Cost:* one call per round, and none at all if the first proposal satisfies the
check. *Gate:* string equality against a number the model never saw.

**The loop feeds the failure back.** The gate's own error is the next round's
context, so "self-correction" needs no machinery:

```sh
until jq -e -f proposed.jq fixtures/ctx.json >/dev/null 2>&1; do
  { echo "the filter so far:"; cat proposed.jq
    echo "jq says:"; jq -f proposed.jq fixtures/ctx.json 2>&1 | head -1; } \
    | clank --thinking off --max-tokens 200 \
        -m "The context is my broken jq filter and jq's error. Fix the filter. Output only the filter." \
    | tr -d '`' > proposed.jq
done
```

*Verified, and the first version did not converge.* Five rounds, every one of them
a correct filter wrapped in markdown fences; the gate never passed, and the gate was
right to refuse. The model's answer had been correct from round one — the loop was
failing on formatting, not on reasoning. `tr -d '`'` in the pipeline fixed it, and
it then converged in one round. That is the honest cost of owning the loop: its
failure modes are the shell's, and so are the fixes. A framework hides this one and
charges you trust for the hiding.

**Workers are `xargs -P`.** No scheduler, no queue:

```sh
find src -name '*.rs' | xargs -P4 -I{} sh -c \
  'clank --thinking off --max-tokens 80 -c {} -m "One line: what does this file do?" | sed "s|^|{}: |"'
```

*Verified:* four files summarized concurrently, one line each.
*Cost:* four calls at once, against whatever the endpoint will serve in parallel.

**Memory is a file.** `clank --jsonl … > step1.jsonl`, then
`clank -c step1.jsonl -m "what did you say last?"` answers from the transcript
(§7). Already verified.

Everything a harness advertises, and what it is here:

| the framework calls it | here |
|---|---|
| the agent loop | `until <check>; do …; done` |
| goal-directed behaviour | the check, which you write, and the model cannot fake |
| self-correction, reflection | the failure output, as the next round's context |
| subagents, parallel workers | `xargs -P4` |
| tool use | the pipe |
| memory, session state | `-c trace.jsonl`, or any file you keep |
| context compaction | a pipeline stage of its own, or a `-c` tree you curate |
| a persistent agent | a timer and a drain file (§8) |

Eight lines of shell, and you own its failure modes. That is the trade: the
framework's real product is that somebody else owns them — which is the same fact
as the trust problem.
