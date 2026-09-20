//! Guards the repository's own rules: `.jev/rules/*.json`.
//!
//! A rule file that does not parse is *skipped with a reason* by the server, and a rule
//! whose `applies_to` claims nothing can never produce a finding — silence either way,
//! which is indistinguishable from a clean tree. The server lints most of what follows
//! when it runs; this test is what catches it when the server is not running, which is
//! most of the time, and in CI, which never runs it.
//!
//! The last check is the one that is not a restatement of the format: a rule whose pattern
//! matches nothing in this tree must say so out loud, in `NO_CANDIDATE_IN_THIS_TREE`, so a
//! guard cannot rot into a comment (or the reverse) without someone deciding it.

use regex::Regex;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const SCHEMA: &str = "jev.rules/1";
const MAX_TITLE: usize = 60;
const VERBS: [&str; 8] = ["fix", "fixAll", "harden", "types", "docs", "rewrite", "test", "generate"];

/// Rules that find no candidate in this tree today, on purpose. Some name a construct that
/// is absent by construction and would be a regression if it appeared (an async runtime,
/// process-global state, a silent retry); the documentation rules name a habit this
/// repository's prose was cleaned of, one pattern at a time. Each was shown to fire on a
/// document written to violate it (the throwaway harness behind the rule set), so this
/// list is a statement about *this tree*, not a claim that the rules are decorative.
/// `system-prompt-is-not-a-style-guide` joined it on 2026-09-20, when the last style
/// directive left `src/tools.rs` and the rule became a guard against its return.
const NO_CANDIDATE_IN_THIS_TREE: [&str; 10] = [
    "doc-no-chatty-hedging",
    "doc-no-decorative-glyph",
    "doc-no-question-heading",
    "no-async-in-clank",
    "no-hidden-retry",
    "no-narration-labels",
    "no-process-global-state",
    "no-rhetorical-contrast",
    "no-unmeasured-superlative",
    "system-prompt-is-not-a-style-guide",
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every file in the repository, as a path relative to its root, with `/` separators.
/// Build output and scratch are not part of the tree a rule is written about.
fn tracked_files(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        if entry.path().is_dir() {
            if !matches!(name.as_str(), "target" | "local" | ".git") {
                tracked_files(&entry.path(), &rel, out);
            }
        } else {
            out.push(rel);
        }
    }
}

/// The glob language `applies_to` is written in: `**` is zero or more path segments,
/// `*` is zero or more characters inside one segment, `?` is one character.
fn glob_match(pattern: &str, path: &str) -> bool {
    let pat: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let seg: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    fn segments(pat: &[&str], seg: &[&str]) -> bool {
        match pat.split_first() {
            None => seg.is_empty(),
            Some((&"**", rest)) => (0..=seg.len()).any(|skip| segments(rest, &seg[skip..])),
            Some((&p, rest)) => match seg.split_first() {
                Some((&s, srest)) => segment(p, s) && segments(rest, srest),
                None => false,
            },
        }
    }
    fn segment(p: &str, s: &str) -> bool {
        fn here(p: &[char], s: &[char]) -> bool {
            match p.split_first() {
                None => s.is_empty(),
                Some(('*', rest)) => (0..=s.len()).any(|skip| here(rest, &s[skip..])),
                Some(('?', rest)) => match s.split_first() {
                    Some((_, srest)) => here(rest, srest),
                    None => false,
                },
                Some((&c, rest)) => match s.split_first() {
                    Some((&sc, srest)) if sc == c => here(rest, srest),
                    _ => false,
                },
            }
        }
        here(&p.chars().collect::<Vec<_>>(), &s.chars().collect::<Vec<_>>())
    }
    segments(&pat, &seg)
}

struct Rule {
    file: String,
    id: String,
    applies_to: Vec<String>,
    pattern: Option<String>,
    kind: String,
}

