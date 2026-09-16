#!/usr/bin/env bash
# clank demo: a 4-stage unix pipeline driven by a local model.
#
#   stage 1  generate : bash script that renders fixtures/notes.md as HTML
#                       (tools available; the model reads the fixture itself)
#   stage 2  critique : critic checks the script's claims against the
#                       filesystem with its read-only tools
#   stage 3  finalize : corrected script, emitted as JSON per --json-schema
#                       (tools off: this server build rejects tools + schema)
#   stage 4  one-liner : model proposes a single bash one-liner that runs
#                       and verifies stage 3; syntax-checked only - model
#                       output is never executed by this script
# Traces: one --jsonl file per stage under local/demo-traces/ (gitignored).
#
# NOTE: model output is data in this script - it is syntax-checked but
# never executed. Inspect the traces and run things yourself.
cd "$(dirname "$0")"

C=./target/release/clank
M=${CLANK_MODEL:-qwen3.6-35b-a3b-iq3xxs}
B=${CLANK_BASE_URL:-http://127.0.0.1:37313/v1}
T=local/demo-traces
mkdir -p "$T"

echo "== stage 1: generate"
S1=$("$C" --model "$M" --base-url "$B" \
  -m 'Write a bash script that reads fixtures/notes.md and prints a valid HTML5 page to stdout: a doctype, exactly one <h1>Notes</h1> heading, and one <p> containing the note text. Output ONLY the script text - no markdown fences, no commentary. You may read the fixture with your tools.' \
  --jsonl | tee "$T/1-generate.jsonl" | jq -r 'select(.type=="assistant") | .content')
echo "  $(printf '%s' "$S1" | wc -l) lines of script"

echo "== stage 2: critique"
S2=$("$C" --model "$M" --base-url "$B" \
  -c <(printf '%s' "$S1") \
  -m 'The context is a bash script that renders fixtures/notes.md as HTML. Critique it. Use your tools to verify every claim: does fixtures/notes.md exist? Does the script quote and escape correctly? Would it emit the promised doctype, <h1>Notes</h1> and <p>? List concrete defects, or state that it is correct.' \
  --jsonl | tee "$T/2-critique.jsonl" | jq -r 'select(.type=="assistant") | .content')
echo "  critique: $(printf '%s' "$S2" | wc -w) words"

echo "== stage 3: finalize"
S3=$("$C" --model "$M" --base-url "$B" \
  -c <(printf '%s\n\n--- critique ---\n%s' "$S1" "$S2") \
  -m 'Using the critique, output the final corrected script as JSON matching the schema: an object with a single string property "script". Output ONLY the JSON.' \
  --json-schema '{"type":"object","properties":{"script":{"type":"string"}},"required":["script"]}' \
  --no-tools --jsonl | tee "$T/3-finalize.jsonl" | jq -r 'select(.type=="assistant") | .content')
printf '%s' "$S3" | jq -r .script > "$T/final.sh"
bash -n "$T/final.sh"
echo "  final.sh written, passes bash -n"

echo "== stage 4: one-liner"
CMD=$("$C" --model "$M" --base-url "$B" \
  -c "$T/final.sh" \
  -m 'The context is the content of the file local/demo-traces/final.sh. Propose a single bash one-liner that runs `bash local/demo-traces/final.sh`, verifies the output contains <html and the heading Notes, and prints VERIFIED on success. Output JSON matching the schema: an object with a single string property "cmd". Output ONLY the JSON.' \
  --json-schema '{"type":"object","properties":{"cmd":{"type":"string"}},"required":["cmd"]}' \
  --no-tools --jsonl | tee "$T/4-oneliner.jsonl" | jq -r 'select(.type=="assistant") | .content' | jq -r .cmd)
# syntax-check only: model output is data, never executed by this script
bash -n <(printf '%s' "$CMD")
echo "  one-liner (syntax-checked, not executed): $CMD"

echo "demo: all stages passed (model output never executed)"
