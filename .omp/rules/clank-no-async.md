---
description: clank is synchronous by construction, so a write that introduces async is a design change
globs: ["src/**/*.rs"]
condition: 'async fn|async move|\.await|tokio::|async_std'
scope: "tool:edit(*.rs), tool:write(*.rs)"
interruptMode: always
repeatMode: once
---

This stage is synchronous by construction: one invocation is one request in one
process, and parallelism belongs to `xargs -P` above clank (PROTOCOL invariants
1 and 5). An async runtime is a second scheduler inside a step. Write the change
synchronously.
