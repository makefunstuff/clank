# `clank` — the model stage

stdout is the answer, stderr is diagnostics, the exit code is the verdict. One
prompt, one request, one answer.

## Flags

`--json-schema` and `--system` accept `@path` to read the payload from a file.

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

`CLANK_DEBUG=/path/req.json clank …` writes the exact request body of the first
round.
