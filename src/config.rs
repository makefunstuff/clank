//! Optional `.clank/config.toml` in the working directory.
//!
//! Discovery is that directory and nowhere else: a file in a parent is a
//! monorepo landmine, so it is not read. `--config PATH` names a file, and
//! `CLANK_CONFIG` names one when the flag is absent. Either of those is an
//! explicit path: missing or unreadable is a usage error. A missing
//! `./.clank/config.toml` is not an error.
//!
//! A file that exists and does not parse is a usage error for whichever binary
//! opened it.
//!
//! Precedence is the caller's job, and it is always flags, then the environment,
//! then this file, then built-ins. `[clank]` is the model and endpoint for
//! `clank` and `clank-jev`. `[web]` is search settings for `clank-web`.

use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const DIR_NAME: &str = ".clank";
pub const FILE_NAME: &str = "config.toml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub path: PathBuf,
    pub clank: Clank,
    pub web: Web,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Clank {
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub api_key: Option<String>,
    pub timeout: Option<u64>,
    pub max_tokens: Option<u32>,
    pub max_rounds: Option<u64>,
    pub system: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Web {
    pub default_provider: Option<String>,
    pub limit: Option<u32>,
    pub format: Option<String>,
    pub brave: ProviderKeys,
    pub tavily: ProviderKeys,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderKeys {
    pub api_key_env: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct Raw {
    clank: Clank,
    web: Web,
}

/// `./.clank/config.toml` inside `dir`, when that file exists. Parents are not searched.
pub fn implied_file(dir: &Path) -> Option<PathBuf> {
    let candidate = dir.join(DIR_NAME).join(FILE_NAME);
    candidate.is_file().then_some(candidate)
}

/// `flag` is `--config`. It wins over `CLANK_CONFIG`, which wins over the file
/// in the working directory.
pub fn load(flag: Option<&Path>) -> Result<Option<File>, String> {
    if let Some(path) = flag.filter(|p| !p.as_os_str().is_empty()) {
        return read_at(path).map(Some);
    }
    if let Some(path) = env_var("CLANK_CONFIG") {
        return read_at(Path::new(&path)).map(Some);
    }
    let start = std::env::current_dir().map_err(|e| format!("current directory: {e}"))?;
    match implied_file(&start) {
        Some(path) => read_at(&path).map(Some),
        None => Ok(None),
    }
}

fn read_at(path: &Path) -> Result<File, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let mut file = parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    file.path = path.to_path_buf();
    Ok(file)
}

pub fn parse(text: &str) -> Result<File, String> {
    let raw: Raw = toml::from_str(text).map_err(|e| e.to_string())?;
    validate(&raw)?;
    Ok(File { path: PathBuf::new(), clank: raw.clank, web: raw.web })
}

fn validate(raw: &Raw) -> Result<(), String> {
    if let Some(url) = raw.clank.base_url.as_deref() {
        nonblank("[clank].base_url", url)?;
        if !http_url(url.trim()) {
            return Err(format!("[clank].base_url {url:?} must be an http or https URL"));
        }
    }
    if let Some(model) = raw.clank.model.as_deref() {
        nonblank("[clank].model", model)?;
    }
    if let Some(name) = raw.clank.api_key_env.as_deref() {
        env_name("[clank].api_key_env", name)?;
    }
    if let Some(0) = raw.clank.timeout {
        return Err("[clank].timeout must be at least 1".into());
    }
    if let Some(0) = raw.clank.max_tokens {
        return Err("[clank].max_tokens must be at least 1".into());
    }
    if let Some(0) = raw.clank.max_rounds {
        return Err("[clank].max_rounds must be at least 1".into());
    }
    if let Some(name) = raw.web.default_provider.as_deref() {
        if !matches!(name, "brave" | "tavily") {
            return Err(format!("[web].default_provider {name:?} is unknown (brave, tavily)"));
        }
    }
    if let Some(n) = raw.web.limit {
        if !(1..=20).contains(&n) {
            return Err(format!("[web].limit {n} is outside 1..=20"));
        }
    }
    if let Some(fmt) = raw.web.format.as_deref() {
        if !matches!(fmt, "jsonl" | "text") {
            return Err(format!("[web].format {fmt:?} must be jsonl or text"));
        }
    }
    for (label, section) in [("[web.brave]", &raw.web.brave), ("[web.tavily]", &raw.web.tavily)] {
        if let Some(name) = section.api_key_env.as_deref() {
            env_name(&format!("{label}.api_key_env"), name)?;
        }
    }
    Ok(())
}

fn nonblank(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{label} is empty"))
    } else {
        Ok(())
    }
}

fn env_name(label: &str, name: &str) -> Result<(), String> {
    if valid_env_name(name) {
        Ok(())
    } else {
        Err(format!("{label} {name:?} is not an environment variable name"))
    }
}

pub fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    name.len() <= 128 && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn http_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")) else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty() && !host.contains(' ')
}

