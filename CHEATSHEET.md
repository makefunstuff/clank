# clank cheatsheet

`clank` behaves like a unix filter: prompt and context in on argv/stdin, data
out on stdout, diagnostics on stderr. One prompt, one request, one answer, and
the context is exactly what you piped — the shell does the searching, the
fan-out and the looping.

- **[docs/use-cases.md](docs/use-cases.md)** — real jobs, verified commands, prices and gates
- **[docs/macbook-omlx-local-inference.md](docs/macbook-omlx-local-inference.md)** — running it against local models on a 16 GB Mac: which model per job, hallucination probes
- **`clank-jev`** — the sibling binary for typed decisions: `--ask`/`--checks`, providers `typesafe` / `openrouter` / `kev`, gates in the exit code (see the README)
- **[PROTOCOL.md](PROTOCOL.md)** — the contract: invariants, request sequence, event set, exit codes

Defaults: `CLANK_MODEL=qwen3.8-27b-gsq-rco-iq3xxs`,
`CLANK_BASE_URL=http://127.0.0.1:40583/v1`. Flags override `$CLANK_*`.

## Contract

| stream | carries |
|---|---|
| stdout | data only: the answer, framed `--each` answers, or `--jsonl` events |
| stderr | breadcrumbs (`> tool …`, `< tool ok (N B)`), `--show-thinking` reasoning, errors |
| stdin | prompt (if no `-m`/positional), else context; with `--each`, the item list |

Exit codes: `0` ok · `1` model/server/IO, truncated answer, empty answer, or any
failed `--each` item · `2` usage. `clank-jev` adds `3` for a provider, network or
credential failure, so "could not ask" never looks like "did not pass". SIGPIPE restored, so `clank … | head` dies
cleanly. Nothing is ever coloured — there is no `NO_COLOR` to honour; `-q`
silences breadcrumbs.

## One-liners

```sh
# nothing to pipe? ask about the workspace and let it look around
clank --thinking off -m "where is the transcript cap defined? cite file:line"

# nothing to pipe and nothing to look at: blind on purpose
clank --no-tools -m "write a regex that matches ISO-8601 dates"

# ask about a pipe or a file
rg "userData" src/ | clank --thinking off -m "what does this do?"
clank --thinking off -m "explain this" < src/main.rs
sed -n '10,40p' src/main.rs | clank -m "what does this do?"

# context from files (repeatable, in order) and from a tree
clank -c ctx.json -m "summarize"
clank -c a.json -c b.txt -m "summarize both"
#   [{"text": "…"}, {"file": "rel/path"}, {"children": [{"file": "a.md"}]}]

# map one prompt over many items, one framed answer each.
# the item IS the context — hand over text, not paths (see Traps below)
rg -n "TODO" src/ | clank --each --thinking off -m "one line: actionable now, or not?"
git log --format=%s -5 | clank --each -m "one line: rewrite in the imperative mood"
find src -name '*.rs' -print0 | clank --each -0 -m "one line: what is this path for?"

# one file per item: the shell reads it, so the model gets content
for f in src/*.rs; do
  clank -c "$f" --thinking off -m "one line: what is this file responsible for?" </dev/null
  echo
done

# decisions for routing: pick one of your options, branch on it
route=$(printf '%s' "$task" | clank-jev --ask 'What kind of task is this?' --choice code,prose,math --min-prob 0.7)
case "$route" in code) clank --model local-code -m "$task" ;; *) clank --model local-fast -m "$task" ;; esac
printf '%s' "$text" | clank-jev --ask 'Is this a refund request?' --boolean       # prints true|false
cat state.txt | clank-jev --checks checks.json --json | jq -r '.answers.team.value'
printf '%s' "$trace" | clank-jev --checks fixtures/checks-verification.json --min-prob 0.6   # exit 1 = a claim conflicts with the tool results
clank-jev --provider kev --ask 'Which team?' --choice billing,shipping < ticket.txt           # local, no credentials

# structured output: one request, grammar enforced by the server
clank -m "…" --json-schema @schema.json | jq -er .
clank --thinking off --json-schema '{"type":"object","properties":{"kind":{"type":"string","enum":["code","docs"]}},"required":["kind"]}' -m "Classify: code or docs."

# reusable instruction block written once, passed every time
clank --system @prompts/review-sh.md -c demo.sh -m "Review the script."

# continue a run: a trace is context
clank --jsonl -m "…" | clank -m "what did you say?"
clank -c trace.jsonl -m "what did you read?"

# machine-readable stream, and the run it came from
clank --jsonl -m "…" | jq -c 'select(.type=="assistant") | .content'
clank --jsonl -m "…" | jq -c 'select(.type=="run")'
```

## `clank-jev` flags

The decision stage. Credentials come from the environment only.

| flag | meaning |
|---|---|
| `--ask TEXT` | one question; needs exactly one of the three shapes below |
| `--choice A,B,C` | unordered options, printed by name |
| `--boolean` | yes/no, printed as `true` or `false` |
| `--score low,mid,high` | ordered levels, lowest first; prints the level number |
| `--checks FILE` | a JSON file of questions: `{id: {type, instructions, criteria, reasons?}}` |
| `--min-prob P` | exit 1 if any answer's probability is below this |
| `--expect VALUE` | exit 1 unless the decision equals this (one question) |
| `--expect-min N` | exit 1 unless an ordered decision is at least this level |
| `--print-reason` | print the closed-choice reason instead of the value |
| `--provider NAME` | `auto`, `typesafe`, `openrouter`, `kev` |
| `--model ID` | defaults per provider: `jev-latest`, `typesafe/jev-1.13`, `kev-latest` |
| `--base-url URL` | move the endpoint (a local `kev`, a proxy, a stub) |
| `--timeout SECS` | per-request timeout, default 60 |
| `--json` | the full result object instead of the bare value |
| `-q` / `--quiet` | no diagnostic line on stderr |

