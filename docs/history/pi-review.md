# pi, inspected

`earendil-works/pi` (pi.dev): a minimal terminal coding-agent harness by Mario
Zechner (MIT, TypeScript monorepo). Sources: the repository at `main` and the
author's two design posts; every claim carries a path or a quote. Citations in
the two tables were re-read against the source (`output-capture.ts:6-7`, the
bash truncation footers, the `edit-diff.ts` fuzzy path,
`session/jsonl/types.ts:4`, `docs/json.md` "Output Format",
`docs/session-format.md` "Session Version"); the loop's line numbers were
corrected against the file.

pi and clank agree about composition and disagree about state and closure: pi
maximises extensibility, clank maximises honesty.

## What it is

Four npm packages (`packages/{ai,agent,tui,coding-agent}`), four front ends:
interactive TUI, `-p/--print`, `--mode json`, `--mode rpc`, plus an in-process
SDK (`README.md`, "Modes", "Programmatic Usage"). Built-in tools: `read`, `bash`,
`edit`, `write`, `grep`, `find`, `ls` (`--tools read,grep,find,ls` in the CLI
reference). Sessions are append-only JSONL trees in `~/.pi/agent/sessions/`.
Capabilities: TypeScript extensions, skills, prompt templates, themes, npm/git
packages.

## Where pi independently confirms clank's doctrine

> "MCP servers also aren't composable. Results returned by an MCP server have to go through the agent's context to
> be persisted to disk or combined with other results." — *What if you don't need MCP at all?*

Clank's invariant 3 stated as a complaint about tool protocols: data that passes
through the model's context cannot compose. He replaces MCP with "a handful of
CLI scripts plus a README" and reports the cost: Playwright MCP is 21 tools /
13.7k tokens, Chrome DevTools MCP 26 tools / 18.0k tokens, against a
four-command README. His design post adds clank's own premise:

> "context engineering is paramount. Exactly controlling what goes into the model's context yields better
> outputs… Existing harnesses make this extremely hard or impossible by injecting stuff behind your back that
> isn't even surfaced in the UI." — *What I learned building an opinionated and minimal coding agent*

## Good — worth taking

| # | mechanism | evidence | what clank does today |
|---|---|---|---|
| 1 | **A versioned header line starts the event stream** — "The first line is the session header: `{"type":"session","version":3,…}`" | `docs/json.md` (Output Format) | **done 2026-09-17**: `--jsonl` opens with a `run` event carrying clank's version, model, base-url and argv (API keys redacted) |
| 2 | **Fail the whole tool batch when the turn was truncated** — `stopReason === "length"` routes to `failToolCallsFromTruncatedMessage`, "every tool call in the message may carry truncated arguments. Fail them all instead of executing potentially borked calls" | `packages/agent/src/agent-loop.ts:243-245` | clank rejects a truncated round outright (exit 1); if `--tools` survives, take the narrower rule too |
| 3 | **Truncation is communicated, not silent** — results carry `[Showing lines a-b of N … Full output: <path>]` | `tools/bash.ts:114-126`, `tools/read.ts:133-135` | **already done**: `read_file` prints total/shown, `search` prints "(truncated at 500 matches)" (`src/tools.rs:137,142,211`) |
| 4 | **Failure-as-data for tools**: a tool throw becomes `{isError:true}` fed back to the model — "the thrown error is caught, reported to the LLM with `isError: true`" | `agent-loop.ts:763-768`, `docs/extensions.md` | already done (`tools::execute` returns `(text, ok)`) |
| 5 | **Thinking level is a first-class flag** — `--thinking` at `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`, plus `--model sonnet:high` shorthand | `README.md`, "Model Options" | **done 2026-09-17** as `--thinking off\|<level>`, sending `chat_template_kwargs` or `reasoning_effort` and exiting 2 on an unknown level; no generic `--opt` |
| 6 | **Framing rule stated as a contract, with the failure named** — "strict JSONL semantics with LF (`\n`) as the only record delimiter", and Node `readline` is non-compliant because it also splits `U+2028`/`U+2029` | `docs/rpc.md` | clank splits on `\n` in Rust, and **PROTOCOL.md states the rule and names the `readline` failure** |
| 7 | **Honesty about what is not contained** — "Pi does not include a built-in sandbox", naming host shell, filesystem, credentials and extension code as unconfined | `docs/security.md` | clank's `read_file` reaches anywhere the user can read, and **PROTOCOL.md says so**; no `--root` flag exists |
| 8 | **Provenance injected for child processes** — bash-tool commands get `PI_MODEL`, `PI_SESSION_ID`, `PI_PROVIDER`, `PI_REASONING_LEVEL`; children see `AI_AGENT=pi` | `README.md`, "Environment Variables" | clank spawns nothing, so nothing to inject; the same instinct as #1 at process level |
| 9 | **System prompt as named, independently patchable sections** — `<tools>`, `<rules>`, `<project_context>`, diffed by name across turns | `core/system-prompt.ts:46-48,146-172` | not needed for a fixed prompt; the naming idea applies if `--system` grows |
| 10 | **A documented session format replays without the app** — JSONL, `id`/`parentId` tree, `readFileSync` example in the spec | `docs/session-format.md` | clank's trace-as-context is the miniature; the format precedent exists if branching ever matters |

