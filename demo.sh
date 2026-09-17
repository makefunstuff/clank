#!/usr/bin/env bash
# clank demo: a four-stage unix pipeline driven by a local model.
#
# The shape: every stage is a pipe, the shell gathers the evidence, and clank is
# the model-shaped step in the middle. Nothing here asks the model to read the
# filesystem for itself - the fixture, the candidate script and the previous
# stage's trace are all fed in as pipe or file context, so the input of every
# stage is visible in this script.
#
#   stage 1  generate : the fixture piped in        -> a bash script
#   stage 2  critique : script + fixture + the shell's own measurements piped in
#                                                   -> concrete defects
#   stage 3  finalize : candidate + stage 2's trace -> corrected script as JSON
#                       (--json-schema: one request, grammar enforced by the server)
#   stage 4  one-liner: the corrected script piped in -> a verifying one-liner
#                       as JSON
#
# Every stage runs with --thinking off. Two reasons: the schema-constrained
# stages would otherwise spend the budget thinking inside the grammar, and this
# endpoint's template leaks a `</think>` tag into the answer for strict-output
# prompts when thinking is on. clank passes the model's text through verbatim -
# it does not strip tags - so the caller turns thinking off instead.
#
# Every stage is gated: a non-zero exit, an answer that is not valid JSON, or a
# script that fails `bash -n` stops the demo with a non-zero exit code. A demo
# that prints "passed" after producing a broken artifact is worse than no demo.
#
# Traces: one --jsonl file per stage under local/demo-traces/ (gitignored).
# NOTE: model output is data in this script - it is syntax-checked but never
# executed. Inspect the traces and run things yourself.
cd "$(dirname "$0")"

C=./target/release/clank
M=${CLANK_MODEL:-qwen3.6-35b-a3b-iq3xxs}
B=${CLANK_BASE_URL:-http://127.0.0.1:37313/v1}
T=local/demo-traces
mkdir -p "$T"

die() { echo "demo: FAILED: $*" >&2; exit 1; }
answer() { jq -er 'select(.type=="assistant") | .content' "$1" || die "$2: no answer in the trace"; }

echo "== stage 1: generate"
cat fixtures/notes.md | "$C" --model "$M" --base-url "$B" --thinking off \
  -m 'The context is a note file. Write a bash script that reads fixtures/notes.md and prints it as a valid HTML5 page to stdout: a doctype, exactly one <h1>Notes</h1> heading, and one <p> containing the note text. Output ONLY the script text - no markdown fences, no commentary.' \
  --jsonl > "$T/1-generate.jsonl" || die "stage 1: clank exited non-zero"
answer "$T/1-generate.jsonl" "stage 1" > "$T/candidate.sh"
bash -n "$T/candidate.sh" || die "stage 1: candidate.sh does not pass bash -n"
echo "  candidate.sh: $(wc -l < "$T/candidate.sh") lines, bash -n ok"

echo "== stage 2: critique"
{
  printf -- '─── candidate.sh ───\n'
  cat "$T/candidate.sh"
  printf -- '\n─── fixtures/notes.md (the file the script must read) ───\n'
  cat fixtures/notes.md
  printf -- '\n─── measurements taken by the shell ───\n'
  printf 'wc -c fixtures/notes.md: %s\n' "$(wc -c < fixtures/notes.md)"
  printf 'bash -n candidate.sh: %s\n' "$(bash -n "$T/candidate.sh" >/dev/null 2>&1 && echo 'syntax ok' || echo 'SYNTAX ERROR')"
} | "$C" --model "$M" --base-url "$B" --thinking off \
  -m 'Critique the candidate script in the context against the file it must read and the measurements the shell took. Every claim you make must be checkable against the context: quoting and escaping, the doctype, the <h1>Notes</h1> heading, the <p>. List concrete defects, or state that it is correct.' \
  --jsonl > "$T/2-critique.jsonl" || die "stage 2: clank exited non-zero"
answer "$T/2-critique.jsonl" "stage 2" > "$T/critique.txt"
echo "  critique: $(wc -w < "$T/critique.txt") words"

echo "== stage 3: finalize"
cat "$T/2-critique.jsonl" | "$C" --model "$M" --base-url "$B" --thinking off \
  -c "$T/candidate.sh" \
  -m 'The context is a candidate bash script (the file node) and the transcript of a critique of it. Output the final corrected script as JSON matching the schema: an object with a single string property "script". Output ONLY the JSON.' \
  --json-schema '{"type":"object","properties":{"script":{"type":"string"}},"required":["script"]}' \
  --jsonl > "$T/3-finalize.jsonl" || die "stage 3: clank exited non-zero"
answer "$T/3-finalize.jsonl" "stage 3" | jq -er .script > "$T/final.sh" \
  || die "stage 3: answer was not an object with a string 'script'"
bash -n "$T/final.sh" || die "stage 3: final.sh does not pass bash -n"
echo "  final.sh: $(wc -l < "$T/final.sh") lines, bash -n ok"

echo "== stage 4: one-liner"
cat "$T/final.sh" | "$C" --model "$M" --base-url "$B" --thinking off \
  -m 'The context is the content of a bash script. Propose a single bash one-liner that runs `bash local/demo-traces/final.sh`, verifies the output contains <html and the heading Notes, and prints VERIFIED on success. Output JSON matching the schema: an object with a single string property "cmd". Output ONLY the JSON.' \
  --json-schema '{"type":"object","properties":{"cmd":{"type":"string"}},"required":["cmd"]}' \
  --jsonl > "$T/4-oneliner.jsonl" || die "stage 4: clank exited non-zero"
CMD=$(answer "$T/4-oneliner.jsonl" "stage 4" | jq -er .cmd) \
  || die "stage 4: answer was not an object with a string 'cmd'"
# syntax-check only: model output is data, never executed by this script
bash -n <(printf '%s' "$CMD") || die "stage 4: one-liner does not pass bash -n"
echo "  one-liner (syntax-checked, not executed): $CMD"

echo "demo: all four stages passed (model output never executed)"
