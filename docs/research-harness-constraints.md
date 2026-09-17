# Minimal harnesses, and the constraints the model already carries

Research note, 2026-09-17. Question: how do you keep a model harness minimal
*and* usable, by leaning on what the model and the serving stack already
guarantee instead of writing the guarantees yourself?

**Verdict.** The doctrine is consistent across every primary source checked:
the harness should be a thin loop around the model's *native* format, and the
lever that matters is not more code but fewer invented protocols. The failure
mode is not minimalism — it is over-constraining. Format restrictions measurably
degrade reasoning, and the more a harness constrains, the more it costs.

Labels: **[P]** primary documentation, **[V]** verified on this machine,
**[I]** inference from the above, **[U]** unverified.

## 1. What the model and the server already guarantee

Everything in this table is something a harness does *not* have to implement —
provided it uses the native path rather than inventing a format.

| capability | who provides it | local status |
|---|---|---|
| chat framing (roles, control tokens, generation prompt) | the model's Jinja chat template, rendered server-side; `--jinja` is **default enabled** in llama-server **[P]** | templates fetched from `/props`: 8953 B (`:40583`) and 8058 B (`:37313`) **[V]** |
| tool-call syntax | the template's native format: Hermes 2/3 (Qwen 2.5 family), Llama 3.x, Mistral Nemo, Functionary, FireFunction, Command R7B, DeepSeek R1; a "Generic" fallback exists for unrecognised templates **[P]** | both endpoints declare `supports_tools`, `supports_tool_calls`, `supports_object_arguments`, `supports_parallel_tool_calls` **[V]** |
| parsing tool calls out of the stream | the server, driven by the template (`--skip-chat-parsing` exists to *disable* it) **[P]** | clank reads `tool_calls` deltas and never parses markup itself **[V]** |
| a separate reasoning channel | template + `--reasoning-format` (`deepseek` → thoughts in `reasoning_content`, `auto` default) **[P]** | clank drops `reasoning_content` and emits only `content` **[V]** |
| reasoning control, per request | `chat_template_kwargs` (README: *"For example: `{"enable_thinking": false}`"*) and `reasoning_effort` (*"If `none`, reasoning/thinking is disabled"*) **[P]** | clank sends neither; thinking is therefore on by template default **[V]** |
| schema-constrained output | grammar-based sampling from `json_schema` (a **native `/completion`** parameter), and `response_format: {"type":"json_schema","schema": …}` on the chat route; the chat route also documents `reasoning_format`, `parse_tool_calls`, `parallel_tool_calls` and `reasoning_control` **[P]** | clank's **top-level `json_schema` is enforced** on `/v1/chat/completions` — probed 2026-09-17: instructed to reply `beta` under a schema whose only legal value is `alpha`, both endpoints answered `{"word":"alpha"}` **[V]**. In llama.cpp's *documented* `response_format` shape the same schema was **not** enforced on `:40583` **[V]** |
| prefix/KV reuse | prompt caching on by default, slot similarity `-sps 0.10`, idle-slot caching; `return_progress` reports `prompt_progress` with `total`, `cache`, `processed`, `time_ms` **[P]** | **measured** 2026-09-17: a repeat request reported `cache: 6318` of `total: 6322` prompt tokens with `time_ms` falling 8199 → 129; an `--each` run through clank showed 93 then 103 cached of 127 **[V]** |
| assistant prefill / continuation | `continue_final_message` (HF), and server-side assistant prefill is a listed feature **[P]** | unused by clank **[V]** |
| tool calling and a schema at once | the server parses both, but the combination is refused here **[P]** | clank observed a 400 when `tools` and `json_schema` are sent together, and split the requests accordingly **[V]** |

Two consequences follow directly. First, the model only *requests* a tool call —
execution is the application's job, which is exactly the boundary clank draws.
Second, `parallel_tool_calls` is off unless the request asks for it, and both
local templates advertise support, so parallel calls are a request field away
rather than a harness feature.

## 2. What no amount of training covers

These stay in the harness no matter how thin it gets, and each one is a place
where a minimal harness can still lie:

