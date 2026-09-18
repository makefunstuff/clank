#!/usr/bin/env bash
# Record docs/images/clank-demo.gif — the README demo — from a real session.
#
#   scripts/record-demo.sh
#
# The tape (scripts/demo.tape) drives the real binary against a live endpoint:
# every command in the recording actually ran, and the model's answers in the
# GIF are the ones it gave. Nothing here executes model-authored text; the
# commands in the tape are ours.
#
# The endpoint and the model come from CLANK_BASE_URL / CLANK_MODEL, the same
# variables clank itself reads, so the recording names what produced it.
#
# Gates, in order - any failure exits non-zero:
#   1. vhs, ttyd, ffmpeg, jq, rg, curl present
#   2. vhs is a version that can render (0.12.0 cannot, see below)
#   3. target/release/clank exists (built if needed)
#   4. the endpoint answers and serves the model
#   5. the model is warm (one throwaway request, so the tape does not record a
#      cold model load as dead air)
#   6. the recording wrote a new GIF that is not empty
#
# Known upstream bug, and the reason for gate 2: vhs 0.12.0 cancels the context
# it then hands to Render(), so ffmpeg never starts, the error is swallowed, and
# vhs exits 0 having written nothing. 0.11.0 renders correctly.
#
# Requires: vhs, ttyd, ffmpeg, jq, ripgrep, curl.
#   mise install vhs@0.11.0 ttyd@1.7.7

set -uo pipefail

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT" || exit 1

TAPE=scripts/demo.tape
OUT=docs/images/clank-demo.gif
MIN_BYTES=50000

CLANK_BASE_URL=${CLANK_BASE_URL:-http://127.0.0.1:40583/v1}
CLANK_MODEL=${CLANK_MODEL:-qwen3.8-27b-gsq-rco-iq3xxs}
export CLANK_BASE_URL CLANK_MODEL

die() { printf 'record-demo: FAILED: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

for tool in vhs ttyd ffmpeg jq rg curl; do
  have "$tool" || die "$tool not on PATH"
done

# Presence is not enough for ttyd: a mise shim with no version selected is on
# the PATH and still cannot run, and vhs only fails on it much later.
ttyd --version 2>/dev/null | grep -q '^ttyd version' \
  || die "'ttyd --version' printed no version (mise shim without a selected version?). Select one: mise use -g ttyd@1.7.7"

# Gate 2: vhs 0.12.0 renders nothing and still exits 0.
vhs_version=$(vhs --version 2>&1 | sed -n 's/.*version v\([0-9][0-9.]*\).*/\1/p')
[ -n "$vhs_version" ] || die "could not read a version out of 'vhs --version'. A mise shim with no version selected fails this way (it prints mise's error instead); select one: mise use -g vhs@0.11.0"
case "$vhs_version" in
  0.1[2-9].*|[1-9].*) die "vhs $vhs_version cannot render: 0.12.0 passes an already-cancelled context to ffmpeg, so it exits 0 without writing a file (verified). Install 0.11.0: mise install vhs@0.11.0" ;;
esac

# Gate 3: the binary the recording runs. The tape types `clank`, so the release
# directory goes on PATH for the recorded shell (vhs passes its own environment
# to ttyd, and ttyd's shell inherits it).
[ -x target/release/clank ] || cargo build --release || die "cargo build --release failed"
export PATH="$ROOT/target/release:$PATH"
printf 'record-demo: %s (vhs %s)\n' "$(clank -V 2>/dev/null || echo 'clank (unknown version)')" "$vhs_version"

# Gate 4: the endpoint serves the model the recording will name.
models=$(curl -s -m 10 "$CLANK_BASE_URL/models") || die "no answer from $CLANK_BASE_URL"
printf '%s' "$models" | jq -e --arg m "$CLANK_MODEL" '.data[] | select(.id == $m)' >/dev/null 2>&1 \
  || die "$CLANK_MODEL is not served by $CLANK_BASE_URL (models: $(printf '%s' "$models" | jq -rc '[.data[].id]' 2>/dev/null || echo unparsable))"

# Gate 5: warm the model. A cold load took 61 s on this box; recorded, that is
# a minute of a frozen terminal. The budget is generous on purpose: a truncated
# answer is a failure, and a warm-up that fails on truncation is a false alarm.
printf 'record-demo: warming %s\n' "$CLANK_MODEL"
if ! clank --thinking off --max-tokens 24 -m 'reply with exactly: pong' >/dev/null 2>&1; then
  die "warm-up request failed against $CLANK_BASE_URL"
fi

# Gate 6: the recording itself.
before=$(stat -c %Y "$OUT" 2>/dev/null || echo 0)
printf 'record-demo: recording %s\n' "$TAPE"
vhs "$TAPE" >/dev/null || die "vhs exited non-zero"
[ -f "$OUT" ] || die "vhs wrote no $OUT"

after=$(stat -c %Y "$OUT")
[ "$after" -gt "$before" ] || die "$OUT was not rewritten (still from an earlier run)"
size=$(stat -c %s "$OUT")
[ "$size" -ge "$MIN_BYTES" ] || die "$OUT is only $size bytes - the recording is empty or broken"

printf 'record-demo: %s rewritten, %s bytes, %sx%s, %s s\n' \
  "$OUT" "$size" \
  "$(ffprobe -v error -select_streams v:0 -show_entries stream=width -of csv=p=0 "$OUT")" \
  "$(ffprobe -v error -select_streams v:0 -show_entries stream=height -of csv=p=0 "$OUT")" \
  "$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$OUT")"
printf 'record-demo: model %s @ %s\n' "$CLANK_MODEL" "$CLANK_BASE_URL"
