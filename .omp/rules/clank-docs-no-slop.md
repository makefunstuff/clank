---
description: this repository's prose is evidence, so slop and unmeasured claims do not belong in it
globs: ["**/*.md"]
condition: '(?i)\bnot just\b|\bgenuinely\b|\bseamless(ly)?\b|it''s worth noting|\bTODO\b|\bFIXME\b|\byou should\b|\bnote that\b'
scope: "tool:edit(*.md), tool:write(*.md)"
interruptMode: never
repeatMode: once
---

The documentation is read as evidence, not as marketing. State the claim; drop
the framing ("not just X, but Y"), the filler ("it is worth noting",
"basically") and the advice to the reader. Every number names the model,
endpoint, date or command that produced it. Leave no `TODO` in a document that
presents itself as finished.
