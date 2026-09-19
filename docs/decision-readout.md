# Decision readout: option probabilities instead of generated answers

Research note, 2026-09-17, prompted by [TheoLeeCJ/openjev](https://github.com/TheoLeeCJ/openjev)
(MIT, created 2026-09-16, 631 stars at the time of reading).

**Verdict: the pattern is worth having in clank; the implementation is not
portable to it.** OpenJev needs full-vocabulary logits from a local PyTorch
model. This server already exposes the same readout over its OpenAI-compatible
route — verified below — so clank can offer it in ~50 lines, with no new
dependency, no model download, and no second inference stack.

## What the two projects are

**Jev** (TypeSafe, announced 2026-09-14) is a closed service for *typed*
decisions: you send state, a criterion and options, and it returns option
probabilities rather than prose. **OpenJev** is an independent reproduction of
that *interface pattern* with open models — its README is explicit that it does
not reproduce Jev's model or training. That distinction is the point: what is
reproducible is the interface and the readout method.

## The method, as implemented

From `src/openjev_phase1/core.py` and `direct.py` (read at `master`):

| step | what it does |
|---|---|
| input row | `{id, state, question, options:[{id, description}]}`; state may be a string, object or array; 2–16 options |
| prompt | a strict system directive ("choose exactly one listed option … only its uppercase letter, no explanation"), then a JSON user message with `evidence`, `criterion`, `options` labelled `A`… `P` |
| slots | the token id of each option letter, *verified* to be exactly one token, and verified that appending the letter to the prompt yields `ids + [slot]` |
| readout | one forward pass, `logits[:, -1, :]`, restricted to the slot ids, softmax → probabilities |
| provenance | `prompt_sha256`, `prompt_version`, `input_tokens`, `forward_seconds`, model metadata, and the caveat string: *"conditional option score; uncalibrated as decision confidence"* |

Their published measurements, on a frozen Qwen3.5-4B and one RTX 3090: **1.023 s
for 21 direct decisions versus 5.332 s** for the same 21 as a generated JSON
array (5.21×), and up to **20 decisions/s** when a long state is prefilled once
and branched across criteria (versus 2.33/s scoring each fresh). Their reuse
paths are marked experimental — BF16 execution changed 5–6 of 777 argmaxes.

## Verified on this box, over this server

The readout does not need PyTorch: llama.cpp's chat route returns the same
distribution when asked. Request (thinking **must** be off — with thinking on the
single allowed token is spent inside `reasoning_content`, observed):

```sh
curl -s http://127.0.0.1:40583/v1/chat/completions -H 'Content-Type: application/json' -d '{
  "model":"qwen3.8-27b-gsq-rco-iq3xxs",
  "messages":[{"role":"system","content":"…choose exactly one listed option…"},
              {"role":"user","content":"{\"evidence\":\"the build is green\",\"criterion\":\"does this report success?\",\"options\":[{\"letter\":\"A\",\"description\":\"yes\"},{\"letter\":\"B\",\"description\":\"no\"}]}"}],
  "max_tokens":1,"temperature":0,"logprobs":true,"top_logprobs":8,
  "chat_template_kwargs":{"enable_thinking":false}}'
```

Answer, in full: `content: "A"`, and

```
'A'  logprob -0.004      'B'  -5.813      ''   -8.983      'The' -9.055
'Yes' -9.494             '<tool_call>' -10.242   'I' -10.910   'Y' -11.062
```

0.28 s warm. Renormalising {A, B} gives A = 0.997. That is the OpenJev readout —
one forward pass, zero generated tokens, typed probabilities — obtained from the
server clank already talks to. The native `/completion` route also carries the
distribution (`completion_probabilities` with `n_probs`), so a fallback exists if
`logprobs` is ever unavailable on the chat route.

**A sanity run on Qwen3.8-27B** (12 statements, two-option criterion, thinking
off): **10/12 argmax correct, median 0.66 s per decision** (the first item paid a
54 s model load), smallest A/B margin 0.171. This is a sanity check, not an
evaluation: asking "is this statement true?" of a one-line state is degenerate —
the state *is* the claim — and both misses are explained by that, not by the
readout. A real evaluation needs scenario-style states and criteria, which is
step 1 of the branch plan.

## What integration would mean

Not vendoring OpenJev: it is a torch + transformers program that loads model
weights onto a CUDA device, and its value here is the *method*. What clank would
add is the unix interface to that method, inside the existing contract:

```sh
clank --decide < decisions.jsonl        # one request per row, one JSON line out
{"id":"route-1","top":"access","probabilities":{"access":0.91,"billing":0.09}}
```

- Input and output are JSONL, the row schema is OpenJev's (it is good: state may
  be structured, options carry ids and descriptions, 2–16 options).
