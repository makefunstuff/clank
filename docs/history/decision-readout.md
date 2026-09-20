# Decision readout: option probabilities instead of generated answers

Research note, 2026-09-17, prompted by
[TheoLeeCJ/openjev](https://github.com/TheoLeeCJ/openjev) (MIT, created
2026-09-16, 631 stars at the time of reading).

**Verdict: the pattern is worth having in clank; the implementation is not
portable to it.** OpenJev needs full-vocabulary logits from a local PyTorch
model. This server already exposes the same readout over its OpenAI-compatible
route (verified below), so a `--decide` mode would take about 50 lines: no new
dependency, no model download, no second inference stack.

## The two projects

| project | what it is |
|---|---|
| Jev (TypeSafe, announced 2026-09-14) | a closed service for *typed* decisions: send state, a criterion and options; get option probabilities rather than prose |
| OpenJev | an independent reproduction of that *interface pattern* with open models; its README states it does not reproduce Jev's model or training. The reproducible part is the interface and the readout method |

## The method, as implemented

From `src/openjev_phase1/core.py` and `direct.py`, read at `master`:

| step | what it does |
|---|---|
| input row | `{id, state, question, options:[{id, description}]}`; state may be a string, object or array; 2–16 options |
| prompt | a strict system directive ("choose exactly one listed option … only its uppercase letter, no explanation"), then a JSON user message with `evidence`, `criterion`, `options` labelled `A`…`P` |
| slots | the token id of each option letter, verified to be exactly one token, and that appending the letter to the prompt yields `ids + [slot]` |
| readout | one forward pass, `logits[:, -1, :]`, restricted to the slot ids, softmax; the result is probabilities |
| provenance | `prompt_sha256`, `prompt_version`, `input_tokens`, `forward_seconds`, model metadata, and the caveat string *"conditional option score; uncalibrated as decision confidence"* |

Frozen Qwen3.5-4B, one RTX 3090: 1.023 s for 21 direct decisions against 5.332 s
for the same 21 as a generated JSON array (5.21×), and up to 20 decisions/s with
a long state prefilled once and branched across criteria, against 2.33/s scoring
each fresh. The reuse paths are marked experimental: BF16 execution changed 5–6
of 777 argmaxes.

## Verified on this box, over this server

llama.cpp's chat route returns the same distribution, so the readout needs no
PyTorch. Thinking **must** be off: with thinking on, the single allowed token is
spent inside `reasoning_content` (observed).

```sh
curl -s http://127.0.0.1:40583/v1/chat/completions -H 'Content-Type: application/json' -d '{
  "model":"qwen3.8-27b-gsq-rco-iq3xxs",
  "messages":[{"role":"system","content":"…choose exactly one listed option…"},
              {"role":"user","content":"{\"evidence\":\"the build is green\",\"criterion\":\"does this report success?\",\"options\":[{\"letter\":\"A\",\"description\":\"yes\"},{\"letter\":\"B\",\"description\":\"no\"}]}"}],
  "max_tokens":1,"temperature":0,"logprobs":true,"top_logprobs":8,
  "chat_template_kwargs":{"enable_thinking":false}}'
```

Answer: `content: "A"`, with

```
'A'  logprob -0.004      'B'  -5.813      ''   -8.983      'The' -9.055
'Yes' -9.494             '<tool_call>' -10.242   'I' -10.910   'Y' -11.062
```

0.28 s warm. Renormalising {A, B} gives A = 0.997: the OpenJev readout, one
forward pass, zero generated tokens, typed probabilities, from the server clank
already talks to. The native `/completion` route also carries the distribution
(`completion_probabilities` with `n_probs`), the fallback if `logprobs` is
unavailable on the chat route.

A sanity run on Qwen3.8-27B (12 statements, two-option criterion, thinking off)
gave **10/12 argmax correct, median 0.66 s per decision**, the first item paying
a 54 s model load, with a smallest A/B margin of 0.171. It is a sanity check,
not an evaluation: asking "is this statement true?" of a one-line state is
degenerate (the state *is* the claim), which explains both misses. A real
evaluation needs scenario-style states and criteria, step 1 of the branch plan.

## Integration shape

OpenJev is not a vendoring target: a torch + transformers program that loads
model weights onto a CUDA device. clank would add the unix interface to its
*method*, inside the existing contract:

```sh
clank --decide < decisions.jsonl        # one request per row, one JSON line out
{"id":"route-1","top":"access","probabilities":{"access":0.91,"billing":0.09}}
```

