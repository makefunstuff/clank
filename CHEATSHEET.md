# clank cheatsheet

`clank` is a unix filter: prompt and context in on argv/stdin, data out on
stdout, diagnostics on stderr. One prompt, one request, one answer, and the
context is exactly what was piped.

- [PROTOCOL.md](PROTOCOL.md) — invariants, request sequence, event set, exit
  codes
- [README.md](README.md) — install, `clank-jev`, `clank-web`, containers
- [docs/use-cases.md](docs/use-cases.md) — jobs with their gates and prices
- [docs/macbook-omlx-local-inference.md](docs/macbook-omlx-local-inference.md) —
  local models on a 16 GB Mac

Set `CLANK_BASE_URL` and `CLANK_MODEL`, or `[clank]` in `./.clank/config.toml`.
That file is the working directory only. `--config PATH` or `CLANK_CONFIG` names
a different file. Precedence is flags, then the environment, then the file, then
built-ins. There is no built-in model or base URL. Web search keys live under
`[web]` and are read by `clank-web`. The key stays in the environment;
`api_key_env` names the variable.


## Install

```sh
cargo install clank-cli-app --locked   # package ≠ binary → clank / clank-jev / clank-web
# fallback: cargo install --locked --git https://github.com/makefunstuff/clank --tag v0.1.0
```

Never `cargo install clank` (unrelated crates.io crate).

## Contract

| stream | carries |
|---|---|
| stdout | data only: the answer, framed `--each` answers, or `--jsonl` events |
| stderr | breadcrumbs (`> tool …`, `< tool ok (N B)`), `--show-thinking` reasoning, errors |
| stdin | prompt (if no `-m`/positional), else context; with `--each`, the item list |

| code | meaning |
|---|---|
| `0` | ok |
| `1` | model/server/IO failure, a truncated answer, an empty answer, or any failed `--each` item |
| `2` | usage |
| `3` | `clank-jev` only: provider, network or credential failure |

`clank-web` uses `0` / `1` / `2`. A provider, network or credential failure is
`1`. An empty result set is `0`.

SIGPIPE is restored, so `clank … | head` dies cleanly. Output is never coloured:
there is no `NO_COLOR` to honour, and `-q` silences the breadcrumbs.

## Performance / memory

The big bill is the **model server**, not this process (`clank` is a few MB).
Prefer:

- `--thinking off` (or non-TTY default when coding ships it) — thinking tokens
  cost latency/RAM whether shown or not
- cut `--max-tokens` for scripts — default **8192** is the silent fat default
  for one-liners / `--each`
- `--no-tools` when context is already piped — tool rounds multiply requests
- pipe a slice (`sed`/`rg`), not a whole tree via `-c`
- keep `--each` serial on-device; `xargs -P` multiplies **server** VRAM (local
  MLX/omlx can OOM) — not this binary

Measure `/usr/bin/time -v` on `clank` **and** the inference server RSS
separately. Do not compare `clank` RSS to an OpenCode TUI (~hundreds of MB) as
if they were the same product — pipe stage vs agent harness.

**Path footgun:** `--each` items that look like paths are still **text** unless
you `-c` / shell-read content into the prompt. Warn once per run when coding
adds it; until then, compose with `cat`/`rg` yourself.

## One-liners

