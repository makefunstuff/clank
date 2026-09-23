//! Guards against documentation drift.
//!
//! Docs rot silently: a flag is added and no reference mentions it, an event
//! type is renamed and the contract still lists the old name. These checks read
//! the *sources of truth* — the binary's own `--help`, and the event list in
//! `src/context.rs` — and require the docs to agree.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &str) -> String {
    std::fs::read_to_string(repo().join(path)).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// Every long flag the binary offers, minus the two everyone knows.
fn long_flags() -> Vec<String> {
    flags_of(env!("CARGO_BIN_EXE_clank"))
}

/// The same, for the sibling binary: it has its own surface and its own contract,
/// so it gets the same guard rather than being remembered.
fn jev_flags() -> Vec<String> {
    flags_of(env!("CARGO_BIN_EXE_clank-jev"))
}

fn flags_of(bin: &str) -> Vec<String> {
    let flags = parse_long_flags(bin);
    assert!(flags.len() > 10, "parsed too few flags: {flags:?}");
    flags
}

fn parse_long_flags(bin: &str) -> Vec<String> {
    let out = Command::new(bin).arg("--help").output().expect("run --help");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut flags: Vec<String> = Vec::new();
    for token in text.split(|c: char| c.is_whitespace() || c == ',' || c == '[' || c == ']') {
        if let Some(rest) = token.strip_prefix("--") {
            let name = rest.trim_end_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
            if name.is_empty() || name == "help" || name == "version" {
                continue;
            }
            let flag = format!("--{name}");
            if !flags.contains(&flag) {
                flags.push(flag);
            }
        }
    }
    flags
}

/// The event types the code can emit, read from the one place that lists them.
fn event_types() -> Vec<String> {
    let source = read("src/context.rs");
    let line = source
        .lines()
        .find(|l| l.contains("const EVENT_TYPES"))
        .expect("context.rs must declare EVENT_TYPES");
    let inner = line.split('[').nth(2).expect("EVENT_TYPES array literal");
    let names: Vec<String> = inner
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect();
    assert_eq!(names.len(), 6, "expected six event types, got {names:?}");
    names
}