- **truthful exit codes** — truncation at the token budget and an empty turn are
  not results; nothing in the model's training makes a partial answer
  self-reporting. (llama.cpp signals `finish_reason: "length"`; clank turns that
  into exit 1.)
- **limits** — `max_tokens`, per-request timeouts, round caps: the model cannot
  bound its own cost.
- **the composition contract** — stdout is data, stderr is diagnostics, exit
  codes are 0/1/2. The model has no opinion about pipelines.
- **the capability boundary itself** — which tools exist, and therefore what the
  model is allowed to do, is a harness decision, not a model behaviour.
- **provenance** — which model, which endpoint, which version produced a trace.
- **error surfacing** — a tool that raises must become data the model can react
  to, not a crash, and the model's malformed output must not be silently
  repaired into a success.

Anthropic's guidance covers the same ground from the other side: keep a hard
budget, make invalid actions difficult, add deterministic checks, and *"give the
model enough tokens to think before it writes itself into a corner"* **[P]** —
which is the argument for treating truncation as failure rather than as output.

## 3. Evidence against over-constraining

This is the strongest empirical finding in the set, and it argues against the
temptation to wrap everything in schema:

- **"Let Me Speak Freely? A Study on the Impact of Format Restrictions on
  Performance of Large Language Models"** (Tam et al., arXiv 2408.02442, 2024)
  finds *"a significant decline in LLMs reasoning abilities under format
  restrictions"* and that *"stricter format constraints generally lead to
  greater performance degradation in reasoning tasks"* **[P]**.
- Anthropic's tool-design appendix says to *"keep the format close to what the
  model has seen naturally occurring in text on the internet"* and warns against
  *"formatting overhead such as having to keep an accurate count of thousands of
  lines of code, or string-escaping any code it writes"* — i.e. wrapping code in
  JSON costs escapes **[P]**.
- llama.cpp frames its generic fallback the same way: native formats are
  cheaper and more reliable, generic handling *"may consume more tokens and be
  less efficient than a model's native format"* **[P]**.

For clank this validates a choice already made: the tool rounds run
**unconstrained**, and exactly one final request carries the schema. Constraining
the exploration phase would buy nothing and, on this evidence, cost reasoning.

## 4. The minimal-harness doctrine, as the sources state it

- Anthropic, *Building effective agents* (Dec 2024): *"the most successful
  implementations use simple, composable patterns rather than complex
  frameworks"*; *"start by using LLM APIs directly: many patterns can be
  implemented in a few lines of code"*; frameworks *"often create extra layers of
  abstraction that can obscure the underlying prompts and responses"*; and
  *"incorrect assumptions about what's under the hood are a common source of
  customer error"* **[P]**.
- Their tool-design advice is a harness rule, not a prompt rule: they changed
  their file tool to *require absolute paths* and *"found that the model used
  this method flawlessly"* — the interface shape carried the reliability **[P]**.
- HuggingFace, *Chat templates*: a chat is *"still just a sequence of tokens"*;
  the template defines the control tokens, and *"with the wrong control tokens,
  these models would have drastically worse performance"* **[P]**.
- The same document documents `continue_final_message` as *"prefilling a model
  response… very useful for improving the accuracy of instruction following when
  you know how to start its replies"* — a trained continuation affordance a thin
  harness can use instead of a retry loop **[P]**.

## 5. Findings specific to this machine

Verified locally against both running llama-server processes
(`ps -ww -o args=`, `curl /props`); server build `b2493-17252c769`.

| | `:40583` (qwen3.8-27b) | `:37313` (qwen3.6-35b-a3b) |
|---|---|---|
| `supports_tools` / `tool_calls` / `object_arguments` | true | true |
| `supports_parallel_tool_calls` | true (not request-enabled) | true (not request-enabled) |
| `supports_preserve_reasoning` | true | true |
| `supports_reasoning_effort` | **true** | **false** |
| reasoning flags | `--reasoning auto --reasoning-budget -1 --reasoning-effort medium` | `--reasoning on --reasoning-budget 2048` |
| KV cache quantisation | `-ctk q4_0 -ctv q4_0` | `-ctk q4_0 -ctv q4_0` |

