# verification log

Dated records of what was run, against which endpoint and model, and what came
back. The [README](../../README.md)'s *Verified* section is the summary; this
file is the evidence behind it, kept so a claim can be checked months later
without re-reading the commits.

New records are appended here and summarised in the README, not the other way
round: this is the only place a date, a model name and a quoted answer belong.

- Endpoints: qwen 3.8 27b (Qwen3.8-27B GSQ-RCO IQ3_XXS) on `:40583` and qwen 3.6
  35B-A3B (Qwen3.6-35B-A3B-UD-Q5_K_S) on `:37313`, both with SSE streaming and
  tool calling, plus a remote gateway on `:4000`. On 2026-09-19, oMLX 0.7.0 on
  `:8000` serving `Qwen3.5-9B-MLX-4bit`, `MiniCPM5-2B-MLX-8bit` and
  `gemma-4-E4B-it-MLX-4bit` on a 16 GB M1 Pro: all three answer, stream and
  expose reasoning; the fourth listed model, `Bonsai-2-27B-CRACK-1.75bit-JANG`,
  loads on no request at all — the runtime answers 409, and clank prints the
  server's reason verbatim, 402 parameter names included. Details and raw output
  in [macbook-omlx-local-inference.md](../macbook-omlx-local-inference.md).
- **A top-level `json_schema` is the endpoint's grammar to enforce, not
  clank's.** Against the local endpoints it held: asked to reply `beta` under a
  schema whose only legal value was `alpha`, clank printed `{"word":"alpha"}`.
  Through the gateway (2026-09-19) it did not: a fenced block came back with the
  wrong key, and clank exited 1 with *final output is not valid JSON;
  --json-schema requested*. The contract held either way — a violation is a
  failure, not a result — but assume the grammar holds only where it has been
  measured. Per model on oMLX the same day: `Qwen3.5-9B-MLX-4bit` returns bare
  valid JSON; `gemma-4-E4B-it-MLX-4bit` wraps it in a Markdown fence and
  `MiniCPM5-2B-MLX-8bit` answers in prose, both `exit 1`. A schema and tools
  never share a request; with `--tools` the rounds run unconstrained and one
  final request carries the schema. This server build rejects the pair (400, by
  curl probe).
- The tools fallback exists because of a measured failure. With nothing piped
  and no tools,
  `clank -m "which file defines the transcript rendering, and what is the output cap? cite file:line"`
  answered *"`src/renderer.ts`, cap 8000 at `src/renderer.ts:14`"* — a file that
  does not exist, in the citation format of a real answer, exit 0. The same
  question with the tools offered answered `src/context.rs:68` and cap `400` at
  `src/context.rs:136`, with the test that asserts it. On oMLX the 2B reproduced
  that invention *with* the tools on: two lookups in the stderr breadcrumbs,
  then `src/renderer.ts:14` again. What survives a model that fabricates is the
  exit code and the observable channel, not the feature.
- `--thinking off` arrives as `chat_template_kwargs.enable_thinking=false` and a
  level as `reasoning_effort`, with no field at all by default and exit 2 for an
  unknown level. It removes reasoning on both local endpoints; an effort level
  is honoured by one and silently ignored by the other. Reasoning reaches stderr
  only under `--show-thinking`, and stdout stays the answer alone.
- `cargo test` drives the real binary against a stub SSE server that records
  every request body, and asserts what this README and PROTOCOL.md claim: the
  default is one request with no tools; a schema and tools never share a
  request; the schema'd round carries the tool results it read; piped evidence
  is still context when `-c` is used; `--each` frames one answer per item
  (byte-exact) and shares the prompt prefix; a failed item is an `error` event
  plus exit 1 while the others are still answered; an empty item list exits 0; a
  truncated or empty answer exits 1 rather than reporting success; the `run`
  event's argv has any `--api-key` redacted; a trace with a `run` header still
  reads back as context.
- `clank-jev` is verified live two ways: against hosted Jev through OpenRouter's
  Decisions endpoint — `fixtures/checks-verification.json` returned
  `instruction_conflict` and `verification_contradiction`, both at p ≥ 0.93, in
  one request — and against a local `kev-0.6b` on `:8009`, where 18 typed
  decisions scored 16/18 with a 17% shuffled-context control at an 83 ms median
  and no credentials. Its contract is guarded the same way clank's is:
  `tests/docs.rs` reads its `--help` and requires every flag to appear in
  README.md and CHEATSHEET.md, and requires PROTOCOL.md to document exit code 3.
- `demo.sh` ran end to end on 2026-09-17 against `:37313`: four stages, 61 s,
  all gates passed. Its first run found a real defect — an ungated stage wrote a
  broken candidate and the script still announced success. On 2026-09-19 against
  oMLX it passed on `Qwen3.5-9B-MLX-4bit` in 35 s, and stopped at a different
  gate on each of the other two: gemma on the fenced schema at stage 3, the 2B
  on a truncated answer at stage 1.
- Live: tree context renders text, file and nested nodes in document order;
  `--jsonl` parses with `jq`; repeatable `-c` concatenates files in order
  (checked through the `CLANK_DEBUG` request dump); `--system` appends the
  directive to the system prompt; `clank --jsonl | clank` and
  `clank -c trace.jsonl` render the prior run as a tagged transcript;
  `clank … | head -1` exits 0; `--list-tools` prints the four definitions
  without a model call; no prompt with empty stdin exits 2, an unreadable
  context file exits 1, `--help` exits 0.
- `--tools` is no longer unexercised: the oMLX runs sent it through the round
  loop end to end, with the lookups on stderr as promised. Still unexercised
  against a live model: the schema-after-tools sequence, where the wire tests
  cover the request and output contract only. TTY-with-no-prompt reads one line,
  in code; it is not exercisable headless.

## What is not verified

Anything this log does not name. In particular: the schema-after-tools sequence
has never run against a live model (the wire tests cover the request and output
contract only), the TTY-with-no-prompt path is exercised in code and is not
headless-testable, and the numbers from oMLX, the GGUF endpoints and the gateway
were measured on this machine on the dates given — they are records, not
re-runnable claims.