- One request per row, thinking forced off, `max_tokens: 1`, `logprobs` with
  `top_logprobs` ≥ the option count.
- Reuse `--each`'s machinery: framed rows, per-row errors that do not stop the
  run, exit 1 if any row failed, and the existing closed event set
  (`item` / `decision` / `error`).
- Port their two checks as *wire* checks: if a slot letter is absent from the
  returned distribution, that row is an error rather than a silent zero; if the
  server returns no `logprobs` at all, the invocation fails loudly instead of
  degrading into generation.
- Provenance in every row, as clank already does for runs: model, `prompt` id
  (the effective system prompt), input tokens, and the prompt hash.
- No new dependency, no weights, no GPU management: the server owns inference,
  as it does for every other mode.

## What is *not* established

- **Quality on these models.** OpenJev's numbers (0.813 balanced accuracy on
  authored decisions, 0.637 on WANLI, 0.845 modal agreement on the TypeSafe
  subset) are Qwen3.5-4B. Nothing transfers to a 27B or a 35B-A3B without
  measuring, and the subset comparison is against records, not a live endpoint.
- **Calibration.** Their own output carries "uncalibrated as decision
  confidence"; probabilities are *relative* option scores.
- **The reranker path** (Qwen3-Reranker) — a different model class, out of scope.
- **Letter-slot tokenisation** across models: A–P are single tokens on Qwen
  tokenisers; a model where they are not must fail loudly, which is why the
  check is ported rather than assumed.


## Should clank grow `--use-jev`? (measured 2026-09-19)

Short answer: **no — not as a flag inside clank.** The composition already covers it,
and putting it in the binary buys latency in exchange for a second wire protocol, a
cloud account and a credential inside a tool whose value is being one honest stage.

What the numbers say. Same fixture, 18 typed decisions (6 `choice`, 6 `noul`,
6 `score`, chance = 39%), same rows, shuffled-context control included
(`~/Work/jev-vs-cactus/`):

| arm | accuracy | shuffled control | p50 | cost |
|---|---|---|---|---|
| clank, closed answer space (`--json-schema` enum, local 9B) | **18/18 = 100%** | 11% | 1845 ms | $0 |
| clank, prose (local 9B) | 17/18 = 94% | 11% | 5980 ms | $0 |
| Jev `typesafe/jev-1.13` via OpenRouter Decisions | 17/18 = 94% | 11% | **591 ms** | $0.000015 |
| jevmlx (local MLX, 3B, Jev-style readout) | 12/18 = 67% | 28% | 583 ms | $0 |
| Cactus/Needle 3 (35 MB tool model, forced) | 6/18 = 33% | **44%** | 116 ms | $0 |

So on this box and this task: the **technique** (closed answer space) is worth 6
points and 3× speed versus prose, and clank already has it in `--json-schema` —
no Jev required, no network, no account. What Jev adds is **0.59 s instead of
1.85 s** and **a probability to gate on**, which clank answers do not carry at all.
Cactus is not in the running as a decision engine: it scores below chance and its
accuracy goes *up* when the context is scrambled.

### The three shapes, if we ever do want it

| shape | what it is | verdict |
|---|---|---|
| `--use-jev` (client for the Decisions endpoint) | clank POSTs typed questions to `api.alpha.decisions` and prints the typed answer | **no.** A second wire protocol and a second source of truth inside the binary; `PROTOCOL.md` rules the second one out, and the shell can already do it as a stage |
| a `jev` sidecar — **built, as `clank-jev`** | `state | clank-jev --ask … --choice …` → the value, or JSON with probabilities; clank stays the prose stage | **yes, and it is what we did.** A second binary in this crate, no change to clank's contract, and the endpoint is a flag: `typesafe`, `openrouter`, or a local `kev` with no credentials |
| `--decide` with a logit readout | clank sends `logprobs` with the slot tokens and reads the distribution itself — the § branch plan below, Jev's *method*, no Jev | the honest native version, and **blocked on this Mac's oMLX**: it returns no logprobs at all (`choices[0].logprobs` absent, verified on two models). llama.cpp does return them (`:8012` on this box: a one-token request came back with `top_logprobs` including `A` -1.63 vs `Yes` -0.95), so the mode is testable against llama.cpp without giving clank a second protocol |

### What we measured after building it (2026-09-19)

| arm | accuracy | shuffled control | p50 | cost |
|---|---|---|---|---|
| clank, closed answer space (local 9B, `--json-schema` enum) | 18/18 = 100% | 11% | 1845 ms | $0 |
| **`clank-jev --provider kev`** (local trained 0.6B) | **16/18 = 89%** | 17% | **83 ms** | $0 |
| `clank-jev --provider openrouter` (hosted Jev 1.13) | 17/18 = 94% | 11% | 591 ms | $0.000015 |

