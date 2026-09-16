# clank cheatsheet

`clank` = local-model harness that behaves like a unix filter: prompt/context
in on argv/stdin, data out on stdout, diagnostics on stderr. Read-only tools
(`read_file`, `list_dir`, `search`, `stat`) — no shell, no editing.

Defaults: `CLANK_MODEL=qwen3.8-27b-gsq-rco-iq3xxs`,
`CLANK_BASE_URL=http://127.0.0.1:40583/v1`. Override with flags or `$CLANK_*`.

## Contract

| stream | carries |
|---|---|
| stdout | data only: final assistant text, or `--jsonl` events |
| stderr | breadcrumbs (`> tool …`, `< tool ok (N B)`), errors |
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

## Flags

| flag | env | meaning |
|---|---|---|
| `-m TEXT` / positional | | prompt |
| `-c FILE` (repeatable) | | context file (tree, text, or JSONL trace) |
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
| `--json-schema JSON` | | constrain final answer to a JSON schema (needs `--no-tools` on this server build) |
| `--no-tools` | | disable tool calling |

Debug: `CLANK_DEBUG=/path/req.json clank …` writes the exact request body
(first round) to the file.

## Recipes

```sh
# explain a file
clank -m "explain this file" < src/main.rs

# explain a selection of it
sed -n '10,40p' src/main.rs | clank -m "what does this do?"

# structured output
clank -m 'Reply with JSON: {"ping":"pong"}' \
  --json-schema '{"type":"object","properties":{"ping":{"type":"string"}},"required":["ping"]}' \
  --no-tools | jq

# filter a pipe, keep the trace
cat big.txt | clank --jsonl -m "summarize" | tee -a trace.jsonl

# narrow the tool search
clank -m "find TODOs in the src tree"
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
