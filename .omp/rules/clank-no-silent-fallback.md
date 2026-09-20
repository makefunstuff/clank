---
description: a fallback indistinguishable from a real result reports a success the code cannot back
globs: ["src/**/*.rs"]
condition: 'unwrap_or\(|unwrap_or_default\(|unwrap_or_else\('
scope: "tool:edit(*.rs), tool:write(*.rs)"
interruptMode: never
repeatMode: once
---

Never report success you cannot back: a fallback that looks like a real answer
is a lie the caller cannot detect. If the value is needed, produce it or fail
with the reason on stderr and exit 1; a field the protocol may legitimately omit
is the one case where a default is honest.
