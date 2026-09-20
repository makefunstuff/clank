---
description: clank observes and proposes; a write that touches the filesystem is outside its capability boundary
globs: ["src/**/*.rs"]
condition: 'fs::write|File::create|OpenOptions|create_dir|remove_file|remove_dir|set_permissions|fs::rename'
scope: "tool:edit(*.rs), tool:write(*.rs)"
interruptMode: always
repeatMode: once
---

The capability boundary is observation: no writes, no shell, no network beyond
the model endpoint. clank proposes and the shell or the human applies (PROTOCOL
invariant 4). The one file the contract allows is the request dump the caller
opts into with `CLANK_DEBUG`.
