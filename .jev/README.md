# `.jev/rules` — the repository's conventions, as rules Jev can enforce

Each file is one rule in the `jev.rules/1` shape: an `inspection` regex finds
candidate lines locally (no model), and a `judgement` asks the decision model
about those lines. The regex cannot tell a doc comment from a write; the model
cannot be trusted to scan a file. Neither does the other's job.

The rules restate constraints already written down in
[PROTOCOL.md](../PROTOCOL.md), the taste this repository is held to, and the
habits its prose was cleaned of — several of them learned from a failure
recorded in
[docs/history/verification-log.md](../docs/history/verification-log.md). They
are held where a session will hit them: in the editor, on the line being
written.

## The contract (invariants)

| rule | claims | the question it asks |
|---|---|---|
| `no-async-in-clank` | `src/**/*.rs` | is this line async by construction, in a stage that promised one request in one process? |
| `no-process-global-state` | `src/**/*.rs` | does this state outlive the invocation, so a later run sees what an earlier one left? |
| `no-write-to-the-tree` | `src/**/*.rs` | does this write a path the caller did not ask for? |
| `stdout-is-data` | `src/**/*.rs` | would a consumer piping stdout into `jq` receive this as part of the answer? |
| `exit-code-never-lies` | `src/**/*.rs` | does this code contradict the exit-code table? |
| `no-panic-on-model-input` | `src/**/*.rs` | can a malformed answer panic the process instead of exiting 1 with a reason? |
| `no-second-endpoint-in-clank` | `src/**/*.rs` | is this a destination other than the configurable base URL? |
| `system-prompt-is-not-a-style-guide` | `src/tools.rs` | does this constrain tone, length or formatting instead of stating rules? |
| `no-hidden-retry` | `src/**/*.rs` | does this send a second request the caller did not ask for? |

## Taste: weightless code and needless abstraction

| rule | claims | the question it asks |
|---|---|---|
| `code-no-speculative-abstraction` | `src`, `tests` | is this trait, generic or trait object there for a second implementation that does not exist? |
| `code-no-wrapper-for-one-caller` | `src/**/*.rs` | is this function a name, or does it check something? |
| `code-no-silent-fallback` | `src/**/*.rs` | does a failure become a value the caller cannot tell from success? |
| `code-no-second-home-for-a-value` | `src/**/*.rs` | is this value stated somewhere else too, so the two can drift? |
| `code-no-comment-restating-the-code` | `src/**/*.rs` | does this comment say what rather than why? |
| `test-no-plumbing-assertion` | `tests/**/*.rs` | would any plausible bug fail this assertion, or does it pass for anything? |

## Documentation

| rule | claims | the question it asks |
|---|---|---|
| `measurement-names-its-source` | `*.md`, `docs/**/*.md` | is this a result with no model, endpoint, date or command behind it? |
| `no-unmeasured-superlative` | `*.md`, `docs/**/*.md` | is this adjective doing the work a number should do? |
| `no-rhetorical-contrast` | `*.md`, `docs/**/*.md` | is this claim framed as a contrast it does not need? |
| `no-narration-labels` | `*.md`, `docs/**/*.md` | is this a label over a result instead of the result? |
| `doc-no-chatty-hedging` | `*.md`, `docs/**/*.md` | does this phrase carry anything a deletion would lose? |
| `doc-no-second-person-advice` | `*.md`, `docs/**/*.md` | is this advice to a reader, or a fact about the subject? |
| `doc-no-placeholder` | `*.md`, `docs/**/*.md` | is this unfinished work shipped as finished? |
| `doc-no-decorative-glyph` | `*.md`, `docs/**/*.md` | does this glyph decorate, or does the text define it? |
| `doc-no-question-heading` | `*.md`, `docs/**/*.md` | could a noun phrase say the same as this question? |

## Running them

- In an editor with Jev pointed at this repository: ambient on save, or
  `:Jev inspect` for a pass on demand. `:Jev status` names the rules that
  loaded, the rules hash and the pass cost; `:Jev inspect` reports which files
  were skipped and why.
- Without an editor: `jev inspect <path> --force` from the repository root. Pass
  an **absolute** path — `jev.inspect` matches `{path}` with `ends_with`, so a
  relative one can match a copy of the same name elsewhere in the tree. The
  hosted decision tier needs a key; a local one is configured with
  `JEV_DECIDE_BASE_URL` and `JEV_DECIDE_WIRE`.
- A file that fails to parse, carries the wrong `schema`, or names a pattern
  that does not compile is **skipped with a reason**, and the rest of the
  directory still loads. Duplicate ids, a title over 60 characters, an empty
  title and a `min_probability` outside `[0, 1]` are reported by the server's
  own lint — which is why there are no rules here policing the rule format
  itself.
- `tests/rules.rs` holds the same shape as a test: every file parses, one rule
  per file, the globs claim something, and a rule whose pattern matches nothing
  in this tree must be listed — with a reason — rather than quietly guarding
  nothing.
- Editing a rule does not repaint findings already on screen: save the buffer,
  or `:Jev inspect --force` / `jev.recompute`.

### What the pass does not see

Four limits, all measured on 2026-09-20 by writing plausible violations into
this repository and watching what the pass made of them:

- **A rule asks about at most `max_candidates_per_rule` matches per document, in
  line order** (a client setting, 8 by default). A tenth `println!` in a long
  file is never put to the model, however wrong it is. Narrowing the pattern is
  the fix — see the next point for why raising the ceiling is not.
- **One decision call per rule per document, not per candidate.** A larger
  candidate set costs no more money, and does cost judgement: raising the
  ceiling from 8 to 32 silenced a rule that had been firing at 8 on the same
  document. Keep the batch small.
- **The same shape is caught in a small file and declined in a large one.** A
  progress `println!` and a `!value.is_empty()` assertion each produce a finding
  in a 20-line file and neither does in `src/main.rs` or `tests/wire.rs`. A
  clean large file is a weaker signal than a clean small one, so read silence as
  "nothing stood out", not as proof.
- **An ambient pass needs a trigger the client has to send.** The pass runs on
  save and on idle *after a change*, so a client that only opens files and pulls
  diagnostics never starts one: with a blatant `pub async fn` open for six
  seconds, the decision count did not move. Findings then appear only when a
  pass is asked for (`:Jev inspect`, `jev.inspect`), which is how every number
  in this section was produced.

## Adding a rule

One file per rule, named after its `id`. Keep the `title` under 60 characters,
make `applies_to` non-empty, and set `min_probability` above the band your own
false answers land in rather than at the default. A rule whose `applies_to`
matches no file is a comment, not a guard: `:Jev inspect` will report that no
rule claimed the file.

Write the *criteria* as if you were the one answering: the question the model is
asked should be decidable from the line, the excerpt around it and the rule's
own text, and the `false` side should name the legitimate case you expect to see
— a repair, a fixture, a quoted command — because criteria that name only the
defect produce findings on the repository's own code, and a rule that cries wolf
is worse than no rule.