One scout claim needs correcting: pi's OpenAI cache keys and `cache_control`
breakpoints are inapplicable to clank, though not because clank's `--each` fails
to reuse a cache. `--each` keeps every message but the trailing item block
byte-identical, which is what the server-side prefix cache reuses
(`-sps/--slot-prompt-similarity`, see `research-harness-constraints.md` §1);
explicit client-side cache keys are unnecessary, prefix stability is the
mechanism.

## Bad — do not copy

| # | problem | evidence | why it matters here |
|---|---|---|---|
| 1 | **An empty turn is a success.** `hasMoreToolCalls = false` is set before the tool-call check, so a message with no content and no calls reaches `break` and emits a successful `agent_end` | `agent-loop.ts:237,284` | clank exits 1 on an empty answer (`src/main.rs`), its clearest advantage over pi |
| 2 | **Unbounded loop, no turn cap, no duplicate-call detection.** "The agent loop doesn't let you specify max steps or similar knobs… I never found a use case for that, so why add it?" | design post; `agent-loop.ts:177,181`; no `maxTurns` in the tree | correct for an interactive tool with a human present; unsafe for a pipeline stage — keep `--max-rounds` |
| 3 | **The wire format is documentation of internal TypeScript types**, an open union (`AgentEvent` plus provider-specific message subclasses), with no documented stability or exit-code contract; the only guard is `version:3` | `docs/json.md` | clank's event set and exit codes are closed; keep them closed |
| 4 | **Doc/code drift already exists, in both directions**: `docs/json.md` shows the header as `{"type":"session","version":3,…}` and `docs/session-format.md` calls v3 "the current version", while the codec defines `JSONL_FORMAT_VERSION = 4` | `docs/json.md` (Output Format), `docs/session-format.md` (Session Version) vs `session/jsonl/types.ts:4` | clank's `tests/wire.rs` asserts documented shapes over the wire |
| 5 | **Extensions and packages are arbitrary code with full permissions, loaded from npm/git**: "Pi packages run with full system access… Review source code before installing" | `README.md`, "Pi Packages" | no plugins |
| 6 | **Network behaviour beyond the model endpoint**: version check to pi.dev, install telemetry, provider attribution headers, and an invitation to publish OSS sessions to HuggingFace | `README.md`, "Telemetry and update checks" | clank's only egress is the endpoint; document that |
| 7 | **Scale**: four packages, ~30 providers, a TUI framework, and a ~1700-line OpenAI adapter | tree; `packages/ai/src/api/openai-completions.ts` | clank's own binary is ~1.9k lines, 2.8k for the crate with the sibling |
| 8 | **`edit`'s fuzzy fallback silently accepts non-exact matches** (smart quotes, trailing whitespace, NFKC) despite an "exact text" contract | `tools/edit-diff.ts:241-263` | ambiguity should be an error; clank avoids the case by not editing at all |
| 9 | **Token/cache accounting is best-effort by the author's own account**: providers report tokens at different times, abort loses them, some fields are merely absent | design post, "There. Are. Four. Ligh… APIs" | if clank ever prints cost, label it approximate |
| 10 | **UI-shaped machinery in the data path**: `OutputCapture` keeps a buffered view plus an adaptive publisher (100 ms floor, 100 KB/s target) and spills to temp files | `utils/output-capture.ts:6-7`, `bash.ts:84` | a filter has no viewport and writes no files |

## What clank should take, concretely

1. **A `run` header event in `--jsonl`** (pi's #1) — **implemented 2026-09-17**:
   first line `{"type":"run","clank":"0.1.0","model":…,"base_url":…,"argv":[…]}`;
   traces become self-describing evidence.
2. **`--thinking <level>`** (pi's #5) — **implemented 2026-09-17**, mapping to
   per-request `chat_template_kwargs` / `reasoning_effort`; the probe (README,
   *Verified*) shows the template switch is honoured on both local endpoints and
   a level on one of them.
3. **State the LF-only framing rule** (pi's #6) — **done**, in PROTOCOL.md and
   the cheatsheet.
4. **Say what is not contained** (pi's #7) — **done** in PROTOCOL.md (*Neither
   mode is a sandbox*); no `--root`.
5. **If `--tools` survives, fail truncated batches wholesale** (pi's #2).

## What pi means practically for this stack

pi supports the llama.cpp **router server** as a provider (`/login llama.cpp`,
then `/llama` to load models and `/model` to select), tool calling delegated to
the server's `--jinja` flag (`docs/llama-cpp.md`). It requires a router build
started without `--model`/`-hf`; per-model context comes from llama.cpp model
presets.

For agent-shaped work against these endpoints pi already speaks the protocol
these servers speak; clank stays a single, inspectable, stateless stage in a
pipeline.

## Sources

- Repository (pinned at the reviewed commit's tree, `main`):
  https://github.com/earendil-works/pi
- `packages/agent/src/agent-loop.ts`,
  `harness/tools/{bash,read,edit,edit-diff}.ts`,
  `harness/utils/{truncate,output-capture}.ts`,
  `harness/session/jsonl/{types,codec}.ts`, `harness/compaction/compaction.ts`,
  `packages/coding-agent/{README.md,docs/{json,rpc,security,extensions,llama-cpp,session-format,compaction}.md}`
- Mario Zechner, *What I learned building an opinionated and minimal coding
  agent* — https://mariozechner.at/posts/2025-11-30-pi-coding-agent/ ; *What if
  you don't need MCP at all?* —
  https://mariozechner.at/posts/2025-11-02-what-if-you-dont-need-mcp/
