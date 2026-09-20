# clank — status

Project log, last written 2026-09-20. The contract is
[PROTOCOL.md](PROTOCOL.md); the reference is [README.md](README.md); the dated
evidence is
[docs/history/verification-log.md](docs/history/verification-log.md). This file
says where the project is, what is open, and how to check any of it.

## Open questions

1. **The `.jev/rules` floors (`min_probability` 0.7–0.8) have met a real
   decision model, and held.** Through the editor, against `typesafe/jev-1.13`
   on OpenRouter: the 26-rule set over seven files claims 14–15 rules per `src`
   file, finds 8–60 candidates each, and publishes **no** findings — the
   overengineering and prose rules cried wolf nowhere (`src/jev.rs`: 60
   candidates, 0 findings). What is still unmeasured is the *band*: the
   per-answer probability of a candidate the judge rejected is not exposed, so a
   floor cannot yet be set just above it. Re-run with `:Jev inspect` after a
   rules edit, or `jev inspect <file> --force` with `JEV_DECIDE_BASE_URL`
   pointed at a local `kev`.
2. **The repository has not been released.** No tag exists, so the release
   workflow's Actions plumbing is unexercised; only its shell has been run,
   locally. The first `git tag v0.1.0 && git push origin v0.1.0` is the test.
3. **Whether to keep the 2026-09-20 splits**: `docs/clank-jev.md` (out of
   README) and `docs/history/` (closed records). Both are reversible with
   `git mv`.

## Resolved on 2026-09-20

- **`- Be concise.` is gone from both built-in prompts.** PROTOCOL.md states the
  prompt does not shape tone, length or formatting and argues it at length; the
  code carried the directive anyway, and the rule fired on it. The rule is now a
  guard against its return: `system-prompt-is-not-a-style-guide` has no
  candidate in this tree and is listed as such in `tests/rules.rs`. Answers may
  run longer.
- **Two rules were retired, killed by the smoke test.** `code-no-avoidable-copy`
  and `tools-and-schema-never-share-a-request` each found their candidates live
  but the judge declined every purpose-built violation, and on re-reading the
  fixtures it was right: `let owned = text.to_string(); owned.len()` is
  pointless rather than defective, and a `Request { tools, json_schema }`
  literal says nothing about what the caller does with it. Both questions need
  knowledge a line and a 200-line excerpt do not carry, so the rules could never
  speak. The wire test already guards the request split
  (`the_schema_never_travels_in_the_same_request_as_tools`).
- **Three serialisation fallbacks fail loudly.** `--list-tools`, the schema
  embedded in the final request, and `clank-jev`'s result object all printed an
  empty line and exited 0 if a value they built themselves failed to serialise —
  unreachable in practice, and a lie if it ever happened. They now report the
  failure and exit 1 (`Fail::Internal`, added for exactly this), PROTOCOL's
  exit-code row for `1` says so, and the `CLANK_DEBUG` dump writes the reason
  into the file rather than leaving an empty body.

## Where it is

- `clank` 0.1.0: one prompt, one request, one answer — the six invariants in
  PROTOCOL.md. `clank-jev` 0.1.0: typed decisions for routing, providers
  `typesafe`, `openrouter`, `kev`.
- 81 tests (`cargo test`), all against stub servers: no model, no key, no
  network. `tests/wire.rs` (24, the protocol), `tests/jev.rs` (10, the sibling),
  `tests/docs.rs` (6, docs vs code), `tests/rules.rs` (3, the rules themselves),
  plus 38 unit tests in `src/`.
- CI on every push and pull request: warnings are errors, `cargo test`, and the
  docs wrapping check. Release on a `v*` tag: three native targets, each archive
  smoke-tested before it is attached, published only when all of them are up.
- `.jev/rules/` holds 26 rules, in three groups: the contract's invariants, the
  taste the code is held to (weightless code, needless abstraction, avoidable
  copies, silent fallbacks, a value with two homes, comments that restate the
  code, assertions no plausible bug would fail), and the documentation habits
  the prose was cleaned of. Enforced in an editor by Jev and in CI by
  `tests/rules.rs`; `.jev/README.md` is the index.

## How to check it

```sh
cargo test                                            # 81 tests, no model needed
python3 scripts/reflow-docs.py --check $(git ls-files '*.md' ':!fixtures/*')   # doc convention
python3 local/verify-rules.py                         # machine-local scratch: needs jev and a stub
```

`local/` is gitignored. It holds the throwaway harness behind the rule set — it
starts a stub decision endpoint, rebuilds a probe repository, and checks that
each rule loads, fires on a document written to violate it, and respects its
floor — plus the scratch trees it builds. Nothing in the repository depends on
it; `tests/rules.rs` is what CI runs.

## Known limits

- Everything measured against a model is a dated record, not a re-runnable
  claim: it needs the endpoint it was measured on. The log says which.
- `--tools` against a live model below ~9B is unreliable; the fallback exists
  for the empty case, and the exit code, not the feature, is what survives a
  model that fabricates.
- No sandbox: clank runs with your permissions, and `read_file` reaches anything
  you can read. PROTOCOL.md says so; a container with the tree mounted read-only
  is the containment.

## In the editor, on this machine

Verified 2026-09-20 through OMP: `jev-lsp` runs against this repository, reports
`rules.loaded: 14`, and answers `:Jev inspect` with real judgements from
`typesafe/jev-1.13` — findings on `src/tools.rs` (question 1), none across the
docs.

rust-analyzer was broken in two ways, both fixed:

1. The pinned `nightly-2026-02-17` had no `rust-analyzer` component, so the
   server exited 1 and every `.rs` file went unanalyzed.
2. Once running it reported confident nonsense —
   `cannot index into a value of type &str`,
   `expected &[&str], found &[&str; 4]`, `None` as a variable that should be
   snake_case — on code `cargo check --all-targets` compiles without a warning.
   Cause: rust-analyzer discovers its sysroot by asking `rustc`, and this box
   resolves `rustc` to Arch's `/usr/bin/rustc`, whose sysroot `/usr` ships **no
   rust-src**, so `core` and `std` never loaded.

The fix is a stable toolchain matching the 1.98 compiler that builds the crate:

```sh
rustup toolchain install stable --profile minimal \
  --component rust-analyzer --component rust-src
```

`~/.local/bin/rust-analyzer-omp` puts that toolchain's `bin` first on `PATH` and
execs its analyzer, and `~/.omp/agent/lsp.json` registers it as
`rust-analyzer.command`. The pinned nightly stays the default toolchain,
untouched; backups sit beside `lsp.json`. Six files across `src/` and `tests/`
now come back with zero diagnostics, and the analyzer agrees with `cargo check`.

Two gotchas that cost time: `jev.inspect` matches its `{path}` with `ends_with`,
so pass an absolute path — a relative `README.md` matched a scratch copy under
`local/` and reported *"no rule applies"* for the repository's own README. And
editing a rule does not repaint findings already on screen:
`:Jev inspect --force`, or `jev.recompute`.
