# `clank-jev` — typed decisions for routing

The second binary in this crate. One job: ask typed questions about a state and
get answers a shell can branch on. `clank` writes prose; `clank-jev` picks one
of *your* options and says how sure it is.

```sh
route=$(printf '%s' "$task" | clank-jev --ask 'What kind of task is this?' \
          --choice code,prose,math --min-prob 0.7) || route=unclear
case "$route" in
  code) clank --model local-code -c src/context.rs -m "$task" ;;
  *)    clank --model local-fast -m "$task" ;;
esac
```

It is a binary rather than a `clank` flag because clank's contract is one
prompt, one request, one answer, no second wire protocol (invariants 1–3), and
because a decision stage stands on its own in a git hook, a Makefile or a cron
job, without carrying a chat client along. [PROTOCOL.md](../PROTOCOL.md) has the
admission-rules argument.

## Providers

Credentials come from the environment, never from argv:

| `--provider` | endpoint | credential | default model |
|---|---|---|---|
| `auto` *(default)* | whichever credential is set | — | — |
| `typesafe` | `api.typesafe.ai/v1/systemone` | `TYPESAFE_API_KEY` (or `JEV_API_KEY`, `JEV_CLI_API_KEY`) | `jev-latest` |
| `openrouter` | `openrouter.ai/api/alpha/decisions` | `OPENROUTER_API_KEY` | `typesafe/jev-1.13` |
| `kev` | `127.0.0.1:8009/v1/systemone` (`--base-url` to move it) | none | `kev-latest` |

`./.clank/config.toml` is optional, and it is read from the working directory
only. `--config PATH` or `CLANK_CONFIG` names a different file.
`[clank].base_url` and `[clank].model` apply when `--base-url` and `--model`
were not passed, and they replace the provider built-ins above. `--timeout` wins
over `[clank].timeout`, which wins over 60 seconds. Provider credentials stay
the variables in the table; `[clank].api_key_env` names the fallback variable
when those are unset. `[web]` is search configuration for `clank-web`. A missing
file in the working directory leaves the flags and the provider environment in
charge. `CLANK_MODEL` and `CLANK_BASE_URL` belong to `clank`.

