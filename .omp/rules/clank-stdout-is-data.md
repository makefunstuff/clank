---
description: stdout carries the answer or events, so a breadcrumb written there is not data
globs: ["src/**/*.rs"]
condition: '\bprintln!|\bprint!'
scope: "tool:edit(*.rs), tool:write(*.rs)"
interruptMode: never
repeatMode: once
---

stdout is the answer, the framed `--each` answers, `--jsonl` events, or
`--list-tools`/`--help` output, and nothing else — a consumer pipes it into `jq`
(PROTOCOL invariant 6). Breadcrumbs, progress lines and diagnostics belong on
stderr, where `-q` can silence them.