`TYPESAFE_API_KEY` (or `JEV_API_KEY`, `JEV_CLI_API_KEY`) selects the TypeSafe
route; `OPENROUTER_API_KEY` selects OpenRouter's Decisions endpoint; `--provider
kev` needs no credential at all.

## Flags

| flag | env | meaning |
|---|---|---|
| `-m, --message TEXT` / positional | | prompt |
| `-c, --context FILE` (repeatable) | | context file: JSON tree, plain text, or clank JSONL trace |
| `--each` | | run the prompt once per stdin item |
| `-0` / `--null` | | with `--each`: NUL-separated items (`find -print0`) |
| `--json-schema JSON` | | constrain the final answer to a JSON schema (one request; the grammar is server-enforced) |
| `--system TEXT` | `CLANK_SYSTEM` | directive appended to the built-in system prompt |
| `--thinking LEVEL` | | `off` disables thinking via the template; a level (`minimal`…`max`) goes as `reasoning_effort`; default sends nothing |
| `--show-thinking` | | stream the model's reasoning to stderr (billed either way) |
| `--tools` | | offer the four read-only observers (default: on only when nothing was piped) |
| `--no-tools` | | never offer them, even with nothing to pipe |
| `--list-tools` | | print the tool definitions as JSON, no model call |
| `--jsonl` / `-j` | | JSONL events on stdout instead of text |
| `-q` / `--quiet` | | suppress stderr breadcrumbs |
| `--model` / `--base-url` / `--api-key` | `CLANK_MODEL` / `CLANK_BASE_URL` / `CLANK_API_KEY` | endpoint |
| `--timeout N` | `CLANK_TIMEOUT` | per-request timeout, seconds (default 600) |
| `--max-tokens N` | | completion cap (default 8192) |
| `--max-rounds N` | | tool-call rounds with `--tools` (default 12) |

`--json-schema` and `--system` accept `@path` to read the payload from a file —
that is how a schema or a skill clank wrote earlier comes back in.

Debug: `CLANK_DEBUG=/path/req.json clank …` writes the exact request body of the
first round.

## Keeping it cheap

```
one call ≈ 0.25 s + 0.02 s × output tokens
         + prompt tokens at 1.3 ms cold / 0.02 ms cached
```

```sh
rg -n "TODO" src/ | clank --each --thinking off -m "one line: actionable?"   # no thinking
clank -c big-context.json -m "…"                                        # stable head = cached prefix
clank -m "the first line is all I need" | head -1                       # stop early, 12× faster
printf '%s\n' a b c | clank --json-schema @list-schema.json -m "…"      # one call, three items
```

## Traps

- **`--each` items are text, not files.** `printf 'src/main.rs\n' | clank --each
  -m "summarize this file"` shows the model the path and nothing else, and it
  answers accordingly (*"This file exists but its purpose is not described in the
  provided context"*, measured on oMLX 2026-09-19). Read files in the shell — the
  `-c` loop above — or pipe content you already gathered (`rg -n …`).
- **A `while read` loop must give each call its own stdin.** clank reads stdin, so
  without `</dev/null` the first call eats the rest of the item list.
- **`--json-schema` is the server's grammar to enforce, not clank's.** It is not
  enforced everywhere it is accepted: measured on oMLX, one model returned bare
  JSON, one wrapped it in a fence, one answered in prose. Gate with `jq -er`.
- **A budget overrun is `exit 1`, not a short answer.** `--max-tokens` cutting the
  answer is a failure by design, so it cannot ship through a `&&` chain.
- **`xargs -P` against one endpoint is not free parallelism.** Measured on oMLX:
  12.5 s with `-P2` against 5.8 s serial over four files, plus interleaved stdout.
  It went the other way on llama.cpp (`docs/use-cases.md` §3) — measure yours.

## Integrations

```sh
# whatever is on a tmux pane (verified)
tmux capture-pane -p | clank --thinking off -m "one line: what is wrong?"

# output anywhere: a file (atomic), a FIFO, a socket
clank -m "…" > out.tmp && jq -e . out.tmp && mv out.tmp out.json
mkfifo /tmp/p && clank -m "…" > /tmp/p & cat /tmp/p
socat UNIX-LISTEN:/tmp/s - > got.txt & clank -m "…" | socat - UNIX-CONNECT:/tmp/s
```

```vim
" neovim: the buffer is the pipe (verified headlessly with nvim --headless)
:%!clank --thinking off -m "Add a comment line after the shebang. Output only the script."
:'<,'>!clank -m "what does this do?"
```

```sh
# herdr: syntax checked against herdr pane --help / herdr agent --help;
# not executed in the review (it would create panes in your session)
herdr pane list | jq -r '.result.panes[].pane_id'
herdr pane split --current --direction right --ratio 0.4
herdr pane run <PANE_ID> 'rg "userData" src/ | clank --thinking off -m "what does this do?"'
herdr pane read <PANE_ID> --source recent
herdr agent prompt <TARGET> "review the diff in src/" --wait
```

`pane run` sends the text and an Enter; there is no `pane wait-output` — for
agent panes, wait with `herdr agent wait <TARGET> --until <status>`.
