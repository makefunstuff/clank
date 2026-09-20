---
description: one invocation is one request, so a retry or a second call is a decision the caller did not make
globs: ["src/**/*.rs"]
condition: '(?i)\bretry\b|\bbackoff\b|for attempt|max_attempts|reconnect'
scope: "tool:edit(*.rs), tool:write(*.rs)"
interruptMode: always
repeatMode: once
---

PROTOCOL's request sequence is a contract: one invocation is one request,
`--tools` adds only the rounds the model asks for, and a transport failure is
exit 1 with a reason. A retry, a reconnect or a fallback model spends the
caller's budget on a decision the caller did not make.
