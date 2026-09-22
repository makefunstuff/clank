#!/bin/sh
# The secret-free jev gate: drive the built clank-jev against a stub endpoint,
# so the CLI, its checks file and the gate itself are exercised on the real wire
# protocol with no key, no network and no provider.
#
# The binary must be built first — `cargo build --locked --bin clank-jev`, which
# is what the ci workflow's `jev` job does before calling this.
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

bin=${CLANK_JEV_BIN:-target/debug/clank-jev}
if [ ! -x "$bin" ]; then
    echo "jev-ci-stub: $bin is not built; run: cargo build --locked --bin clank-jev" >&2
    exit 1
fi

work=$(mktemp -d)
stub_pid=''
cleanup() {
    [ -n "$stub_pid" ] && kill "$stub_pid" 2>/dev/null
    rm -rf "$work"
}
trap cleanup EXIT INT TERM

# The answers the stub gives. Ids and types match fixtures/checks-commit.json:
# `describes` is a yes/no question that also carries a reason set, so the CLI
# asks a second, closed-choice question for it under `<id>.reason`.
cat > "$work/reply.json" <<'JSON'
{"model": "stub-1", "answers": {
  "describes": {"type": "noul", "noul": 0.95},
  "describes.reason": {"type": "choice", "choice": "no_conflict",
            "probabilities": {"no_conflict": 0.95, "mismatch": 0.03, "missing_evidence": 0.02}},
  "shape": {"type": "choice", "choice": "conventional",
            "probabilities": {"conventional": 0.93, "plain": 0.05, "unclear": 0.02}}
}, "usage": {"input_tokens": 64, "output_tokens": 4}}
JSON

python3 scripts/jev-stub.py --reply "$work/reply.json" --url-file "$work/url" &
stub_pid=$!

i=0
while [ ! -s "$work/url" ]; do
    i=$((i + 1))
    if [ "$i" -gt 200 ]; then
        echo "jev-ci-stub: the stub never reported a URL" >&2
        exit 1
    fi
    sleep 0.05
done
url=$(cat "$work/url")

# A commit-shaped state: the message and the diff, which is what the checks
# fixture asks about.
cat > "$work/state" <<'STATE'
MESSAGE:
hooks: gate the push on a jev decision

DIFF:
diff --git a/.githooks/pre-push b/.githooks/pre-push
new file mode 100755
--- /dev/null
+++ b/.githooks/pre-push
@@ -0,0 +1,3 @@
+#!/usr/bin/env bash
+set -eu
+# ask clank-jev whether the message describes this diff
STATE

# The gate passes when every answer clears --min-prob. Two questions, so stdout
# is the result object, which is printed and left on the job log.
"$bin" --checks fixtures/checks-commit.json --provider kev --base-url "$url" \
    --min-prob 0.7 --quiet < "$work/state"

# ...and fails when the floor sits above the stub's confidence, so what is under
# test includes the gate and not just the request. Exit 1 is the failed gate;
# anything else here would mean the second run broke rather than gated.
set +e
"$bin" --checks fixtures/checks-commit.json --provider kev --base-url "$url" \
    --min-prob 0.99 --quiet < "$work/state" >/dev/null 2>&1
code=$?
set -e
if [ "$code" -ne 1 ]; then
    echo "jev-ci-stub: --min-prob 0.99 should have exited 1 (a failed gate), got $code" >&2
    exit 1
fi

echo "jev-ci-stub: checks wired (exit 0 at p>=0.7, exit 1 below 0.99)" >&2