Three things worth acting on:

1. **A warning that concerns the server, not the harness.** llama.cpp's
   function-calling doc says: *"Beware of extreme KV quantizations (e.g. `-ctk
   q4_0`), they can substantially degrade the model's tool calling
   performance."* **[P]** Both endpoints run `q4_0` for K and V **[V]**.
   Recorded because it explains a possible tool-calling symptom, **not** because
   clank can act on it: no request clank sends changes how the server allocates
   or quantizes its cache. Server configuration is deliberately out of this
   project's scope.
2. **Reasoning: on by default, and only one knob is portable.** Probed
   2026-09-17: `chat_template_kwargs: {"enable_thinking": false}` removes
   reasoning on **both** endpoints (0 reasoning bytes), while
   `reasoning_effort` is honoured on `:40583` (69→35 chars at `low`) and
   **silently ignored** on `:37313` (306→308 chars at `none`, i.e. no effect and
   no error) despite `supports_reasoning_effort: false` being advertised there.
   clank's `--thinking off` therefore uses the template switch, and a level is
   documented as advisory rather than a promise.
3. **The server can now run the tool loop itself.** llama.cpp ships an
   experimental built-in toolset — `read_file, file_glob_search, grep_search,
   exec_shell_command, write_file, edit_file, get_info` — with
   `--tools-runtime docker:<image>` / `ssh:<target>` for isolation, plus MCP
   server config **[P]**. It is a superset of clank's four read-only observers,
   sandboxed where clank is not. That is a duplication argument, not a feature
   argument: the honest options are to keep clank's loop as a deliberately
   weaker observer, or to drop it and let the server own tools.
4. **Resolved by probe (2026-09-17): clank's schema spelling is the enforced
   one.** Top-level `json_schema` overrode an explicit instruction on both
   endpoints (`{"word":"alpha"}` for a prompt saying `beta`), so the two-phase
   design rests on a real grammar. The *documented* chat-route spelling
   (`response_format: {"type":"json_schema","schema": …}`) did **not** enforce
   it on `:40583` — the model simply answered `beta`. (OpenAI's nested
   `response_format.json_schema.name/schema` shape was not tried.) Practical
   consequence: keep the top-level field, and treat the documented alternative
   as unverified on this build.

5. **`:37313` leaks a closing thinking tag into the answer.** Observed
   2026-09-17 while running `demo.sh`: with thinking on, a strict-output prompt
   (*"output ONLY the script text"*) returned content beginning with a literal
   `</think>` line, which a `bash -n` gate then rejected. `reasoning_content`
   parsing works for ordinary prompts on that endpoint (probe: 306 reasoning
   chars, clean content), so this is a template/parser edge case on structured
   answers rather than a general failure. clank passes model text through
   verbatim — stripping tags would be rewriting the answer — so the demo runs
   every stage with `--thinking off`, which is the portable switch (see item 2 above).

### 5.1 The KV question, costed (server-side, for completeness)

Recorded so the warning above has a size attached to it — clank cannot act on
any of it. The warning would be expensive on a dense model with a full KV cache
on every layer; these two models are not that. Metadata read from the GGUF files
(`gguf-py` reader, `/data/src/llama.cpp/gguf-py` +
`~/.venvs/ruview/bin/python`; tensor names, not guesses):

| | `:40583` qwen3.8-27b (`qwen35`) | `:37313` qwen3.6-35b-a3b (`qwen35moe`) |
|---|---|---|
| blocks | 65 | 41 |
| blocks with `attn_k`/`attn_v` (**growing KV cache**) | **17** (every 4th, plus the last) | **11** (every 4th, plus the last) |
| blocks with linear-attention / SSM tensors | 48 | 30 |
| `head_count_kv` × (`key_length`+`value_length`) | 4 × 512 | 2 × 512 |
| KV values per token | 17 × 2048 = **34,816** | 11 × 1024 = **11,264** |

Three quarters of the layers carry a fixed-size recurrent state, not a KV cache,
so the KV is only 17/65 and 11/41 of the depth. At the configured
`--ctx-size 131072`, one shared pool (`--kv-unified`):

| K+V quantisation | bytes/value | `:40583` | `:37313` |
|---|---|---|---|
| `q4_0` (current) | 0.5625 | **2.39 GiB** | **0.77 GiB** |
| `q8_0` | 1.0625 | 4.52 GiB (**+2.13**) | 1.46 GiB (**+0.69**) |
| `f16` | 2.0 | 8.50 GiB (**+6.11**) | 2.75 GiB (**+1.98**) |

So undoing the flagged quantisation on the 27B endpoint costs about **2.1 GiB
of VRAM**, not the 6–8 GiB it would cost on a dense 27B — and about **0.7 GiB**
on the MoE endpoint. These are computed from metadata, not measured: both
servers were observed **asleep** (`--sleep-idle-seconds 300`; `nvidia-smi`
showed 1.8 GiB / 0.17 GiB in use), so no loaded footprint was readable at the
time of writing **[U]**.

Two caveats on the numbers. The weights themselves also have to fit — the 27B
file is 10.4 GB (`-ngl all`, KV now ~2.4 GiB, so q8_0 KV lands near 14.5 GiB of
a 16 GiB card, which fits with little room). And `--kv-unified` allocates the
pool for the whole context at startup, so the cost is paid whether or not the
context is used.

### 5.2 Where the latency is, measured

All numbers on this box, 2026-09-17, `--thinking off` unless stated, one warm
endpoint, answers deliberately tiny so the model's own generation cost is not
the subject. "First output" is the time until the first byte reaches stdout.

| what varies | first output | total | factor |
|---|---|---|---|
| thinking on (trivial prompt) | 1.35 s | 1.35 s | — |
| `--thinking off` | 0.25 s | 0.25 s | **5× faster** |
| 6k-token prefix, cold | 10.39 s | 10.39 s | — |
| same prefix, second call | 0.21 s | 0.21 s | **50× faster** |
| full answer (200 lines, 127 tokens) | — | 7.10 s | — |
| `\| head -1` (consumer stops) | 0.60 s | 0.60 s | **12× faster** |
| 4 items, serial `--each` | — | 2.42 s | — |
| 4 items, `xargs -P2` | — | 1.50 s | 1.6× |
| 4 items, one batched call | — | 1.64 s | 1.5× |

**clank's own share of that**, measured against a stub endpoint that answers
instantly (no inference), median of 5–7 runs:

| clank work | time |
|---|---|
| process startup (`--list-tools`, no request) | **1.0 ms** |
| one request, no context | 1.4 ms |
| one request, 1 MB piped context | 7.6 ms |
| one request, 10 MB piped context | 35.7 ms |
| `--each`, 1 item, 1 MB shared context | 6.4 ms |
| `--each`, 20 items, 1 MB shared context | 32.2 ms (≈1.4 ms per extra item) |

A first pass measured 50.6 s for the serial arm because the endpoint had gone to
sleep (`--sleep-idle-seconds 300`) and the first request paid a model load;
re-measured warm it is 2.42 s. The largest single latency number observed all
day was that load, not anything in the harness.

### 5.3 clank vs headless pi, same endpoint, same work

Measured 2026-09-17 against `:40583`, both with `--thinking off`, both replying
to the same prompt. pi (0.85.1) was pointed at the local server through a
temporary `~/.pi/agent/models.json` (created for the test, since removed).

| arm | trivial answer ("reply exactly: pong") | 100-line answer |
|---|---|---|
| clank | **0.18 s** | **5.8 s**, 5.8 s |
| pi `-p` | 0.80 s | 7.2 s, 9.1 s |
| pi `-p --no-session --no-context-files -nt` | 1.06 s | — |
| clank process startup (`--list-tools`) | 1 ms | — |
| pi startup (`--version`) | 0.21 s | — |

Where the difference comes from, in pi's own numbers: for a five-word question
pi reports a prompt of about **2 600 tokens** (2 598 of them cache reads) against
clank's ~120, because its system prompt carries the tool set and the rules, and
it persists a session per run. So the gap is *fixed overhead* — a node runtime,
config and session machinery, and a 20× larger prompt — not a tuning difference:
4.4× on a trivial answer, 1.25–1.6× on an identical 100-line answer, converging
as generation time grows. For a pipeline of many small calls (50 items with
`--each`) that fixed cost is paid 50 times.

That overhead is also the feature list we chose not to have — sessions, tools,
compaction, model routing — so the honest reading is not "clank is faster" but
"clank does less, on purpose, and the speed is a by-product of the contract".

## 6. What this implies for clank

| change | why | evidence |
|---|---|---|
| ~~`--opt key=<json>`~~ → implemented as `--thinking off|<level>` (2026-09-17) | the per-request fields are documented, and a fixed enum is more discoverable than a generic hatch; `seed`/`temperature` remain unexposed | **[P]** server README |
| `--show-thinking` → `reasoning_content` on stderr (**implemented** 2026-09-17) | the server already parses it; clank used to discard it, so thinking was paid for invisibly | **[V]** |
| ~~cost line on stderr from `return_progress` + `stream_options`~~ — **implemented, then removed** (2026-09-17): it made the prefix-cache claim measurable, and having measured it, clank sends plain OpenAI-compatible requests again and leaves token accounting to the server's log | the measurement survives in §5.2; the field does not | **[V]** |
| keep the two-phase schema (unconstrained rounds → one constrained answer) | format restrictions degrade reasoning; do not constrain exploration | **[P]** arXiv 2408.02442 |
| ~~probe whether the chat route applies the `json_schema` grammar~~ — **resolved**: the top-level field is enforced, `response_format` is not | "the server enforces the schema" is load-bearing for the two-phase design; it now holds on evidence | **[V]** probe |
| keep truncation and empty-turn as failures | nothing in training makes a partial answer self-reporting | **[P]** Anthropic Appendix 2 |
| reconsider `--tools` | the server owns a sandboxable superset; clank's loop is duplicated capability | **[P]** server README |
| ~~test `-ctk/-ctv q8_0`~~ — **dropped: server configuration, out of scope for the harness** | clank's requests are unaffected by how the server sizes or quantizes its cache; §5.1 keeps the measurement only as context for the warning | — |

## 7. Limitations of this note

- Search providers failed for the function-calling leaderboards (BFCL and
  similar), so the "native formats beat generic" claim rests on llama.cpp's own
  statement and HF's warning, not on an independent measurement.
- Everything above marked **[V]** was observed on 2026-09-17 by probing both
  running endpoints directly (schema enforcement, reasoning knobs, streamed
  usage, cache progress), with the probe scripts kept out of the repo. Still
  unobserved: the KV-quantisation warning, which needs a server restart with
  different flags.
- The KV-quantisation warning is llama.cpp's; whether it bites on this box is
  unmeasured.
- `stream_options: {"include_usage": true}` does not appear in the server README
  I read, but it **works**: probed 2026-09-17, both endpoints returned
  `usage.completion_tokens` and `prompt_tokens_details.cached_tokens` in the
  final chunk when it was requested, and returned no usage at all when it was
  not **[V]**.

## Sources

- Anthropic, *Building effective agents* —
  https://www.anthropic.com/engineering/building-effective-agents
- HuggingFace, *Chat templates* —
  https://github.com/huggingface/transformers/blob/main/docs/source/en/chat_templating.md
- HuggingFace, *Tool use* —
  https://huggingface.co/docs/transformers/en/chat_extras
- llama.cpp, *Function calling* —
  https://github.com/ggml-org/llama.cpp/blob/master/docs/function-calling.md
- llama.cpp, *HTTP server* (flags, request/response fields) —
  https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md
- Tam et al., *Let Me Speak Freely?* — https://arxiv.org/abs/2408.02442
- OpenAI, *Structured model outputs* —
  https://developers.openai.com/api/docs/guides/structured-outputs