The sidecar earns its place, and the local backend is the surprise: a **0.6B
trained model** lands within five points of hosted Jev on this fixture, passes the
control, and answers in 83 ms with no network and no key. On kev's own harder
out-of-domain suites the gap is much wider (0.598–0.631 against Jev's 0.857), so
treat 89-vs-94 as this fixture, not as a general equivalence. `kev-4b` (the
checkpoint its authors recommend) serves bf16 only, ~8.5 GB, and does not fit on a
16 GB Mac beside a resident oMLX model.

### What would change the answer

- If decisions were being taken at a rate where 1.2 s each matters (thousands per
  run), the sidecar earns its place on latency alone.
- If a threshold had to be set on a *calibrated* probability — Jev's own claim, not
  ours to verify — then only Jev or a local readout can supply it; a prompted JSON
  number is not a probability.
- If oMLX grows logprobs, `--decide` becomes the best answer on every axis except
  latency, and this section should be rewritten.

## The bar to beat: Needle 3

Read while researching this: [cactus-compute/needle](https://github.com/cactus-compute/needle)
(Apache-2.0 for code *and* weights, 11k stars, Hugging Face `Cactus-Compute/needle3`).
It is a **model**, not a harness, and it attacks the same job as Jev's interface —
typed tool calls and structured extraction — with a trained specialist instead of a
prompt or a readout:

- 29M–121M parameters. The shipped 20-layer archive is **35 MB**; every depth from
  2 to 20 layers is a trained subnetwork, sliced with `needle build --layers N`
  down to about **9 MB**. (A Reddit post advertises "829 MB"; the project's own
  description and device guide say 8–29 MB, so treat that number as wrong.)
- Grammar compiled from *your* tool schemas constrains every token, so a call
  always parses; a deterministic repair step grounds arguments in the request.
- Engine under 1 MB per platform — 13 folders including a C API, wasm, and a WASI
  component; ~400 tok/s on a Raspberry Pi 5 at 20 layers, ~4000 at 2.
- Its runner serves HTTP: `./needle --model needle3.cact --tools tools.json --serve`
  answers `POST /complete {"input": "..."}` (documented; not verified here).

**Its behaviour contract is this project's doctrine, enforced inside the model**
— the strongest outside confirmation of the rules clank applies at the process
level:

| Needle 3 | clank |
|---|---|
| off-topic input returns an empty `function_calls`, explicitly "no free-text fallback" | an empty answer is a failure (exit 1), never a success |
| `confidence` is calibrated, and the engine withholds calls below 0.1 into `suppressed_calls` | a truncated or unverifiable answer exits non-zero instead of being passed on |
| `validation.ungrounded` names arguments not evidenced in the input | the context is exactly what was piped; its absence is stated, not guessed |
| grammar constrained by the declared schemas | `--json-schema`, enforced by the server |

Two consequences for the branch plan:

1. **Step 3 needs a third arm.** The comparison that decides anything is not
   "readout versus generated JSON" but "readout versus a 9–35 MB specialist",
   measured on the same fixture: accuracy, latency, and what each one refuses.
   OpenJev's readout has *no* refusal gate — its own output says "uncalibrated as
   decision confidence" — and that gap is exactly what Needle closes.
2. **It is not an integration target.** clank speaks OpenAI-compatible
   `/v1/chat/completions`; Needle speaks its own `/complete`. Teaching clank a
   second wire protocol to reach it would be the second source of truth PROTOCOL
   rules out, and vendoring the engine would put an inference stack inside a
   client. If a decision stage on this box should be Needle, the honest shape is
   the shell's: its runner serves HTTP, something translates, clank stays a stage —
   or clank is simply not in that path.

Fine-tuning note for a local-first setup: `needle finetune --generate` synthesises
training data through **OpenRouter** (default `deepseek/deepseek-v4-flash`) — a
cloud dependency in the *training* path only; inference never touches the network.
Anonymous telemetry is on by default and opts out with `NEEDLE_TELEMETRY=0`.

## Branch plan

Branch `decide`, in this order:

1. `docs/decision-readout.md` (this note) plus an eval fixture of **scenario-style
   rows** with known answers, so step 3 has something honest to measure.
2. `--decide`: JSONL in, JSONL out, one request per row, thinking off, slot
   verification, per-row errors, exit codes as above. Wire tests script the
   `logprobs` payload, so the mode is tested without a model.
3. Measure on the fixture: accuracy, per-row latency, and the comparison that
   OpenJev's README makes — the same decisions answered by a *generated* JSON
   array — on this box, so the speed claim is ours rather than borrowed.
4. Only if 3 is convincing: a README section and a cheatsheet entry.

Explicitly out of scope: vendoring their code, torch/transformers, the WebGPU
browser demo, the reranker, and any claim about Jev itself.