`kev` is a local System One server; [kev](https://github.com/jaredpalmer/kev) is
a trained Jev-family model (LoRA + pointer readout head on Qwen, one prefill
pass) that speaks the same request and response shapes, so it needs no
credentials and no network:

```sh
git clone https://github.com/jaredpalmer/kev && cd kev && uv sync --extra serve
KEV_DTYPE=fp32 uv run --extra serve python -m kev.serve --run jaredpalmer/kev-0.6b --port 8009
```

On the same 18 typed decisions as
[history/decision-readout.md](history/decision-readout.md), `kev-0.6b` scored
**16/18 = 89%** at an 83 ms median with a 17% shuffled-context control (hosted
Jev: 94%, 591 ms, $0.000015). `kev-4b`, the checkpoint they recommend, does not
fit here: bf16 only, ~8.5 GB against 16 GB shared with a resident oMLX model.

## Questions

Every question shape is available on the command line, so a script needs no
file: `--ask` with `--choice A,B,C` (unordered options), `--boolean` (yes/no),
or `--score low,mid,high` (ordered levels). `--checks FILE` takes the full set,
in Jev's own JSON shape: `{id: {type, instructions, criteria, reasons?}}`.

**A closed-choice reason rides along with the value**, decided in the same
request. `fixtures/checks-verification.json` asks the two questions a shadow
watchdog asks, each with its own reason set:

```sh
printf '%s\n' "USER: run the tests, do not touch the config" \
  "TOOL cargo test -> FAILED" "ASSISTANT: all tests pass, config updated" \
  | clank-jev --checks fixtures/checks-verification.json --json --min-prob 0.6
```

```json
{"answers":{"verification":{"type":"noul","value":true,"probability":0.98,
  "reason":"verification_contradiction","reason_probability":1.0}}, ...}
```

## Gates

`--min-prob` fails a decision you asked not to trust; `--expect` and
`--expect-min` fail one that is not the value you needed. A question the
provider skipped, or an answer carrying no probability, fails the gate instead
of passing by default. `--print-reason` puts the closed-choice reason on stdout
instead of the value, for a script that routes on *why* rather than *what*.

`--min-prob` compares against **confidence in the decision**, not the
probability of "yes". For a yes/no question those differ: a decisive *no* has
`probability` near 0 and `confidence` near 1, so a gate on the raw probability
would reject the model for being certain. The JSON carries both — `probability`
is P(true) for `noul` and the winning option's share otherwise, `confidence` is
`max(p, 1-p)` for `noul` and the provider's own normalized margin for `choice`/`score`
when it sends one.

| code | meaning |
|---|---|
| `0` | decided, and every gate passed |
| `1` | a gate failed (**the decision is still printed**, because the caller asked not to trust it, not to lose it), or the result object could not be produced |
| `2` | usage: no question shape, an empty state, a malformed checks file |
| `3` | provider, network or credentials |

stdout is the bare value for one question (`code`, `true`, `2`), a JSON object
for several; diagnostics and every gate failure go to stderr; `-q` silences
them.

## Flags

Credentials come from the environment only. Which variable selects which
provider is in the table under [Providers](#providers).

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

## Combinations

`clank` and `clank-jev` are both stages, so they compose in both orders. Each of
these was run; the observed behaviour is quoted.

**Generate, then validate** — clank writes, jev checks it against a rubric, and
a failed check stops the pipeline. `fixtures/checks-commit.json` asks whether
the message describes the diff and what shape its subject line has:

```sh
msg=$(git show HEAD | clank -q --thinking off -m 'Write the commit message for this diff.')
{ git show --stat HEAD; printf 'MESSAGE:\n%s\n' "$msg"; } \
  | clank-jev --checks fixtures/checks-commit.json --min-prob 0.6
```

Observed: `describes` = true, reason `no_conflict`, p=0.89 — and the gate still
fired at `shape` (p=0.500), because `README.md: update …` is a path prefix
rather than a typed one.

**Audit a trace after the fact** — a `--jsonl` trace is evidence, so the
watchdog questions can be asked of it afterwards:

```sh
clank --jsonl -m 'where is the transcript cap defined? cite file:line' > trace.jsonl
clank-jev --checks fixtures/checks-verification.json --min-prob 0.6 < trace.jsonl
```

Observed: `verification` = false, reason `no_conflict` (the citation was real),
and `instruction` at p=0.23 — no user instruction exists in a trace, so the
check reports it cannot decide and the gate fails the run.

**Fan out, act only on confident decisions** — the loop is the shell's:

```sh
while read -r subject; do
  v=$(printf '%s' "$subject" | clank-jev -q --ask 'Could this break an existing caller?' \
        --boolean --min-prob 0.7) || { echo "unclear: $subject"; continue; }
  [ "$v" = true ] && echo "check: $subject"
done < <(git log --format=%s -8)
```

Observed: 7 of 8 doc-only subjects decided `false` at confidence ≥ 0.83, and one
came back `unclear` at 0.68 — the run that found the `--min-prob` semantics
above.

**Escalate: local first, hosted only when the local answer is not confident:**

```sh
ask() { printf '%s' "$1" | clank-jev -q --provider "$2" --ask 'Which team owns this?' \
          --choice BILLING,TECHNICAL,ACCOUNT --min-prob "$3"; }
v=$(ask "$state" kev 0.9) || v=$(ask "$state" openrouter 0.5)
```

Observed: the local `kev-0.6b` decided `TECHNICAL` at confidence 0.84, below the
0.9 gate, so the hosted Jev was asked and agreed — 83 ms and $0 spent before
reaching the network.

**Break a tie between two answers** — two models, one judge:

```sh
a=$(clank -q --model local-fast -c src/context.rs -m "$task One line.")
b=$(clank -q --model local-code -c src/context.rs -m "$task One line.")
printf 'A: %s\n\nB: %s\n' "$a" "$b" \
  | clank-jev --ask 'Which answer names the exact file:line and the correct value?' --choice A,B --min-prob 0.6
```

Observed: it picked B at p=0.75, confidence 0.51, and the gate fired, because
neither answer had the right line number.
