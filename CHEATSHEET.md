# clank cheatsheet

`clank` = local-model harness that behaves like a unix filter: prompt/context
in on argv/stdin, data out on stdout, diagnostics on stderr. One prompt, one
request, one answer, and the context is exactly what you piped — the shell does
the searching and the fan-out. `--tools` adds four read-only observers
(`read_file`, `list_dir`, `search`, `stat`) for the lookup a pipe cannot cover;
no shell, no editing. Full doctrine: [PROTOCOL.md](PROTOCOL.md).

Defaults: `CLANK_MODEL=qwen3.8-27b-gsq-rco-iq3xxs`,
`CLANK_BASE_URL=http://127.0.0.1:40583/v1`. Override with flags or `$CLANK_*`.

## Contract

| stream | carries |
|---|---|
| stdout | data only: final assistant text, or `--jsonl` events |
| stderr | breadcrumbs (`> tool …`, `< tool ok (N B)`), reasoning with `--show-thinking`, errors |
| stdin | prompt (if no `-m`/positional), else context; a TTY with no prompt reads one line |

Exit codes: `0` ok · `1` model/server/IO failure · `2` usage error.
SIGPIPE restored — `clank … | head` dies cleanly. `NO_COLOR` honored.

## Prompt & context

```sh
clank -m "reply with exactly: pong"          # prompt
echo "ping" | clank                          # piped stdin = prompt
clank reply with exactly pong                # positional = prompt
clank -m "what is this?" < file.txt          # stdin = context, -m = question
rg "userData" src/ | clank -m "what does this do?"
clank -c ctx.json -m "summarize"             # context file
clank -c a.json -c b.txt -m "summarize both" # repeatable, in order
```

Context file formats (auto-detected):

- plain text → one text node
- JSON tree: `[{"text": …}, {"file": "rel/path"}, {"children": […] }]`
- clank JSONL transcript (from `clank --jsonl`) → rendered as a tagged
  transcript: `assistant: …`, `> tool path=…`, `< tool ok`

## Continuing a run

The JSONL trace of a run is itself valid context:

```sh
clank --jsonl -m "list the fixtures" | tee trace.jsonl
clank -c trace.jsonl -m "what did the assistant do?"
clank --jsonl -m "…" | clank -m "continue"
```

## Mapping over items

`--each` is one prompt, many items, one framed answer each. Items come from
stdin; the prompt must be `-m`/positional.

```sh
rg -l "TODO" src/ | clank --each -m "one-line summary of this file"
find src -name '*.rs' -print0 | clank --each -0 -m "any overflow risk here?"
git diff --name-only | clank --each -m "what changed here? one line"

# text mode frames the answers with the item they belong to
rg -l "unsafe" src/ | clank --each -m "explain the risk"
# ─── item 1/3 ───
# src/main.rs: …

# failures only, and the run continues past them (exit 1 at the end)
clank --each --jsonl -m "rate the risk 1-5" < files.txt | jq -c 'select(.type=="error")'

# item -> answer, joined on the index every event carries
clank --each --jsonl -m "rate the risk 1-5" < files.txt \
  | jq -s 'map(select(.type=="item" or .type=="assistant" or .type=="error"))
           | group_by(.i)
           | map({input: .[0].input, answer: (.[1].content // null),
                  error: (.[1].message // null)})'
```

Serial by design: put `xargs -P` in front of it if you want parallel fan-out.

## Thinking

```sh
# thinking is on by default in the local templates: cheapest map, off
rg -l "TODO" src/ | clank --each --thinking off -m "one-line summary"

# a specific effort level (the template decides what it honours)
clank --thinking low -m "explain this stack trace" < trace.txt

# watch the reasoning (stderr) while the answer stays on stdout
clank --show-thinking -m "what is wrong here?" < err.log 2>thinking.log

# what produced this trace
clank --jsonl -m "summarize" < big.txt | jq -c 'select(.type=="run")'

```

## Flags

