# clank cheatsheet

`clank` is a unix filter: prompt and context in on argv/stdin, data out on
stdout, diagnostics on stderr. One prompt, one request, one answer, and the
context is exactly what was piped.

- [PROTOCOL.md](PROTOCOL.md) — invariants, request sequence, event set, exit
  codes
- [README.md](README.md) — install, 30s smoke, dated harness cost
- [docs/use-cases.md](docs/use-cases.md) — jobs with their gates and prices
- [docs/clank-jev.md](docs/clank-jev.md) — decision flags, gates, combinations
- [docs/clank-web.md](docs/clank-web.md) — search flags, providers, one-URL
  fetch
- [docs/macbook-omlx-local-inference.md](docs/macbook-omlx-local-inference.md) —
  local models on one 16 GB Mac

Set `CLANK_BASE_URL` and `CLANK_MODEL`, or `[clank]` in `./.clank/config.toml`.
That file is the working directory only. `--config PATH` or `CLANK_CONFIG` names
a different file. Precedence is flags, then the environment, then the file, then
built-ins. There is no built-in model or base URL. Web search keys live under
`[web]` and are read by `clank-web`. The key stays in the environment;
`api_key_env` names the variable.

## Install

Package `clank-cli-app`. Binaries in this tree: `clank`, `clank-jev`,
`clank-web`. `cargo install clank` installs a different crates.io crate.

crates.io `clank-cli-app` 0.1.0, published 2026-09-22 from `fb6067d`, lists
`clank` and `clank-jev` in that crate's `Cargo.toml`. `clank-web` is in this git
tree and is absent from that crate. No `v*` tag contains `clank-web`. The
crates.io cut that includes `clank-web` is not published.

```sh
cargo install clank-cli-app --locked --version 0.1.0
cargo install --locked --git https://github.com/makefunstuff/clank
```

`--version 0.1.0` installs the crates.io cut (`clank`, `clank-jev`). The git
line follows the default branch (`main`) and installs `clank`, `clank-jev`, and
`clank-web`.

```sh
cargo install --locked --git https://github.com/makefunstuff/clank --tag v0.1.0
```

That tag installs `clank` and `clank-jev`. The package name in that tag's
`Cargo.toml` is `clank`. `--git` selects this repository. Release archives for
`v0.1.0` are the same two binaries:
[releases](https://github.com/makefunstuff/clank/releases).

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

## Compose

One path per stage. Flag tables: `clank` below, `clank-jev` in
[docs/clank-jev.md](docs/clank-jev.md), `clank-web` in
[docs/clank-web.md](docs/clank-web.md).

### `clank`

```sh
rg "userData" src/ | clank --thinking off -m "what does this do?"
```

Jobs, gates, and the dated prices are in [docs/use-cases.md](docs/use-cases.md).

### `clank-jev`

```sh
route=$(printf '%s' "$task" | clank-jev --ask 'What kind of task is this?' \
          --choice code,prose,math --min-prob 0.7)
case "$route" in
  code) clank --model local-code -m "$task" ;;
  *)    clank --model local-fast -m "$task" ;;
esac
```

### `clank-web`

```sh
clank-web "rust sigpipe default disposition" | clank -m "summarize with citations"
```

## Flags

`clank` only. `--json-schema` and `--system` accept `@path` to read the payload
from a file.

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

Debug: `CLANK_DEBUG=/path/req.json clank …` writes the exact request body of the
first round.

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

**`cargo install clank` installs a different crates.io crate.** This
repository's package is `clank-cli-app`. The binaries each install command
produces are in [Install](#install).
