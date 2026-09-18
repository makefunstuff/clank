# clank cheatsheet

`clank` behaves like a unix filter: prompt and context in on argv/stdin, data
out on stdout, diagnostics on stderr. One prompt, one request, one answer, and
the context is exactly what you piped — the shell does the searching, the
fan-out and the looping.

- **[docs/use-cases.md](docs/use-cases.md)** — real jobs, verified commands, prices and gates
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
failed `--each` item · `2` usage. SIGPIPE restored, so `clank … | head` dies
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

# map one prompt over many items, one framed answer each
rg -l "TODO" src/ | clank --each --thinking off -m "one-line summary"
find src -name '*.rs' -print0 | clank --each -0 -m "any overflow risk here?"

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
rg -l "TODO" src/ | clank --each --thinking off -m "one-line summary"   # no thinking
clank -c big-context.json -m "…"                                        # stable head = cached prefix
clank -m "the first line is all I need" | head -1                       # stop early, 12× faster
printf '%s\n' a b c | clank --json-schema @list-schema.json -m "…"      # one call, three items
```

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
