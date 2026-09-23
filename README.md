# clank — minimal unix-style inference harness

A Rust CLI that is one stage in a pipeline: prompt and context in on argv/stdin,
the answer on stdout, diagnostics on stderr, a verdict in the exit code. Speaks
to any OpenAI-compatible endpoint. Sibling `clank-jev` routes and gates —
compose them; do not fold chat/agent UX into `clank`.

![clank in a shell](docs/images/clank-demo.gif)

Recorded against a live server by `scripts/record-demo.sh`.

**Site:** [makefunstuff.github.io/clank](https://makefunstuff.github.io/clank/)
· **Cheat sheet:** [CHEATSHEET.md](CHEATSHEET.md) · **Contract:**
[PROTOCOL.md](PROTOCOL.md)

## Install

Package `clank-cli-app` → binaries `clank` and `clank-jev` (package ≠ binary).
Never `cargo install clank` — that crates.io name is unrelated.

```sh
cargo install clank-cli-app --locked   # -> ~/.cargo/bin/clank and clank-jev
```

Git fallback:

```sh
cargo install --locked --git https://github.com/makefunstuff/clank --tag v0.1.0
```

Prebuilt archives: [releases](https://github.com/makefunstuff/clank/releases).

## 30s smoke

```sh
export CLANK_BASE_URL=http://127.0.0.1:8080/v1   # your OpenAI-compatible server
export CLANK_MODEL=$(curl -s "$CLANK_BASE_URL/models" | jq -r '.data[0].id')
clank -m 'reply with exactly: pong'    # -> pong, exit 0
```

Set `CLANK_BASE_URL` and `CLANK_MODEL` (or `--base-url` / `--model`) yourself.

## Web search

`clank` tools are local filesystem observation only — no web fetch. For the net,
compose in the shell (`curl … | clank -m "…"`). A sibling `clank-web` stage
(search providers + `.clank/config.toml`) is the planned pipe for
provider-backed search; it will not fold network into `clank`'s tool loop.

## Harness cost (dated)

**2026-09-22** one-shot on the same Go model (`opencode-go/glm-5.3-flash`),
tools off, prompt `→ pong`. **Pipe vs agent CLI**, not a quality benchmark —
agent harnesses still pay TUI/runtime tax with `--no-tools`.

| | wall median | peak RSS |
|---|---:|---:|
| **clank** (pipe) | **0.75 s** | **~5 MB** |
| pi | 2.2 s | ~177 MB |
| omp | 3.4 s | ~371 MB |
| opencode | 5.7 s | ~562 MB |

Method and caveats:
[docs/history/clank-vs-agents-2026-09-22.md](docs/history/clank-vs-agents-2026-09-22.md).

Token knobs (`--thinking off`, lower `--max-tokens`, `--no-tools` when piped):
[CHEATSHEET.md](CHEATSHEET.md) · latency notes in
[docs/history/research-harness-constraints.md](docs/history/research-harness-constraints.md).

## More

| | |
|---|---|
| [CHEATSHEET.md](CHEATSHEET.md) | flags, one-liners, herdr compose |
| [PROTOCOL.md](PROTOCOL.md) | invariants, exit codes, events |
| [docs/use-cases.md](docs/use-cases.md) | job families, gates, prices |
| [docs/clank-jev.md](docs/clank-jev.md) | typed routing decisions |
| [docs/history/](docs/history/README.md) | dated measures and closed research |


JSONL event kinds (`--jsonl`): `run`, `item`, `tool_call`, `tool_result`,
`assistant`, `error`. See [PROTOCOL.md](PROTOCOL.md).

## Flags (index)

Meanings live in [CHEATSHEET.md](CHEATSHEET.md). Listed so the surface stays
short without drifting from `--help`.

`clank`: `--api-key`, `--base-url`, `--context`, `--each`, `--json-schema`,
`--jsonl`, `--list-tools`, `--max-rounds`, `--max-tokens`, `--message`,
`--model`, `--no-tools`, `--null`, `--quiet`, `--show-thinking`, `--system`,
`--thinking`, `--timeout`, `--tools`

`clank-jev`: `--ask`, `--base-url`, `--boolean`, `--checks`, `--choice`,
`--expect`, `--expect-min`, `--json`, `--min-prob`, `--model`, `--print-reason`,
`--provider`, `--quiet`, `--score`, `--timeout`

## License

MIT. See [LICENSE](LICENSE).