| element | decision |
|---|---|
| input and output | JSONL both ways, one request per row |
| row schema | OpenJev's: state may be structured, options carry ids and descriptions, 2–16 options |
| request shape | thinking forced off, `max_tokens: 1`, `logprobs` with `top_logprobs` ≥ the option count |
| wire checks | a slot letter absent from the returned distribution is a per-row error, not a silent zero; a server returning no `logprobs` at all fails the invocation instead of degrading into generation |
| `--each` machinery | reused: framed rows, per-row errors that do not stop the run, exit 1 if any row failed, the existing closed event set (`item` / `decision` / `error`) |
| provenance and dependencies | every row carries model, `prompt` id (the effective system prompt), input tokens and the prompt hash, as runs already do; no new dependency, no weights, no GPU management, the server owns inference |

## Open questions

| question | state |
|---|---|
| quality on these models | OpenJev's numbers (0.813 balanced accuracy on authored decisions, 0.637 on WANLI, 0.845 modal agreement on the TypeSafe subset) are Qwen3.5-4B; nothing transfers to a 27B or a 35B-A3B without measuring, and the subset comparison is against records, not a live endpoint |
| calibration | their own output carries "uncalibrated as decision confidence"; probabilities are *relative* option scores |
| reranker path | Qwen3-Reranker is a different model class, out of scope |
| letter-slot tokenisation | A–P are single tokens on Qwen tokenisers; a model where they are not must fail loudly, which is why the check is ported rather than assumed |

## The `--use-jev` flag (measured 2026-09-19)

**No, not as a flag inside clank.** The composition already covers it; a flag
would buy latency for a second wire protocol, a cloud account and a credential
inside clank. Same fixture, 18 typed decisions (6 `choice`, 6 `noul`, 6 `score`,
chance = 39%), same rows, shuffled-context control included
(`~/Work/jev-vs-cactus/`):

| arm | accuracy | shuffled control | p50 | cost |
|---|---|---|---|---|
| clank, closed answer space (`--json-schema` enum, local 9B) | **18/18 = 100%** | 11% | 1845 ms | $0 |
| clank, prose (local 9B) | 17/18 = 94% | 11% | 5980 ms | $0 |
| Jev `typesafe/jev-1.13` via OpenRouter Decisions | 17/18 = 94% | 11% | **591 ms** | $0.000015 |
| **`clank-jev --provider kev`** (local trained 0.6B, no key) | **16/18 = 89%** | 17% | **83 ms** | $0 |
| jevmlx (local MLX, 3B, Jev-style readout) | 12/18 = 67% | 28% | 583 ms | $0 |
| Cactus/Needle 3 (35 MB tool model, forced) | 6/18 = 33% | **44%** | 116 ms | $0 |

On this box: the **technique** (closed answer space) is worth 6 points and 3×
speed over prose, and clank already has it in `--json-schema`, with no Jev, no
network and no account. Jev adds **0.59 s instead of 1.85 s** and **a
probability to gate on**, which clank answers do not carry. Cactus is not a
decision engine: it scores below chance and its accuracy goes *up* when the
context is scrambled.

The sidecar earns its place too: a **0.6B trained model** within five points of
hosted Jev on this fixture, passing the control, at 83 ms with no network and no
key. On kev's harder out-of-domain suites the gap is wider (0.598–0.631 against
Jev's 0.857), so read 89-vs-94 as this fixture, not a general equivalence.
`kev-4b`, the checkpoint its authors recommend, serves bf16 only, ~8.5 GB, and
does not fit on a 16 GB Mac beside a resident oMLX model.

### The three shapes

| shape | what it is | verdict |
|---|---|---|
| `--use-jev` (client for the Decisions endpoint) | clank POSTs typed questions to `api.alpha.decisions` and prints the typed answer | **no.** A second wire protocol and a second source of truth inside the binary; `PROTOCOL.md` rules the second one out, and the shell can already do it as a stage |
| a `jev` sidecar — **built, as `clank-jev`** | `state \| clank-jev --ask … --choice …` → the value, or JSON with probabilities; clank stays the prose stage | **yes, and it is what we did.** A second binary in this crate, no change to clank's contract, and the endpoint is a flag: `typesafe`, `openrouter`, or a local `kev` with no credentials |
| `--decide` with a logit readout | clank sends `logprobs` with the slot tokens and reads the distribution itself, Jev's *method* with no Jev (§ branch plan below) | **blocked on this Mac's oMLX**: it returns no logprobs at all (`choices[0].logprobs` absent, verified on two models). llama.cpp does return them (`:8012` on this box: a one-token request came back with `top_logprobs` including `A` -1.63 vs `Yes` -0.95), so the mode is testable against llama.cpp without giving clank a second protocol |

