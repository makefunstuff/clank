# clank — minimal unix-style inference harness

Three stages. `clank-jev` is the typed gate. `clank-web` fetches onto stdout.
`clank` is the model stage: prompt and context in on argv/stdin, the answer on
stdout, diagnostics on stderr, a verdict in the exit code. It speaks to an
OpenAI-compatible endpoint. Its tools stay on the local filesystem.

![clank in a shell](docs/images/clank-demo.gif)

Recorded against a live server by `scripts/record-demo.sh`.

**Site:** [makefunstuff.github.io/clank](https://makefunstuff.github.io/clank/)
· **Cheat sheet:** [CHEATSHEET.md](CHEATSHEET.md) · **Contract:**
[PROTOCOL.md](PROTOCOL.md)

## Install

Package `clank-cli-app`. Binaries in this tree: `clank`, `clank-jev`,
`clank-web`. `cargo install clank` installs a different crates.io crate.

crates.io `clank-cli-app` 0.1.0, published 2026-09-22 from `fb6067d`, installs
`clank` and `clank-jev`. crates.io installs `clank-web` only after a `v0.1.1` or
later publish has passed a three-bin smoke. Until that publish, `clank-web` is a
git install. `--tag v0.1.0` is the two-binary tag; the package name in that
tag's `Cargo.toml` is `clank`. `--tag` on `v0.1.1` or later is the pin after
that smoke.

```sh
cargo install clank-cli-app --locked --version 0.1.0
cargo install --locked --git https://github.com/makefunstuff/clank
cargo install --locked --git https://github.com/makefunstuff/clank --tag v0.1.0
```

`--version 0.1.0` and `--tag v0.1.0` install `clank` and `clank-jev`. The git
line with no tag follows `main` and installs `clank`, `clank-jev`, and
`clank-web`.

`v0.1.0` release archives are the two binaries:
[releases](https://github.com/makefunstuff/clank/releases).

## 30s smoke

```sh
export CLANK_BASE_URL=http://127.0.0.1:8080/v1   # your OpenAI-compatible server
export CLANK_MODEL=$(curl -s "$CLANK_BASE_URL/models" | jq -r '.data[0].id')
clank -m 'reply with exactly: pong'    # -> pong, exit 0
```

Set `CLANK_BASE_URL` and `CLANK_MODEL` (or `--base-url` / `--model`, or
`[clank]` in `.clank/config.toml`) yourself.

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
[docs/clank.md](docs/clank.md). Latency notes:
[docs/history/research-harness-constraints.md](docs/history/research-harness-constraints.md).

## More

| | |
|---|---|
| [CHEATSHEET.md](CHEATSHEET.md) | route, fetch, generate: one recipe each |
| [PROTOCOL.md](PROTOCOL.md) | invariants, exit codes, events |
| [docs/use-cases.md](docs/use-cases.md) | job families, gates, prices |
| [docs/clank.md](docs/clank.md) | model-stage flags |
| [docs/clank-jev.md](docs/clank-jev.md) | typed-gate flags |
| [docs/clank-web.md](docs/clank-web.md) | fetch flags |
| [docs/history/](docs/history/README.md) | dated measures and closed research |


JSONL event kinds (`--jsonl`): `run`, `item`, `tool_call`, `tool_result`,
`assistant`, `error`. See [PROTOCOL.md](PROTOCOL.md).

## License

MIT. See [LICENSE](LICENSE).
