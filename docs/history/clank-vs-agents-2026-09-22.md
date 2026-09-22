# 4-way QA bench: clank vs omp vs pi vs opencode

**When:** 2026-09-22 ~12:28–12:38 EEST  
**Box:** Linux, ~15 GiB RAM (idle OpenCode TUIs left alone; ~2.5 GiB RSS already resident)  
**Prompt:** `reply with exactly: pong` → all four answered `pong`  
**Evidence:** `QA workspace `clank-qa-bench/` (local)`  
**Measurement:** `run_measure.py` (wall + peak RSS via `/proc`, stdin=`DEVNULL`)

## Model IDs used (same Go wire)

| Tool | Model id | Same provider? | Session / wire notes |
|------|----------|----------------|----------------------|
| **clank** | `glm-5.3-flash` via `--base-url http://127.0.0.1:18081/v1` | **Yes** (Go) | Needs local `go_proxy.py` to inject `x-opencode-session` (+ UA). No `--thinking` (Go 400 on `chat_template_kwargs`). |
| **omp** | `opencode-go/glm-5.3-flash` | **Yes** | Native Go session headers. Flags: `omp launch -p --mode text --no-tools --no-session --thinking off`. |
| **pi** | `opencode-go/glm-5.3-flash` | **Yes** | Catalog lacked `glm-5.3-flash`; Go rejects bare requests (`MissingSessionID`). Bench used **temporary** `~/.pi/agent/models.json` (headers + model register); copy: `pi-models.json.bench`. Removed after runs. Flags: `pi -p --mode text --no-tools --no-session --thinking off`. |
| **opencode** | `opencode-go/glm-5.3-flash` | **Yes** | Native. `opencode run --pure --format json` (no `--auto`). Prior 3-run set reused; +1 confirmatory. |

No fake same-provider: all four hit **OpenCode Go / glm-5.3-flash**.

## Metric table

| Metric | clank | omp | pi | opencode |
|--------|------:|----:|---:|---------:|
| **Version / binary** | release `clank` 6.4 MB (`7a3662d` build) | omp **18.2.6** (bun global) | pi **0.73.1** (npm) | opencode-ai **1.18.31** |
| **Ship / entry artifact** | **6.4 MB** ELF | **22 MB** bundled `cli.js` | **870 B** node shim | **177 MB** `opencode.exe` |
| **Install tree (follow symlink)** | n/a (Rust binary) | **53 MB** `@oh-my-pi/pi-coding-agent`; **449 MB** `@oh-my-pi` (incl. **347 MB** `pi-natives-linux-x64`) | **205 MB** `@mariozechner/pi-coding-agent` (+ nested deps) | **726 MB** `opencode-ai` |
| **One-shot wall median (s)** | **0.753** | **3.435** | **2.203** | **5.728** |
| **One-shot peak RSS median** | **~4.9 MB** | **~371 MB** | **~177 MB** | **~562 MB** |
| **Runs** | 3 (prior) | 3 | 3 | 3 prior + 1 confirm (4.75 s / 590 MB) |
| **Tools / agent loops** | `--no-tools` | `--no-tools --no-session` | `--no-tools --no-session` | `--pure` (no `--auto`) |
| **Harness type** | thin pipe / OpenAI client | coding-agent CLI | coding-agent CLI | coding-agent CLI + TUI product |

### Wall time (s)

| Run | clank | omp | pi | opencode |
|-----|------:|----:|---:|---------:|
| 1 | 0.579 | 2.309 | 17.244† | 7.440 |
| 2 | 0.753 | 5.957 | 1.899 | 4.205 |
| 3 | 2.155 | 3.435 | 2.203 | 5.728 |
| **median** | **0.753** | **3.435** | **2.203** | **5.728** |
| confirm | — | — | — | 4.746 |

† pi run 1 cold / network outlier; median still fair.

### Peak RSS (MB)

| Run | clank | omp | pi | opencode |
|-----|------:|----:|---:|---------:|
| 1 | 4.9 | 360.4 | 160.6 | 560.5 |
| 2 | 4.8 | 370.7 | 177.0 | 562.1 |
| 3 | 4.9 | 383.1 | 181.1 | 567.0 |
| **median** | **4.9** | **370.7** | **177.0** | **562.1** |
| confirm | — | — | — | 590.1 |

## Caveats (read before quoting)

1. **Agent harness vs pipe:** clank is a minimal OpenAI-compatible client. omp / pi / opencode are coding-agent CLIs (system prompts, session machinery, node/bun runtime) even with tools disabled — expect tens–hundreds of MB RSS and multi-second cold paths.
2. **Same model, different clients:** wall time includes client startup + network; model latency is shared-ish but not isolated.
3. **Go session header:** clank cannot set custom headers → local proxy. pi needed `models.json` header override for the same reason. omp and opencode speak Go natively.
4. **`--thinking off`:** used on omp/pi; **omitted** on clank→Go (upstream unknown-field 400). opencode `--pure` path as prior.
5. **Idle OpenCode TUIs** (~604–759 MB ×4) were **not** killed; they inflate box pressure but were excluded from one-shot RSS (measured process trees only).
6. **opencode token tax (prior):** ~7.5k input tokens for “pong” in JSON traces; clank/curl path is tens of tokens. Not re-measured for omp/pi JSON this pass.
7. **pi catalog gap:** without the temporary `models.json`, `pi --list-models` had no `glm-5.3-flash` under `opencode-go`, and bare Go calls 400. Fallback that *would* work without headers: `openrouter/z-ai/glm-4.7-flash` (smoke only; **not** used in the table).

## Commands (repro)

```bash
set -a; source $OPENCODE_API_KEY env; set +a
# clank (proxy must be up):
python3 run_measure.py clank-runN ./target/release/clank \
  --no-tools --max-tokens 32 --timeout 120 -m 'reply with exactly: pong'
# omp:
python3 run_measure.py omp-runN omp launch -p --mode text --no-tools --no-session \
  --thinking off --model opencode-go/glm-5.3-flash 'reply with exactly: pong'
# pi (needs models.json headers as in pi-models.json.bench):
python3 run_measure.py pi-runN pi -p --mode text --no-tools --no-session \
  --thinking off --model opencode-go/glm-5.3-flash 'reply with exactly: pong'
# opencode:
python3 run_measure.py opencode-runN opencode run --pure --format json \
  -m opencode-go/glm-5.3-flash 'reply with exactly: pong'
```

## Blunt ranking (this tiny prompt)

- **RSS:** clank (~5 MB) ≪ pi (~177 MB) < omp (~371 MB) < opencode (~562 MB).
- **Wall (median):** clank (~0.75 s) < pi (~2.2 s) < omp (~3.4 s) < opencode (~5.7 s).
- **Disk:** clank 6.4 MB binary vs agent install trees of ~200–700 MB (+ omp natives ~347 MB).

Use **clank** for scripts/pipes/CI; use **omp/pi/opencode** when you want the agent product. Fix clank’s custom-header gap (or native Go session) to drop the proxy crutch.
