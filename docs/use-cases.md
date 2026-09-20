# Use cases

clank takes text on stdin and writes an answer to stdout; the shell decides what
happens to that answer.

Every command with a concrete input was run against the local endpoints on
2026-09-17 (`:40583` qwen3.8-27b unless noted, `--thinking off`), and the
observations quoted are its output. §8 and §9 were run on 2026-09-19 against the
DeepSeek gateway on `:4000`. Blocks containing `…` placeholders illustrate a
rule rather than a runnable line; the herdr block is marked "syntax verified,
not executed" with the reason.

For which model per job, what the small ones get wrong, and the probes behind
it, see [macbook-omlx-local-inference.md](macbook-omlx-local-inference.md).
Prices come from the measured cost model
([history/research-harness-constraints.md](history/research-harness-constraints.md)
§5.2):

```
one call ≈ 0.25 s + 0.02 s × output tokens
         + prompt tokens at 1.3 ms cold / 0.02 ms cached
```
## Nothing to pipe

Piped input is the evidence and nothing else. With no stdin, no `-c` and no
`--each` items the model is offered the four read-only observers, and every
lookup it makes lands on stderr.

| run | result |
|---|---|
| that question, nothing piped | `src/context.rs:68`, `TRANSCRIPT_OUTPUT_CAP = 400` at `src/context.rs:136`, call site at `:112`, after three logged lookups |
| the same question without the tools | `src/renderer.ts:14`, cap 8000: a file that does not exist, in the citation format of a real answer, exit 0 |

```sh
clank --thinking off -m "where is the transcript cap defined, and what enforces it? cite file:line"

clank --no-tools -m "write a regex that matches ISO-8601 dates"
# → No context was provided, so I cannot cite a `file:line`.
clank --no-tools --json-schema @extract.schema.json -m "pull the fields out of this" < input.txt
```
## 1. Text you already have

The pipe is the evidence, so the model opens no file you did not hand it.

| job | cost | gate |
|---|---|---|
| search match | 1 call, ~0.3 s | you read it |
| diff review | 1 call; truncated at 160 tokens, exit 1 | exit 0 and a human reads the lines: a truncated review is a failed run, not a shorter one |
| failure triage | 1 call; the model cannot run anything, so a fix is a proposal (§6) | exit 0 |

```sh
# search match
rg -n -C3 "userData" src/ | clank --thinking off -m "what does this code do? cite file:line"
# diff review before committing
git diff HEAD~1 -- src/context.rs | head -70 \
  | clank --thinking off --max-tokens 400 \
      -m "Review this diff. One line per issue as file:line: problem. If none, output exactly: no issues."
# failure triage, from the command that produced it
make 2>&1 | tail -40 | clank --thinking off --max-tokens 200 -m "root cause, then the one-line fix"
```
## 2. Structured output to act on

The schema is enforced by the server's grammar: in testing, a schema whose only
legal value was `alpha` overrode an explicit instruction to answer `beta`.

| job | cost | gate |
|---|---|---|
| field extraction | 1 call | `jq -er` fails the pipeline on a wrong shape, and clank exits 1 if the answer is not JSON at all |
| list triage | one call per item (0.25 s each) | one `─── item i/n ───` frame per item maps answers to inputs; `--jsonl` + `jq` for machine use |
| schema generation, then reuse from disk | 2 calls: design, then extract | `jq -e` on the schema; 237 bytes of schema then extracted 5 findings from real `rg` output |

```sh
# field extraction
rg -n "unwrap|expect\(" src/ \
  | clank --thinking off --json-schema @schema-findings.json -m "Extract the findings." \
  | jq -er '.findings[] | "\(.file):\(.line): \(.issue)"'
# list triage, one schema'd answer per item
printf 'src/context.rs\nsrc/tools.rs\nREADME.md\n' \
  | clank --each --thinking off --max-tokens 60 \
      --json-schema '{"type":"object","properties":{"kind":{"type":"string","enum":["code","docs"]}},"required":["kind"]}' \
      -m "Classify this path as code or docs. Answer with JSON."
# schema generation, then reuse from disk; @ reads the payload from disk
clank --thinking off --max-tokens 400 \
  -m 'Design a JSON Schema, draft-07 without $schema, for extracting review findings:
      an object with "findings", an array of {file, line, issue}. Output ONLY the schema.' \
  > schema-custom.json
jq -e . schema-custom.json                # gate: it must be valid JSON
rg -n "TODO" src/ | clank --json-schema @schema-custom.json -m "Extract the findings."
```

```
─── item 1/3 ───
{"kind": "code"}
─── item 2/3 ───
{"kind": "code"}
─── item 3/3 ───
{"kind": "docs"}
```
## 3. Mapping over many things

