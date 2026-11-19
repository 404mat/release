//! Repository-scoped configuration.
//!
//! `release.toml` at the repo root is the only committed configuration. Values
//! can be overridden per invocation with `RELEASE_*` environment variables, and
//! nothing credential-related is ever stored here: authentication comes from the
//! `gh` CLI.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::{Error, Result};

pub const CONFIG_FILE: &str = "release.toml";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    remote: Option<String>,
    hook: Option<String>,
    branch: Option<String>,
    prerelease_suffix: Option<String>,
    fetch_timeout: Option<f64>,
    editor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    /// The remote release talks to.
    pub remote: String,
    /// Hook executable, resolved against the repository root.
    pub hook: Option<PathBuf>,
    /// Branch configured as the release branch, if any.
    pub branch: Option<String>,
    /// Suffix used by `release pre` when none is given.
    pub prerelease_suffix: String,
    /// Timeout for the entry fetch and for reachability probes.
    pub fetch_timeout: Duration,
    /// Editor command used by `--edit` and the TUI.
    pub editor: String,
    /// Where the configuration was read from, when it existed.
    pub source: Option<PathBuf>,
}

impl Config {
    pub fn load(cwd: &Path) -> Result<Self> {
        let path = cwd.join(CONFIG_FILE);
        let mut config = Self::default();

        if path.exists() {
            let raw = std::fs::read_to_string(&path).map_err(|error| {
                Error::other(format!("Could not read {}: {error}", path.display()))
            })?;

            let parsed: FileConfig = toml::from_str(&raw).map_err(|error| {
                Error::new(
                    crate::ErrorCode::Usage,
                    format!("Could not parse {}: {error}", path.display()),
                )
            })?;

            config.apply_file(parsed);
            config.source = Some(path);
        }

        config.apply_env();
        config.validate()?;

        Ok(config)
    }

    fn apply_file(&mut self, file: FileConfig) {
        if let Some(remote) = file.remote {
            self.remote = remote;
        }

        if let Some(hook) = file.hook {
            self.hook = Some(PathBuf::from(hook));
        }

        if let Some(branch) = file.branch {
            self.branch = Some(branch);
        }

        if let Some(suffix) = file.prerelease_suffix {
            self.prerelease_suffix = suffix;
        }

        if let Some(timeout) = file.fetch_timeout {
            if timeout > 0.0 {
                self.fetch_timeout = Duration::from_secs_f64(timeout);
            }
        }

        if let Some(editor) = file.editor {
            self.editor = editor;
        }
    }

    fn apply_env(&mut self) {
        if let Some(remote) = env("RELEASE_REMOTE") {
            self.remote = remote;
        }

        if let Some(hook) = env("RELEASE_HOOK") {
            self.hook = Some(PathBuf::from(hook));
        }

        if let Some(branch) = env("RELEASE_BRANCH") {
            self.branch = Some(branch);
        }

        if let Some(suffix) = env("RELEASE_PRERELEASE_SUFFIX") {
            self.prerelease_suffix = suffix;
        }

        if let Some(timeout) =
            env("RELEASE_FETCH_TIMEOUT").and_then(|value| value.parse::<f64>().ok())
        {
            if timeout > 0.0 {
                self.fetch_timeout = Duration::from_secs_f64(timeout);
            }
        }

        if let Some(editor) = env("RELEASE_EDITOR") {
            self.editor = editor;
        }
    }

    fn validate(&self) -> Result<()> {
        if self.remote.trim().is_empty() {
            return Err(Error::usage("The configured remote cannot be empty."));
        }

        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            remote: "origin".to_string(),
            hook: None,
            branch: None,
            prerelease_suffix: "canary".to_string(),
            fetch_timeout: Duration::from_secs(3),
            editor: default_editor(),
            source: None,
        }
    }
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

pub fn default_editor() -> String {
    if let Some(editor) = env("RELEASE_EDITOR") {
        return editor;
    }

    if let Some(editor) = env("VISUAL") {
        return editor;
    }

    if let Some(editor) = env("EDITOR") {
        return editor;
    }

    "vi".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_previously_shipped_behaviour() {
        let config = Config::default();
        assert_eq!(config.remote, "origin");
        assert_eq!(config.prerelease_suffix, "canary");
        assert_eq!(config.fetch_timeout, Duration::from_secs(3));
    }

    #[test]
    fn parses_a_config_file() {
        let parsed: FileConfig = toml::from_str(
            r#"
            remote = "upstream"
            hook = "scripts/hook"
            prerelease_suffix = "beta"
            fetch_timeout = 5
            "#,
        )
        .unwrap();

        let mut config = Config::default();
        config.apply_file(parsed);

        assert_eq!(config.remote, "upstream");
        assert_eq!(config.prerelease_suffix, "beta");
        assert_eq!(config.fetch_timeout, Duration::from_secs(5));
        assert_eq!(config.hook, Some(PathBuf::from("scripts/hook")));
    }

    #[test]
    fn rejects_unknown_keys() {
        let parsed = toml::from_str::<FileConfig>("nope = 1");
        assert!(parsed.is_err());
    }
}