| flag | env | meaning |
|---|---|---|
| `-m TEXT` / positional | | prompt |
| `-c FILE` (repeatable) | | context file (tree, text, or JSONL trace) |
| `--each` | | run the prompt once per stdin item |
| `-0` / `--null` | | with `--each`: NUL-separated items (`find -print0`) |
| `--system TEXT` | `CLANK_SYSTEM` | extra directive appended to the system prompt |
| `--list-tools` | | print tool definitions as JSON, no model call |
| `--jsonl` / `-j` | | JSONL events on stdout instead of text |
| `-q` / `--quiet` | | suppress stderr breadcrumbs |
| `--model` | `CLANK_MODEL` | model name |
| `--base-url` | `CLANK_BASE_URL` | OpenAI-compatible base URL |
| `--api-key` | `CLANK_API_KEY` | bearer token |
| `--timeout N` | `CLANK_TIMEOUT` | per-request timeout, s (default 600) |
| `--max-rounds N` | | tool-call rounds (default 12) |
| `--max-tokens N` | | completion cap (default 8192) |
| `--json-schema JSON` | | constrain final answer to a JSON schema (one request; with `--tools`: rounds first, then one schema'd request) |
| `--tools` | | offer the read-only filesystem tools (off by default) |
| `--thinking LEVEL` | | `off` disables thinking via the template; a level (`minimal`…`max`) goes as `reasoning_effort`; default sends nothing |
| `--show-thinking` | | stream the model's reasoning to stderr (it is billed either way) |

Debug: `CLANK_DEBUG=/path/req.json clank …` writes the exact request body
(first round) to the file.

## Recipes

```sh
# explain a file
clank -m "explain this file" < src/main.rs

# explain a selection of it
sed -n '10,40p' src/main.rs | clank -m "what does this do?"

# structured output: one request, the schema enforced by the server
clank -m 'Reply with JSON: {"ping":"pong"}' \
  --json-schema '{"type":"object","properties":{"ping":{"type":"string"}},"required":["ping"]}' \
  | jq

# structured output from the evidence the shell gathered: the pipe is the input
rg -l '' -g '*.rs' src/ | clank -m 'Emit {"files":["..."]} for these paths.' \
  --json-schema '{"type":"object","properties":{"files":{"type":"array","items":{"type":"string"}}},"required":["files"]}' \
  | jq -r '.files[]'

# any JSON can be piped: the pipe is evidence, whatever its shape
jq -c '.[] | select(.type=="function")' tools.json | clank -m "what tools are these?"

# clank can read its own tool schemas (the schema shape, as data)
clank --list-tools | jq -c '.[] | select(.function.name=="search")' | clank -m "what does this promise?"

# the model writes a jq filter, the shell runs it, jq's exit code is the gate
clank --thinking off --json-schema '{"type":"object","properties":{"filter":{"type":"string"}},"required":["filter"]}' \
  -m 'a jq filter that keeps elements with a "children" key' | jq -r .filter > /tmp/f.jq
jq -c -f /tmp/f.jq data.json || clank -c /tmp/f.jq -m 'that filter failed; fix it'

# filter a pipe, keep the trace
cat big.txt | clank --jsonl -m "summarize" | tee -a trace.jsonl

# the lookup belongs to the shell, not to the model
rg -n "TODO" src/ | clank -m "group these TODOs by file, one line each"
sed -n '1,80p' src/main.rs | clank -m "what does this do?"
```

## Neovim

`clank` is read-only, so filter uses never write files — the buffer is
replaced by the answer.

```vim
" whole buffer through clank; answer replaces the buffer
:%!clank -m "explain this code"

" visual selection through clank (select first, then)
:'<,'>!clank -m "what does this do?"

" ask about a file; answer in the terminal window, buffer untouched
:!clank -m "explain this file" < src/main.rs

" interactive
:terminal clank -m "explain this repo"
```

`:%!cmd` is the classic unix filter: buffer lines → cmd stdin, stdout →
buffer. `:'<,'>!cmd` does the same for a visual range.

## Herdr

Run inside a Herdr-managed pane (`HERDR_ENV=1`); parse JSON IDs, use
`--no-focus` for background work:

```sh
# sibling pane running a clank pipeline
herdr pane split --current --direction right --cwd "$PWD" --no-focus
herdr pane run  <pane> 'rg "userData" src/ | clank -m "what does this do?"'
herdr pane wait-output <pane> --match "done" --timeout 120
herdr pane read <pane> --source recent-unwrapped --lines 100

# continuation chain in one pane
herdr pane run <pane> 'clank --jsonl -m "build the script" | tee trace.jsonl'
herdr pane run <pane> 'clank -c trace.jsonl -m "summarize the run above"'
```

Pairing: an agent does the heavy delegated work, `clank` is the cheap
observable tool beside it:

```sh
herdr agent prompt reviewer "Review the diff in src/" --wait --timeout 600
```

## tmux

```sh
# whatever is on screen becomes the prompt's context
tmux capture-pane -p | clank -m "what is on this screen?"

# pane output piped into clank in the background
tmux pipe-pane 'clank -m "summarize this pane" > summary.txt'

# one pane, summarize another
tmux capture-pane -p -t other | clank -m "summarize that pane" | less
```