#[test]
fn every_flag_is_documented_in_the_reference_docs() {
    let readme = read("README.md");
    let cheatsheet = read("CHEATSHEET.md");
    let mut missing: Vec<String> = Vec::new();
    for flag in long_flags() {
        for (name, doc) in [("README.md", &readme), ("CHEATSHEET.md", &cheatsheet)] {
            if !doc.contains(&flag) {
                missing.push(format!("{flag} in {name}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "flags the binary offers but the docs do not mention: {missing:#?}"
    );
}

#[test]
fn every_clank_jev_flag_is_documented_in_the_reference_docs() {
    let readme = read("README.md");
    let jev = read("docs/clank-jev.md");
    let mut missing: Vec<String> = Vec::new();
    for flag in jev_flags() {
        for (name, doc) in [("README.md", &readme), ("docs/clank-jev.md", &jev)] {
            if !doc.contains(&flag) {
                missing.push(format!("{flag} in {name}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "flags clank-jev offers but the docs do not mention: {missing:#?}"
    );
}

#[test]
fn every_clank_web_flag_is_documented_in_the_reference_docs() {
    let readme = read("README.md");
    let web = read("docs/clank-web.md");
    let flags = parse_long_flags(env!("CARGO_BIN_EXE_clank-web"));
    assert!(flags.len() > 5, "parsed too few flags: {flags:?}");
    let mut missing: Vec<String> = Vec::new();
    for flag in flags {
        for (name, doc) in [("README.md", &readme), ("docs/clank-web.md", &web)] {
            if !doc.contains(&flag) {
                missing.push(format!("{flag} in {name}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "flags clank-web offers but the docs do not mention: {missing:#?}"
    );
}

#[test]
fn the_web_stage_contract_is_written_down_and_true() {
    let protocol = read("PROTOCOL.md");
    assert!(protocol.contains("clank-web"), "PROTOCOL.md must name the web stage");
    assert!(
        protocol.contains(".clank/config.toml"),
        "PROTOCOL.md must say which file holds the web config"
    );
    assert!(protocol.contains("CLANK_CONFIG"), "PROTOCOL.md must name the config-path override");
    assert!(protocol.contains("working directory"), "PROTOCOL.md must say discovery is the working directory");
    for code in ["0", "1", "2"] {
        assert!(
            protocol.contains(&format!("| `{code}` |")),
            "PROTOCOL.md has no row for clank-web exit code {code}"
        );
    }
    let bin = env!("CARGO_BIN_EXE_clank-web");
    let dir = std::env::temp_dir().join(format!("clank-web-docs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".clank")).unwrap();
    std::fs::write(dir.join(".clank/config.toml"), "[web]\ndefault_provider = \"brave\"\n").unwrap();
    let usage = Command::new(bin)
        .current_dir(&dir)
        .args(["--provider", "google", "q"])
        .env_remove("BRAVE_API_KEY")
        .env_remove("TAVILY_API_KEY")
        .env_remove("CLANK_CONFIG")
        .output()
        .unwrap();
    assert_eq!(usage.status.code(), Some(2), "an unknown provider is usage (2)");
    assert!(usage.stdout.is_empty(), "usage writes nothing to stdout");

    let missing = Command::new(bin)
        .current_dir(&dir)
        .args(["--provider", "brave", "q"])
        .env_remove("BRAVE_API_KEY")
        .env_remove("TAVILY_API_KEY")
        .env_remove("CLANK_CONFIG")
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(1), "a missing key is 1");
    assert!(missing.stdout.is_empty(), "a failed search writes nothing to stdout");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_decision_stage_contract_is_written_down_and_true() {
    // Its exit codes are a contract for scripts, so they must be in PROTOCOL.md
    // *and* be the ones the binary actually returns.
    let protocol = read("PROTOCOL.md");
    for code in ["0", "1", "2", "3"] {
        assert!(
            protocol.contains(&format!("| `{code}` |")) || protocol.contains(&format!("0/1/2/{code}")),
            "PROTOCOL.md has no row for clank-jev exit code {code}"
        );
    }
    let bin = env!("CARGO_BIN_EXE_clank-jev");
    let usage = Command::new(bin)
        .args(["--ask", "which?", "--choice", "a,b", "--provider", "nonsense"])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert_eq!(usage.status.code(), Some(2), "an unknown provider is usage (2)");

    // no credentials and no local server: the decision cannot be made, and that is
    // 3, not 1 — "could not ask" is a different failure from "did not pass".
    let env_clear = |cmd: &mut Command| {
        cmd.env_remove("TYPESAFE_API_KEY")
            .env_remove("JEV_API_KEY")
            .env_remove("JEV_CLI_API_KEY")
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("CLANK_CONFIG");
    };
    let mut cmd = Command::new(bin);
    cmd.args(["--ask", "which?", "--choice", "a,b", "--provider", "typesafe"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    env_clear(&mut cmd);
    let mut child = cmd.spawn().unwrap();
    {
        use std::io::Write;
        // The child exits 3 without reading stdin, so the write may fail with EPIPE; the
        // assertion is on the exit code. See tests/jev.rs.
        let _ = child.stdin.as_mut().unwrap().write_all(b"state");
    }
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(3), "missing credentials are 3");
}

#[test]
fn the_event_set_in_the_docs_matches_the_code() {
    let protocol = read("PROTOCOL.md");
    let readme = read("README.md");
    for event in event_types() {
        assert!(
            protocol.contains(&format!("| `{event}` |")),
            "PROTOCOL.md has no row for the `{event}` event"
        );
        assert!(
            readme.contains(&format!("`{event}`")),
            "README.md does not list the `{event}` event"
        );
    }
}

#[test]
fn the_exit_code_table_covers_every_code_the_binary_uses() {
    let protocol = read("PROTOCOL.md");
    for code in ["0", "1", "2"] {
        assert!(
            protocol.contains(&format!("| `{code}` |")),
            "PROTOCOL.md has no row for exit code {code}"
        );
    }
    // and the codes really are the ones in play
    let clank = env!("CARGO_BIN_EXE_clank");
    let usage = Command::new(clank).args(["--thinking", "bogus", "-m", "x"]).output().unwrap();
    assert_eq!(usage.status.code(), Some(2), "a usage error is 2");
    let no_prompt = Command::new(clank).stdin(std::process::Stdio::null()).output().unwrap();
    assert_eq!(no_prompt.status.code(), Some(2), "no prompt is 2");
}

#[test]
fn the_documented_file_paths_exist() {
    // Docs point at files; a rename must not leave a dangling reference.
    for doc in ["README.md", "CHEATSHEET.md", "docs/README.md", "docs/use-cases.md"] {
        let text = read(doc);
        for target in ["PROTOCOL.md", "CHEATSHEET.md", "README.md"] {
            if text.contains(&format!("]({target})")) || text.contains(&format!("](../{target})")) {
                assert!(
                    repo().join(target).exists(),
                    "{doc} links to {target}, which does not exist"
                );
            }
        }
        for line in text.lines().filter(|l| l.contains("docs/")) {
            for part in line.split("docs/").skip(1) {
                let candidate: String = part
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.' || *c == '/')
                    .collect();
                if candidate.ends_with(".md") {
                    let path = repo().join("docs").join(&candidate);
                    assert!(
                        Path::new(&path).exists(),
                        "{doc} mentions docs/{candidate}, which does not exist"
                    );
                }
            }
        }
    }
}