```sh
# nothing to pipe: ask about the workspace, let the model look around
clank --thinking off -m "where is the transcript cap defined? cite file:line"

# nothing to pipe and nothing to look at: blind on purpose
clank --no-tools -m "write a regex that matches ISO-8601 dates"

# a pipe or a file as context
rg "userData" src/ | clank --thinking off -m "what does this do?"
clank --thinking off -m "explain this" < src/main.rs
sed -n '10,40p' src/main.rs | clank -m "what does this do?"

# context files (repeatable, in order), and a tree
clank -c ctx.json -m "summarize"
clank -c a.json -c b.txt -m "summarize both"
#   [{"text": "…"}, {"file": "rel/path"}, {"children": [{"file": "a.md"}]}]

# one prompt over many items, one framed answer each
rg -n "TODO" src/ | clank --each --thinking off -m "one line: actionable now, or not?"
git log --format=%s -5 | clank --each -m "one line: rewrite in the imperative mood"
find src -name '*.rs' -print0 | clank --each -0 -m "one line: what is this path for?"

# one file per item: the shell reads it, so the model gets content
for f in src/*.rs; do
  clank -c "$f" --thinking off -m "one line: what is this file responsible for?" </dev/null
  echo
done

# typed decisions for routing, branching on the exit code
route=$(printf '%s' "$task" | clank-jev --ask 'What kind of task is this?' --choice code,prose,math --min-prob 0.7)
case "$route" in code) clank --model local-code -m "$task" ;; *) clank --model local-fast -m "$task" ;; esac
printf '%s' "$text" | clank-jev --ask 'Is this a refund request?' --boolean       # prints true|false
printf '%s' "$trace" | clank-jev --checks fixtures/checks-verification.json --min-prob 0.6   # exit 1 = a claim conflicts with the tool results
clank-jev --provider kev --ask 'Which team?' --choice billing,shipping < ticket.txt           # local, no credentials

# web search is its own stage: results on stdout, then a summary
clank-web "rust sigpipe default disposition" | clank -m "summarize with citations"
printf '%s\n' "$query" | clank-web | clank --no-tools -m "summarize with citations"
clank-web --fetch https://example.com | clank -m "one paragraph: what is this page?"

# structured output: one request, grammar enforced by the server
clank -m "…" --json-schema @schema.json | jq -er .
clank --thinking off --json-schema '{"type":"object","properties":{"kind":{"type":"string","enum":["code","docs"]}},"required":["kind"]}' -m "Classify: code or docs."

# reusable instruction block written once, passed every time
clank --system @prompts/review-sh.md -c demo.sh -m "Review the script."

# a trace is context: continue a run, or read it back
clank --jsonl -m "…" | clank -m "what did you say?"
clank -c trace.jsonl -m "what did you read?"
clank --jsonl -m "…" | jq -c 'select(.type=="run")'
```