**An item is text, not a file** (measured 2026-09-19 on oMLX): a path handed to
`--each` is summarised as a filename, and the models answered *"This file exists
but its purpose is not described in the provided context."*

| job | cost | gate |
|---|---|---|
| per-file summary | one call per item, 0.25 s + output each | `</dev/null`: clank reads stdin, so without it the first call consumes the rest of the list |
| partial failure | one call per item | exit 1 for "did all of it work", the `error` events for "which part did not": exit 1, three `error` events, `1 2 3` recoverable from the trace |
| batching | 1 call for the whole list | 3 entries from 1 call; at four items serial `--each` 2.4 s, `xargs -P2` 1.5 s, batched 1.6 s, and the gap widens with N |
| batching, on oMLX (MLX runtime, 2026-09-19) | 1 call for the whole list | the comparison came out the other way: 5.8 s serial against 12.5 s with `-P2`, the parallel answers interleaving on stdout |

```sh
# per-file summary, reading each file in the shell
for f in $(git diff --name-only); do
  clank -c "$f" --thinking off --max-tokens 120 -m "one-line summary of this file" </dev/null
  echo
done
# partial failure, retry list from the trace
printf 'a\nb\nc\n' | clank --each --jsonl --max-tokens 1 -m "Say something long." > run.jsonl
echo $?                                   # 1 → something failed, the run still finished
jq -r 'select(.type=="error") | .i' run.jsonl   # 1 2 3 → exactly what to retry
# batching, one call for a list of items
printf 'src/context.rs\nsrc/tools.rs\nREADME.md\n' | clank --thinking off --max-tokens 120 \
  --json-schema '{"type":"object","properties":{"items":{"type":"array","items":{"type":"object",
     "properties":{"path":{"type":"string"},"kind":{"type":"string"}},"required":["path","kind"]}}},"required":["items"]}' \
  -m "The context is a list of paths. Return one entry per path with its kind."
```
## 3b. Model routing for a job

A script that picks its own model needs a decision that fails loudly when it is
not confident; `clank-jev` is that stage, and neither binary knows about the
other.

| cost | gate |
|---|---|
| one Jev call: 83 ms against a local `kev-0.6b`, 591 ms against hosted Jev, $0 and $0.000015, plus the model call you were going to make anyway | the exit code: `--min-prob 0.7` turns a hesitant decision into `exit 1`, so `\|\| route=unclear` catches it before the wrong model is paid for; `--expect` and `--expect-min` do the same for a value you already know you need |

clank's contract is one prompt, one request, one answer, no second wire
protocol, and a decision stage stands alone in a hook, a Makefile or a cron job.
See PROTOCOL.md, *Why the decision stage is a separate binary*.

```sh
route=$(printf '%s' "$task" | clank-jev --ask 'What kind of task is this?' \
          --choice code,prose,math --min-prob 0.7) || route=unclear
case "$route" in
  code) clank --model local-code -c src/context.rs -m "$task" ;;
  prose) clank --model local-fast -m "$task" ;;
  *) clank --model local-code -m "$task" ;;   # unclear: take the careful path
esac
```
## 4. Speed

The model is the constraint, so the lever is the number of tokens you pay for:

| rule | why | measured |
|---|---|---|
| `--thinking off` for mechanical work | thinking is billed and generated before any answer | 1.35 s → 0.25 s |
| keep the prompt head stable, vary the tail | cached prefixes are ~60× cheaper to process than cold ones | 10.4 s → 0.21 s |
| stop reading when you have enough (`\| head -1`) | the call dies with the pipe instead of finishing | 7.1 s → 0.6 s |
| prefer one call with many items over many small calls | the 0.25 s fixed cost is paid per call | 1.5× on 4 items |

```sh
rg -n "TODO" src/ | clank --each --thinking off -m "one line: actionable?"  # rule 1
clank -c big-cached-context.json -m "…"                                   # rule 2 (same file each time)
clank -m "the first line only matters" | head -1                          # rule 3
printf '%s\n' a b c | clank --json-schema @list-schema.json -m "…"        # rule 4
```
## 5. Environment wiring

