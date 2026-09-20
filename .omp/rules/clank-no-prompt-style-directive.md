---
description: the built-in system prompt states rules, it does not shape tone or length
globs: ["src/**/*.rs"]
condition: '(?i)be (concise|brief|short|terse)|keep (it|your (answer|response)|the answer) (short|brief|concise)'
scope: "tool:edit(*.rs), tool:write(*.rs)"
interruptMode: always
repeatMode: once
---

The built-in system prompt states what the model may assume and may not do, plus
how to answer when the context lacks the answer. It does not shape tone, length
or formatting: every added rule is harness influence the reader can no longer
attribute, and wording effects are not measurable through clank. Task-specific
instructions belong in `--system @file`.