The two stages compose in both orders — route then generate, generate then
validate, audit a trace afterwards, escalate local to hosted, break a tie. Six
worked examples with their observed answers are in
[docs/clank-jev.md](docs/clank-jev.md#combinations).

## `clank-jev` flags

Credentials come from the environment only.

| flag | meaning |
|---|---|
| `--ask TEXT` | one question; needs exactly one of the three shapes below |
| `--choice A,B,C` | unordered options, printed by name |
| `--boolean` | yes/no, printed as `true` or `false` |
| `--score low,mid,high` | ordered levels, lowest first; prints the level number |
| `--checks FILE` | a JSON file of questions: `{id: {type, instructions, criteria, reasons?}}` |
| `--min-prob P` | exit 1 unless the decision is at least this confident; for a yes/no question a decisive *no* has p≈0 and confidence≈1, so the gate is on the decision, not on "yes" |
| `--expect VALUE` | exit 1 unless the decision equals this (one question) |
| `--expect-min N` | exit 1 unless an ordered decision is at least this level |
| `--print-reason` | print the closed-choice reason instead of the value |
| `--provider NAME` | `auto`, `typesafe`, `openrouter`, `kev` |
| `--model ID` | provider default, or `[clank].model` when the flag is absent |
| `--base-url URL` | provider endpoint, or `[clank].base_url` when the flag is absent |
| `--config PATH` | config file; otherwise `CLANK_CONFIG`, otherwise `./.clank/config.toml` |
| `--timeout SECS` | per-request timeout; otherwise `[clank].timeout`, otherwise 60 |
| `--json` | the full result object instead of the bare value |
| `-q` / `--quiet` | no diagnostic line on stderr |

`TYPESAFE_API_KEY` (or `JEV_API_KEY`, `JEV_CLI_API_KEY`) selects the TypeSafe
route; `OPENROUTER_API_KEY` selects OpenRouter's Decisions endpoint;
`--provider kev` needs no credential. `[clank].api_key_env` is a fallback when
those are unset. `[web]` is left alone. `CLANK_MODEL` and `CLANK_BASE_URL`
belong to `clank`.

## `clank-web` flags

One search, or one GET. stdout is JSONL (`title`, `url`, `snippet`); stderr is
diagnostics. The schema and the sample file are in
[docs/clank-web.md](docs/clank-web.md).

| flag | meaning |
|---|---|
| `--provider NAME` | `brave` (default) or `tavily`; `[web].default_provider` overrides the built-in when the flag is absent |
| `--limit N` | 1..=20, default 5; overrides `[web].limit` |
| `--format jsonl\|text` | JSONL, or `title<TAB>url<TAB>snippet`; default `jsonl` |
| `--base-url URL` | replace the provider endpoint (a proxy, a stub). Ignores `[clank].base_url` |
| `--config PATH` | config file; otherwise `CLANK_CONFIG`, otherwise `./.clank/config.toml` |
| `--fetch URL` | GET one `http` or `https` URL; no search and no key. Body cap 524288 bytes |
| `--timeout SECS` | per-request timeout, default 30 |
| `-q` / `--quiet` | no result-count line on stderr |

`BRAVE_API_KEY` is the Brave subscription token (`X-Subscription-Token`).
`TAVILY_API_KEY` is sent as `Authorization: Bearer`. `[web.brave].api_key_env`
and `[web.tavily].api_key_env` name a different variable. The key stays in the
environment. The default provider stays Brave when only `TAVILY_API_KEY` is set.

```toml
[web]
default_provider = "brave"
limit = 5
format = "jsonl"

[web.brave]
api_key_env = "BRAVE_API_KEY"

[web.tavily]
api_key_env = "TAVILY_API_KEY"
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
| `--config PATH` | `CLANK_CONFIG` | config file; otherwise `./.clank/config.toml` in the working directory |
| `--model` / `--base-url` / `--api-key` | `CLANK_MODEL` / `CLANK_BASE_URL` / `CLANK_API_KEY` | endpoint; otherwise `[clank]` in that file. No built-in model or base URL |
| `--timeout N` | `CLANK_TIMEOUT` | per-request timeout, seconds; otherwise `[clank].timeout` (default 600) |
| `--max-tokens N` | | completion cap; otherwise `[clank].max_tokens` (default 8192) |
| `--max-rounds N` | | tool-call rounds with `--tools`; otherwise `[clank].max_rounds` (default 12) |

`--json-schema` and `--system` accept `@path` to read the payload from a file.

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

- **`--each` items are text, not files.**
  `printf 'src/main.rs\n' | clank --each -m "summarize this file"` shows the
  model the path and nothing else; measured on oMLX (2026-09-19) it answered
  *"This file exists but its purpose is not described in the provided context."*
  Read the files in the shell (the `-c` loop above) or pipe content you already
  gathered.
- **A `while read` loop must give each call its own stdin.** Without
  `</dev/null` the first call eats the rest of the item list.
- **`--json-schema` is the server's grammar to enforce, not clank's.** It is not
  enforced everywhere it is accepted: on oMLX one model returned bare JSON, one
  wrapped it in a fence, one answered in prose. Gate with `jq -er`.
- **A budget overrun is `exit 1`, not a short answer.** `--max-tokens` cutting
  the answer is a failure, so it cannot ship through a `&&` chain.
- **`xargs -P` against one endpoint is not free parallelism.** At four items on
  llama.cpp it won: 2.4 s serial against 1.5 s with `-P2`. On oMLX (2026-09-19)
  it lost: 5.8 s serial against 12.5 s, with the answers interleaving on stdout.
  Measure yours ([docs/use-cases.md](docs/use-cases.md) §3).

## Integrations

```sh
# whatever is on a tmux pane (verified)
tmux capture-pane -p | clank --thinking off -m "one line: what is wrong?"

# output anywhere: a file (write, gate, rename), a FIFO, a socket
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
# herdr: syntax checked against herdr pane --help and herdr agent --help, not executed
herdr pane list | jq -r '.result.panes[].pane_id'
herdr pane split --current --direction right --ratio 0.4
herdr pane run <PANE_ID> 'rg "userData" src/ | clank --thinking off -m "what does this do?"'
herdr pane read <PANE_ID> --source recent
herdr agent prompt <TARGET> "review the diff in src/" --wait
```

`pane run` sends the text and an Enter; there is no `pane wait-output`. For
agent panes, wait with `herdr agent wait <TARGET> --until <status>`.

## Troubleshooting

**Connection refused, or cannot reach `CLANK_BASE_URL`.** stderr is
`request to <url>/chat/completions failed: …` and the exit code is 1. The server
process is down, or the URL is missing the `/v1` path (`clank` appends
`/chat/completions`).

```sh
curl -sf "$CLANK_BASE_URL/models"    # HTTP 200, JSON body
# CLANK_BASE_URL=http://127.0.0.1:8080/v1
```

**Model not found, or the models list is empty.** A bad id comes back as
`model returned HTTP <status>: …` (exit 1). Set `CLANK_MODEL` from
`GET /v1/models`:

```sh
curl -s "$CLANK_BASE_URL/models" | jq -r '.data[].id'
export CLANK_MODEL=$(curl -s "$CLANK_BASE_URL/models" | jq -r '.data[0].id')
```

An empty `.data` array means the server answered and has no model loaded.

**Never `cargo install clank`.** That crates.io name is unrelated. Use:

```sh
cargo install clank-cli-app --locked
```