| job | fact | gate |
|---|---|---|
| atomic write | a redirect makes a failed run look like a file of results | `jq -e .verdict`: the run answered `"pass"`, kept as `{"verdict": "pass"}`, and on failure nothing is kept |
| run log | every `--jsonl` stream opens with a `run` event (model, endpoint, argv) | `jq -c 'select(.type=="run") \| .clank'` delineates the runs, and the log is context for the next call |
| socket, FIFO, file | a FIFO is a rendezvous, not a buffer: the writer blocks until a reader opens, writes below 4 KB are atomic, and one writer per FIFO | the destination is yours: the socket run received `"pong"`, the FIFO run read its answer back |
| tmux pane | the pane's text is the pipe | live pane: *"ERROR: disk quota exceeded at /var/log"* → *"The disk quota at /var/log has been exceeded."* |
| neovim buffer | `%!` replaces the buffer with the command's stdout | headless run (`nvim --headless -c '%!…' -c wq`) gained `# Print the word "hello"` after the shebang, and no other file is touched |
| herdr pane | checked against `herdr pane --help` / `agent --help` | `pane run` sends text and Enter (`send-text`'s help); there is no `pane wait-output`, and waiting is `herdr agent wait <TARGET> --until <status>` for agent panes. An earlier cheatsheet version invented `wait-output` |

```sh
# atomic write, gated on the answer
clank --thinking off --json-schema '{"type":"object","properties":{"verdict":{"type":"string"}},"required":["verdict"]}' \
  -m "Verdict on this text, one word." <<< "the build is green" > out.tmp \
  && jq -e .verdict out.tmp \
  && mv out.tmp out.json
# append-only run log
clank --thinking off --jsonl -m "Reply with exactly: one" >> log.jsonl
clank --thinking off --jsonl -m "Reply with exactly: two" >> log.jsonl
jq -c 'select(.type=="run") | .clank' log.jsonl          # two runs, delineated
clank -c log.jsonl -m "what have I asked so far?"        # the log is context
# socket
socat UNIX-LISTEN:/tmp/out.sock - > received.txt &        # verified: "pong"
clank --thinking off -m "Reply with exactly: pong" | socat - UNIX-CONNECT:/tmp/out.sock
# FIFO
mkfifo /tmp/p && clank --thinking off -m "Reply with exactly: fifo" > /tmp/p &   # verified
cat /tmp/p
# tmux pane
tmux capture-pane -p | clank --thinking off -m "one line: what is wrong?"
```

```vim
:%!clank --thinking off --max-tokens 60 -m "Add a comment line after the shebang. Output only the script."
:'<,'>!clank --thinking off -m "what does this do?"
```

```sh
# syntax verified, not executed here: it would create panes in your session
herdr pane list | jq -r '.result.panes[].pane_id'          # pane ids
herdr pane split --current --direction right --ratio 0.4   # then read the id from the JSON
herdr pane run <PANE_ID> 'rg "userData" src/ | clank --thinking off -m "what does this do?"'
herdr pane read <PANE_ID> --source recent
herdr agent prompt <TARGET> "review the diff in src/" --wait
```
## 6. Proposal, then gate

clank never executes what the model says: the model writes data, a mechanical
check decides, a human or the shell runs it.

| job | gate |
|---|---|
| a jq filter, written by clank and run by jq | jq's own exit code: the first proposal used a non-existent `@compact` format, jq exited 3 with a legible error, and feeding that error back produced a working filter |
| a script checked before a human runs it | `jq -er .script`, then `bash -n` before `bash s.sh` |
| a reusable skill | the generated skill produced exactly the required format, including `VERDICT:`, on a second, different subject |

```sh
# a jq filter, written by clank and run by jq
clank --thinking off --json-schema '{"type":"object","properties":{"filter":{"type":"string"}},"required":["filter"]}' \
  -m 'a jq filter that keeps elements with a "children" key' | jq -r .filter > proposed.jq
cat proposed.jq                                   # read it: filters can read files
jq -c -f proposed.jq data.json || clank -c proposed.jq -m 'that filter failed; fix it'
# a script, checked before a human runs it
clank --json-schema '{"type":"object","properties":{"script":{"type":"string"}},"required":["script"]}' \
  -m "emit a bash script that renders fixtures/notes.md as HTML, as JSON" | jq -er .script > s.sh
bash -n s.sh && bash s.sh
# a reusable skill: write the instruction block, keep it, pass it as --system
clank --thinking off --max-tokens 300 \
  -m 'Write a reusable instruction block for reviewing a bash script. Markdown, no preamble:
      one finding per line as file:line: issue, no praise, end with a single VERDICT line.' \
  > prompts/review-sh.md

clank --thinking off --max-tokens 300 --system @prompts/review-sh.md -c demo.sh -m "Review the script."
```
## 7. Continuing a run

A trace is rendered as a transcript: `assistant:` lines, `> tool args`,
`< tool ok` with up to 400 bytes of what the tool returned, `─── item i/n ───`
frames and `error: …` lines. Output beyond the cap is elided with a
`… N B not shown` marker; the trace file itself keeps everything.

With `--tools`, a trace of a `read_file` call re-fed into a new run let the
model quote the heading it had read.

```sh
clank --jsonl -m "summarize fixtures/notes.md" | clank -m "what did you say?"   # pipe the trace
clank -c trace.jsonl -m "what did you read?"                                    # or the file
```
## 8. Running on a timer

Nothing requires you to be there: the timer is the control loop, the answer goes
to a file, and the shell assembles the evidence a timer cannot see:

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
Two `systemd --user` units run it; `Persistent=true` is why a timer beats a
crontab line, since a run missed because the machine was off still happens.

```ini
# ~/.config/systemd/user/clank-sweep.service
[Service]
Type=oneshot
Environment=CLANK_BASE_URL=http://127.0.0.1:4000/v1
Environment=CLANK_MODEL=deepseek/deepseek-v4-flash
ExecStart=%h/.local/bin/clank-sweep

# ~/.config/systemd/user/clank-sweep.timer
[Timer]
OnCalendar=daily
Persistent=true
RandomizedDelaySec=15m

[Install]
WantedBy=timers.target
```
| cost | gate |
|---|---|
| one call per run, and none at all on a day where nothing changed | the exit status in the journal and the events in the file: `--jsonl` stops a failed run from looking like a quiet day |

Run on 2026-09-19 against the DeepSeek gateway (`:4000`,
`deepseek/deepseek-v4-flash`), the script by hand twice; **the units themselves
are not installed**. At `--max-tokens 300` it truncated on a two-commit day and
exited 1 (§1), and the file held a lone `run` event with no `assistant`; at 400
it answered. The `run` event carries the model, endpoint, prompt id and argv.
The model ignored "one line" and wrote a paragraph per commit, which clank does
not police, by design; one line noticed the demo renderer added in one commit
was deleted in the next.

| need | reason |
|---|---|
| a stable prompt head | the context varies every run, the instructions must not; a cached prefix measured 10.4 s → 0.21 s (§4) |
| contention | the endpoint has finite slots and a nightly job can land on top of a live session: `flock` on a lockfile, or check the server first |
| proposals only | the file is a queue: the timer applies nothing, and the gate is still a human, just later |

```sh
systemctl --user daemon-reload && systemctl --user enable --now clank-sweep.timer
journalctl --user -u clank-sweep -n 20                       # the timer's exit status
jq -c 'select(.type=="assistant") | .content' ~/.local/state/clank/sweep.jsonl
```
## 9. The loop and the goal, without a framework

An agent loop and goal-directed behaviour are an `until` and an exit code: the
shell decides when the work is done, and only the shell can check.

| job | cost | gate |
|---|---|---|
| the goal as a shell check | one call per round, and none at all if the first proposal satisfies the check | string equality against a number the model never saw: it converged on the first attempt, with `find src -name '*.rs' \| wc -l` |
| failure fed back as context | one call per round | the gate's own error is the next round's context; the first version ran five rounds of correct filters wrapped in markdown fences without passing, because the loop failed on formatting rather than reasoning, and `tr -d '`'` in the pipeline fixed it in one round |
| workers via `xargs -P` | four calls at once, against whatever the endpoint will serve in parallel | four files summarized concurrently, one line each |
| memory | one call to write the trace, one to read it | `clank --jsonl … > step1.jsonl`, then `-c step1.jsonl` answers from the transcript (§7) |

```sh
# the goal as a shell check
truth=$(find src -name '*.rs' | wc -l)      # the goal, computed by the shell
until [ "$(sh candidate.sh 2>/dev/null)" = "$truth" ]; do
  { echo "attempt produced: [$(sh candidate.sh 2>&1 | head -2)]"
    echo "it must print exactly: $truth"; } \
    | clank --thinking off --max-tokens 120 \
        -m "Write a one-line POSIX sh command that prints the number of .rs files under src/. Output only the command." \
    > candidate.sh
done
# failure fed back as context
until jq -e -f proposed.jq fixtures/ctx.json >/dev/null 2>&1; do
  { echo "the filter so far:"; cat proposed.jq
    echo "jq says:"; jq -f proposed.jq fixtures/ctx.json 2>&1 | head -1; } \
    | clank --thinking off --max-tokens 200 \
        -m "The context is my broken jq filter and jq's error. Fix the filter. Output only the filter." \
    | tr -d '`' > proposed.jq
done
# workers, no scheduler, no queue
find src -name '*.rs' | xargs -P4 -I{} sh -c \
  'clank --thinking off --max-tokens 80 -c {} -m "One line: what does this file do?" | sed "s|^|{}: |"'
```
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

The shell keeps the failure modes; a framework's product is that somebody else
owns them.