/// A set, non-empty environment variable, with surrounding whitespace removed.
pub fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// The secret for one section.
///
/// `named` is `api_key_env` and replaces `well_known` when the file sets it.
/// An inline `api_key` is used only when that variable is unset. `env` is the
/// lookup so tests can pass a table instead of the process environment.
pub fn key_from(
    named: Option<&str>,
    inline: Option<&str>,
    well_known: Option<&str>,
    env: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let var = named.or(well_known);
    if let Some(name) = var {
        if let Some(value) = env(name) {
            return Some(value);
        }
    }
    inline.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// A provider body that echoes the token must not reach stderr whole.
pub fn redact(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        text.to_string()
    } else {
        text.replace(secret, "***")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| (*v).trim().to_string()).filter(|v| !v.is_empty())
        }
    }

    #[test]
    fn the_sample_config_is_the_one_the_doc_shows() {
        let file = include_str!("../fixtures/clank.config.toml");
        let doc = include_str!("../docs/clank-web.md");
        let fence = doc.split("```toml\n").nth(1).expect("toml fence in docs/clank-web.md");
        let fence = fence.split("```").next().expect("fence end");
        assert_eq!(fence.trim_end(), file.trim_end(), "docs/clank-web.md and fixtures/clank.config.toml drifted");
        let cfg = parse(file).unwrap();
        assert_eq!(cfg.clank.model.as_deref(), Some("local"));
        assert_eq!(cfg.clank.base_url.as_deref(), Some("http://127.0.0.1:8080/v1"));
        assert_eq!(cfg.clank.api_key_env.as_deref(), Some("CLANK_API_KEY"));
        assert_eq!(cfg.clank.timeout, Some(600));
        assert_eq!(cfg.clank.max_tokens, Some(8192));
        assert_eq!(cfg.clank.max_rounds, Some(12));
        assert_eq!(cfg.web.default_provider.as_deref(), Some("brave"));
        assert_eq!(cfg.web.limit, Some(5));
        assert_eq!(cfg.web.format.as_deref(), Some("jsonl"));
        assert_eq!(cfg.web.brave.api_key_env.as_deref(), Some("BRAVE_API_KEY"));
        assert_eq!(cfg.web.tavily.api_key_env.as_deref(), Some("TAVILY_API_KEY"));
        assert!(!file.contains("40583"));
        assert!(!file.contains("qwen"));
    }

    #[test]
    fn a_bad_file_is_rejected_and_an_empty_one_is_valid() {
        assert!(parse("").is_ok());
        assert!(parse("[web.brave]\napi_key_env = \"BRAVE_API_KEY\"\n").is_ok());
        assert!(parse("[web.brave]\napi_key = \"inline\"\n").is_ok());
        assert!(parse("provider = \"brave\"\n").is_err(), "the old top-level provider key is not this schema");
        assert!(parse("[web]\ndefault_provider = \"google\"\n").is_err());
        assert!(parse("[web]\nlimit = 0\n").is_err());
        assert!(parse("[web]\nlimit = 21\n").is_err());
        assert!(parse("[web]\nformat = \"csv\"\n").is_err());
        assert!(parse("[clank]\ntimeout = 0\n").is_err());
        assert!(parse("[clank]\nbase_url = \"not a url\"\n").is_err());
        assert!(parse("[web.brave]\napi_key_env = \"not a name\"\n").is_err());
        let err = parse("api_key = \"secret\"\n").unwrap_err();
        assert!(err.contains("unknown field"), "{err}");
    }

    #[test]
    fn the_named_variable_replaces_the_well_known_one_and_beats_an_inline_key() {
        let env = env_of(&[("BRAVE_API_KEY", "well"), ("SEARCH_TOKEN", "named")]);
        assert_eq!(key_from(Some("SEARCH_TOKEN"), Some("inline"), Some("BRAVE_API_KEY"), &env).as_deref(), Some("named"));
        assert_eq!(key_from(None, Some("inline"), Some("BRAVE_API_KEY"), &env).as_deref(), Some("well"));
        let unset = env_of(&[]);
        assert_eq!(key_from(Some("SEARCH_TOKEN"), Some(" inline "), Some("BRAVE_API_KEY"), &unset).as_deref(), Some("inline"));
        assert_eq!(key_from(None, Some("  "), Some("BRAVE_API_KEY"), &unset), None);
        assert_eq!(redact("token secret leaked", "secret"), "token *** leaked");
        assert_eq!(redact("untouched", ""), "untouched");
    }

    #[test]
    fn the_implied_file_is_the_working_directory_only() {
        let root = std::env::temp_dir().join(format!("clank-cfg-discover-{}", std::process::id()));
        let parent = root.join("parent");
        let child = parent.join("child");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(parent.join(".clank")).unwrap();
        std::fs::create_dir_all(&child).unwrap();
        let parent_file = parent.join(".clank/config.toml");
        std::fs::write(&parent_file, "[clank]\nmodel = \"parent\"\n").unwrap();
        assert_eq!(implied_file(&child), None, "a parent file is not the working directory");
        assert_eq!(implied_file(&parent), Some(parent_file));
        let sample = include_str!("../fixtures/clank.config.toml");
        assert!(!sample.contains("api_key ="), "the sample names a variable, it does not carry a key");
        let _ = std::fs::remove_dir_all(&root);
    }
}
