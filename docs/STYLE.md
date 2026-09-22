# Docs style (clank)

Short rules so README/CHEATSHEET stay a tool surface, not a manifesto.

1. **First win first.** Install → 30s smoke (`CLANK_BASE_URL` + `pong`) →
   PROTOCOL/CHEATSHEET. Design and history come later.
2. **Name the contract.** stdout = data, stderr = diagnostics, exit = verdict.
   Do not describe a chat product.
3. **No machine cosplay.** Do not sell laptop-only ports/model ids as universal
   defaults. Env/flags are required for real use.
4. **Evidence has a home.** Dated runs live under `docs/history/`; README links,
   it does not re-host essays.
5. **No costume clients.** Do not document bridge/`--opencode`-style remaps.
   Unsupported clients get a linked issue, not a workaround pitch.
6. **Register.** Concrete commands and measured claims. Token lists for banned
   marketing words and rhetorical contrast live in
   `.jev/rules/no-unmeasured-superlative.json` and
   `.jev/rules/no-rhetorical-contrast.json` — edit those, do not duplicate them
   here.
