#!/usr/bin/env python3
"""Rewrap the prose in this repository's markdown to ~80 columns.

One convention, one tool: tables, headings, blockquotes and fenced code are left
byte-identical, inline code spans, links and URLs are atomic, and a bullet's continuation
lines keep the marker's hanging indent. The control is exact and fence-aware — over the
prose lines only, the whitespace-normalised text must be identical before and after — so
the rewrite cannot change a word.

    python3 scripts/reflow-docs.py --check $(git ls-files '*.md')   # exit 1 on drift
    python3 scripts/reflow-docs.py FILE...                          # rewrite in place

Exits: 0 nothing to do (or, without --check, everything written), 1 a file would change
under --check or could not be rewritten without changing words, 2 usage.

"""
import re
import sys

WIDTH = 80
ATOMIC = re.compile(r"`[^`]*`|\]\([^)]*\)|https?://\S+|\S+")


def tokens(text):
    """(token, separated_by_space) pairs, with code spans, links and URLs kept whole.

    The original spacing is carried per token rather than inferred, so a comma that sat
    against a code span stays against it and a wrap can only ever replace a space that was
    already there.
    """
    out, end = [], 0
    for m in ATOMIC.finditer(text):
        out.append((m.group(0), bool(text[end:m.start()])))
        end = m.end()
    return out


def wrap(parts, first_prefix, rest_prefix):
    out, line = [], first_prefix
    for tok, sep in parts:
        add = ("" if line.endswith(" ") else (" " if sep else "")) + tok
        if line.strip() and sep and len(line) + len(add) > WIDTH:
            out.append(line.rstrip())
            line = rest_prefix + tok
        else:
            line += add
    if line.strip():
        out.append(line.rstrip())
    return out


def fenced(lines):
    """True for every line inside a ``` fence, including its delimiters."""
    inside, out = False, []
    for line in lines:
        fence = line.lstrip().startswith("```")
        out.append(inside or fence)
        if fence:
            inside = not inside
    return out


def prose(line):
    s = line.strip()
    return bool(s) and not s.startswith(("|", "```", "#", ">", "<"))


def bullet(line):
    """(whole marker including its indent, bare indent) for a list item, else None."""
    m = re.match(r"^(\s*)(-|\*|\+|\d+\.)\s+", line)
    return (m.group(0), m.group(1)) if m else None


def reflow(text):
    lines = text.split("\n")
    skip = fenced(lines)
    out, i = [], 0
    while i < len(lines):
        line = lines[i]
        if skip[i] or not prose(line):
            out.append(line)
            i += 1
            continue
        block = [line]
        i += 1
        while i < len(lines) and not skip[i] and prose(lines[i]) and not bullet(lines[i]):
            if len(lines[i]) - len(lines[i].lstrip()) < len(line) - len(line.lstrip()):
                break
            block.append(lines[i])
            i += 1
        marker = bullet(line)
        if marker:
            block[0] = block[0][len(marker[0]):]
            joined = " ".join(l.strip() for l in block)
            first, rest = marker[0], " " * len(marker[0])
        else:
            joined = " ".join(l.strip() for l in block)
            indent = " " * (len(line) - len(line.lstrip()))
            first = rest = indent
        out.extend(wrap(tokens(joined), first, rest))
    return "\n".join(out)


def normalise(text):
    lines = text.split("\n")
    skip = fenced(lines)
    keep = [l for l, s in zip(lines, skip) if not s and prose(l)]
    return re.sub(r"\s+", " ", " ".join(keep))


def main():
    args = sys.argv[1:]
    check = "--check" in args
    files = [a for a in args if not a.startswith("--")]
    if not files:
        print(__doc__.strip().splitlines()[-1], file=sys.stderr)
        return 2
    drifted, refused = [], []
    for path in files:
        before = open(path).read()
        after = reflow(before)
        if normalise(before) != normalise(after):
            refused.append(path)
            continue
        n0, n1 = len(before.splitlines()), len(after.splitlines())
        if n0 == n1 and before == after:
            continue
        if check:
            drifted.append(f"{path}: {n0} -> {n1} lines")
            continue
        open(path, "w").write(after)
        print(f"{path}: {n0} -> {n1} lines, words unchanged")
    if refused:
        print("refused (the rewrite would change words): " + ", ".join(refused), file=sys.stderr)
        return 1
    if drifted:
        print("not wrapped to the convention:\n  " + "\n  ".join(drifted), file=sys.stderr)
        print("fix with: python3 scripts/reflow-docs.py " + " ".join(files), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
