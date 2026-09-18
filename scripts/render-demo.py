#!/usr/bin/env python3
"""Capture a real clank session and render it as a terminal-window SVG.

The README image is generated, never hand-drawn: this runs the commands below
against a live endpoint, keeps their actual output, and writes the SVG. Re-run
it after changing anything the demo shows:

    python3 scripts/render-demo.py            # writes docs/images/clank-demo.svg

Stdlib only. The output is a static image of a real session - the model's
answers differ between runs, which is the point.
"""

import html
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "docs/images/clank-demo.svg"

# (command, timeout seconds) - short, real, and each one shows a different part
# of the contract: tools as data, the pipe as context, the framed map, a schema
# from a file gated by jq, and a failure that is loud.
DEMO = [
    ("clank --list-tools | jq -r '.[].function.name'", 20),
    ("rg -n userData fixtures | clank --thinking off --max-tokens 90 "
     "-m 'what does this do? one line'", 120),
    # each item carries its own evidence: the line is the question
    ("printf 'the build is green\\ntwo tests failed\\ndeploy paused\\n' "
     "| clank --each --thinking off --max-tokens 40 "
     "--json-schema @fixtures/verdict.schema.json "
     "-m 'is this line good or bad news? verdict ok or risk'", 180),
    ("echo 'the build is green' | clank --thinking off --max-tokens 150 "
     "--json-schema @fixtures/verdict.schema.json -m 'verdict and reason' | jq -c", 120),
    # a failure is loud and it has an exit code, which is the whole contract
    # stdout is discarded so the image shows the contract, not the stub answer
    ("clank --thinking off --max-tokens 4 -m 'write a paragraph about socks' "
     ">/dev/null; echo exit=$?", 60),
]

WIDTH = 1040
PAD = 22
BAR = 34
LINE = 20
FONT = 13
MAX_CHARS = 112


def capture(command: str, timeout: int) -> list[str]:
    # stderr is merged so the image shows lines in the order they happened
    proc = subprocess.run(
        ["bash", "-lc", command],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        timeout=timeout,
    )
    text = proc.stdout.rstrip("\n")
    return text.split("\n") if text else [""]


def clip(line: str) -> str:
    return line if len(line) <= MAX_CHARS else line[: MAX_CHARS - 1] + "…"


def build() -> str:
    rows: list[tuple[str, str]] = []  # (kind, text)
    for command, timeout in DEMO:
        rows.append(("cmd", "$ " + command))
        for line in capture(command, timeout):
            rows.append(("out", clip(line)))
        rows.append(("gap", ""))

    height = BAR + PAD + LINE * len(rows) + PAD
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{height}" '
        f'viewBox="0 0 {WIDTH} {height}" font-family="ui-monospace, SFMono-Regular, '
        f'Menlo, Consolas, monospace" font-size="{FONT}">',
        f'<rect width="{WIDTH}" height="{height}" rx="10" fill="#1e1e2e"/>',
        f'<path d="M0 10a10 10 0 0 1 10-10h{WIDTH - 20}a10 10 0 0 1 10 10v{BAR - 10}'
        f'H0z" fill="#181825"/>',
    ]
    for i, colour in enumerate(("#f38ba8", "#f9e2af", "#a6e3a1")):
        parts.append(f'<circle cx="{PAD + i * 18}" cy="{BAR // 2}" r="6" fill="{colour}"/>')
    parts.append(
        f'<text x="{WIDTH / 2}" y="{BAR // 2 + 4}" fill="#6c7086" font-size="{FONT - 1}" '
        f'text-anchor="middle">clank -- ~/Work/clank</text>'
    )

    y = BAR + PAD
    for kind, text in rows:
        y += LINE
        if kind == "gap":
            continue
        colour = "#89b4fa" if kind == "cmd" else "#cdd6f4"
        weight = " font-weight=\"600\"" if kind == "cmd" else ""
        parts.append(
            f'<text x="{PAD}" y="{y}" fill="{colour}"{weight} xml:space="preserve">'
            f"{html.escape(text)}</text>"
        )
    parts.append("</svg>")
    return "\n".join(parts) + "\n"


def main() -> int:
    svg = build()
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(svg)
    print(f"wrote {OUT.relative_to(ROOT)} ({len(svg)} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