fn rules() -> Vec<Rule> {
    let dir = root().join(".jev/rules");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no rules: a repository with none gets no findings at all");

    let mut out = Vec::new();
    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {name}: {e}"));
        let doc: Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("{name} is not a rules document: {e}"));
        assert_eq!(
            doc.get("schema").and_then(Value::as_str),
            Some(SCHEMA),
            "{name} must carry \"schema\": {SCHEMA:?} exactly, or the whole file is skipped"
        );
        let body = doc.get("rules").and_then(Value::as_array);
        let body = body.unwrap_or_else(|| panic!("{name} must carry a `rules` array"));
        assert!(!body.is_empty(), "{name} declares no rules");
        for rule in body {
            let id = rule.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
            assert_eq!(
                format!("{id}.json"),
                name,
                "{name}: the file is named after its rule, one rule per file"
            );
            for field in ["text", "title"] {
                assert!(
                    rule.get(field).and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty()),
                    "{name}: {field} is required"
                );
            }
            let title = rule["title"].as_str().unwrap().to_string();
            assert!(
                title.chars().count() <= MAX_TITLE,
                "{name}: the title is {} characters; a finding's label is clipped at {MAX_TITLE}",
                title.chars().count()
            );
            let severity = rule.get("severity").and_then(Value::as_str);
            assert!(
                matches!(severity, None | Some("information") | Some("warning")),
                "{name}: severity {severity:?} is outside information|warning"
            );
            if let Some(verb) = rule.get("verb_hint").and_then(Value::as_str) {
                assert!(VERBS.contains(&verb), "{name}: verb_hint {verb:?} is not a served verb");
            }
            let applies_to: Vec<String> = rule
                .get("applies_to")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            assert!(
                !applies_to.is_empty(),
                "{name}: an empty applies_to never claims a file and can never produce a finding"
            );

            let judgement = rule.get("judgement").expect("judgement is required");
            assert!(
                judgement.get("question").and_then(Value::as_str).is_some_and(|q| !q.trim().is_empty()),
                "{name}: judgement.question is required"
            );
            let criteria = judgement.get("criteria").expect("judgement.criteria is required");
            for side in ["true", "false"] {
                assert!(
                    criteria.get(side).and_then(Value::as_str).is_some_and(|c| !c.trim().is_empty()),
                    "{name}: judgement.criteria needs both `true` and `false`"
                );
            }
            if let Some(p) = judgement.get("min_probability").and_then(Value::as_f64) {
                assert!((0.0..=1.0).contains(&p), "{name}: min_probability {p} is outside [0, 1]");
            }

            let inspection = rule.get("inspection").expect("inspection is required");
            let kind = inspection.get("kind").and_then(Value::as_str).unwrap_or_default().to_string();
            assert!(
                matches!(kind.as_str(), "regex" | "absent"),
                "{name}: inspection.kind {kind:?} is outside regex|absent"
            );
            let pattern = inspection.get("pattern").and_then(Value::as_str).map(str::to_string);
            assert!(pattern.is_some(), "{name}: inspection.pattern is required");
            Regex::new(pattern.as_ref().unwrap())
                .unwrap_or_else(|e| panic!("{name}: the pattern does not compile, so it finds nothing: {e}"));

            out.push(Rule { file: name.clone(), id, applies_to, pattern, kind });
        }
    }

    let ids: BTreeSet<&str> = out.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids.len(), out.len(), "two rules share an id, which the caches key on");
    out
}

impl Rule {
    /// Whether this rule's inspection matches nothing anywhere it claims.
    fn inspection_is_quiet(&self) -> bool {
        let Some(pattern) = &self.pattern else { return true };
        let re = Regex::new(pattern).expect("compiled already");
        let mut files = Vec::new();
        tracked_files(&root(), "", &mut files);
        for path in &files {
            if !self.applies_to.iter().any(|g| glob_match(g, path)) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(root().join(path)) else { continue };
            let matched = match self.kind.as_str() {
                "absent" => !re.is_match(&text),
                _ => text.lines().any(|l| re.is_match(l)),
            };
            if matched {
                return false;
            }
        }
        true
    }
}

#[test]
fn every_rule_is_loadable_and_claimable() {
    let all = rules();
    println!("{} rules in .jev/rules", all.len());
}

#[test]
fn a_rule_that_matches_nothing_says_so() {
    for rule in rules() {
        let quiet = rule.inspection_is_quiet();
        let declared = NO_CANDIDATE_IN_THIS_TREE.contains(&rule.id.as_str());
        assert_eq!(
            quiet, declared,
            "{}: {} in this tree, so {}",
            rule.file,
            if quiet { "no candidate" } else { "candidates found" },
            if quiet {
                "add it to NO_CANDIDATE_IN_THIS_TREE to say that was intended"
            } else {
                "remove it from NO_CANDIDATE_IN_THIS_TREE"
            }
        );
    }
}

#[test]
fn the_glob_language_is_the_one_the_server_uses() {
    assert!(glob_match("src/**/*.rs", "src/main.rs"));
    assert!(glob_match("**/*.rs", "src/deep/mod.rs"));
    assert!(glob_match("*.md", "README.md"));
    assert!(!glob_match("*.md", "docs/README.md"));
    assert!(glob_match("docs/**/*.md", "docs/history/pi-review.md"));
    assert!(!glob_match("docs/**/*.md", "docs/pi-review.txt"));
    assert!(glob_match("src/main.rs", "src/main.rs"));
    assert!(!glob_match("src/main.rs", "src/client.rs"));
}