### Conditions that would change the answer

| condition | consequence |
|---|---|
| decisions taken at a rate where 1.2 s each matters (thousands per run) | the sidecar earns its place on latency alone |
| a threshold set on a *calibrated* probability (Jev's own claim, not ours to verify) | only Jev or a local readout can supply it; a prompted JSON number is not a probability |
| oMLX grows logprobs | `--decide` becomes the best answer on every axis except latency, and this section should be rewritten |

## The bar to beat: Needle 3

Read while researching this:
[cactus-compute/needle](https://github.com/cactus-compute/needle) (Apache-2.0
for code *and* weights, 11k stars, Hugging Face `Cactus-Compute/needle3`). It is
a **model**, not a harness: the same job as Jev's interface, typed tool calls
and structured extraction, with a trained specialist rather than a prompt or a
readout.

| property | value |
|---|---|
| parameters, archive | 29M–121M parameters; the shipped 20-layer archive is **35 MB**, every depth from 2 to 20 layers a trained subnetwork, sliced with `needle build --layers N` down to about **9 MB**. A Reddit post advertises "829 MB"; the project's own description and device guide say 8–29 MB, so treat that number as wrong |
| decoding | grammar compiled from *your* tool schemas constrains every token, so a call always parses; a deterministic repair step grounds arguments in the request |
| engine, throughput | under 1 MB per platform, 13 folders including a C API, wasm and a WASI component; ~400 tok/s on a Raspberry Pi 5 at 20 layers, ~4000 at 2 |
| runner | `./needle --model needle3.cact --tools tools.json --serve` answers `POST /complete {"input": "..."}` (documented; not verified here) |

Its behaviour contract is this project's doctrine, enforced inside the model:

| Needle 3 | clank |
|---|---|
| off-topic input returns an empty `function_calls`, explicitly "no free-text fallback" | an empty answer is a failure (exit 1), never a success |
| `confidence` is calibrated, and the engine withholds calls below 0.1 into `suppressed_calls` | a truncated or unverifiable answer exits non-zero instead of being passed on |
| `validation.ungrounded` names arguments not evidenced in the input | the context is exactly what was piped; its absence is stated, not guessed |
| grammar constrained by the declared schemas | `--json-schema`, enforced by the server |

Two consequences for the branch plan:

| consequence | detail |
|---|---|
| step 3 needs a third arm | readout versus a 9–35 MB specialist, not readout versus generated JSON, measured on the same fixture: accuracy, latency, and what each one refuses. OpenJev's readout has *no* refusal gate, and its own output says "uncalibrated as decision confidence" |
| it is not an integration target | clank speaks OpenAI-compatible `/v1/chat/completions`, Needle its own `/complete`; a second wire protocol is the second source of truth `PROTOCOL.md` rules out, and vendoring the engine would put an inference stack inside a client. If a decision stage on this box should be Needle, the shape is the shell's: its runner serves HTTP, something translates, clank stays a stage, or clank is not in that path |

`needle finetune --generate` synthesises training data through **OpenRouter**
(default `deepseek/deepseek-v4-flash`), a cloud dependency in the training path
only: inference never touches the network. Anonymous telemetry is on by default
and opts out with `NEEDLE_TELEMETRY=0`.

## Branch plan

Branch `decide`, in this order:

| step | what |
|---|---|
| 1 | `docs/decision-readout.md` (this note) plus an eval fixture of **scenario-style rows** with known answers, so step 3 has something to measure |
| 2 | `--decide`: JSONL in, JSONL out, one request per row, thinking off, slot verification, per-row errors, exit codes as above. Wire tests script the `logprobs` payload, so the mode is tested without a model |
| 3 | measure on the fixture: accuracy, per-row latency, and a *generated* JSON array answering the same decisions on this box (the comparison OpenJev's README makes), so the speed claim is ours rather than borrowed |
| 4 | only if 3 is convincing: a README section and a cheatsheet entry |

Explicitly out of scope: vendoring their code, torch/transformers, the WebGPU
browser demo, the reranker, and any claim about Jev itself.
