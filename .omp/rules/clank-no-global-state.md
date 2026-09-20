---
description: clank keeps no state across invocations, so a write that adds global state is a contract breach
globs: ["src/**/*.rs"]
condition: 'static mut|lazy_static!|OnceLock|OnceCell|thread_local!|Mutex<|RwLock<'
scope: "tool:edit(*.rs), tool:write(*.rs)"
interruptMode: always
repeatMode: once
---

Nothing persists between invocations: no session files, no daemon, no cache
clank owns, and no process-global mutable state — a cache the pipeline cannot
see or clear (PROTOCOL invariant 1). Constants and `&'static str` are not state;
a value that has to outlive the call belongs to the caller, not to a static.
