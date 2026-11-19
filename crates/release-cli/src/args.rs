//! Command line surface.
//!
//! The flag set is the one shipped by v6, preserved name for name. Flags added
//! since then are purely additive: they open up behaviour the TUI needs (labelled
//! commit classification, branch targeting, JSON output) without changing what
//! any existing flag means.

use std::path::PathBuf;

use clap::Parser;
use release_core::plan::Options;

#[derive(Debug, Parser)]
#[command(
    name = "release",
    version,
    about = "Generate changelogs with a single command",
    long_about = None,
    disable_help_subcommand = true,
    override_usage = "release [OPTIONS] [TYPE] [SUFFIX]"
)]
pub struct Cli {
    /// Mark the release as prerelease
    #[arg(long)]
    pub pre: bool,

    /// If the release already exists, replace it
    #[arg(long)]
    pub overwrite: bool,

    /// Instead of creating a draft, publish the release
    #[arg(long)]
    pub publish: bool,

    /// Specify a custom file to pipe releases through
    #[arg(short = 'H', long = "hook", value_name = "FILE")]
    pub hook: Option<String>,

    /// Specify previous release
    #[arg(
        short = 't',
        long = "previous-tag",
        value_name = "TAG",
        default_value = "",
        hide_default_value = true
    )]
    pub previous_tag: String,

    /// Specify a short update summary
    #[arg(
        short = 'S',
        long = "summary",
        value_name = "TEXT",
        default_value = "",
        hide_default_value = true
    )]
    pub summary: String,

    /// Create an annotated Git tag instead of a GitHub Release
    #[arg(long = "tag-only")]
    pub tag_only: bool,

    /// Print the release without writing, tagging, pushing, or uploading
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Show the release URL instead of opening it in the browser
    #[arg(short = 'u', long = "show-url")]
    pub show_url: bool,

    /// Skip the questions and create a simple list without the headings
    #[arg(short = 's', long = "skip-questions")]
    pub skip_questions: bool,

    /// Classify a commit, e.g. --set 1a2b3c4=minor (repeatable)
    #[arg(long = "set", value_name = "HASH=TYPE")]
    pub set: Vec<String>,

    /// Release a branch other than the checked-out one
    #[arg(long, value_name = "BRANCH")]
    pub branch: Option<String>,

    /// Do not touch the network and refuse writes (use with --dry-run)
    #[arg(long)]
    pub offline: bool,

    /// Skip the fetch performed on entry
    #[arg(long = "no-fetch")]
    pub no_fetch: bool,

    /// Machine-readable JSON on stdout, warnings on stderr
    #[arg(long)]
    pub json: bool,

    /// The kind of release: major, minor, patch or pre
    #[arg(value_name = "TYPE")]
    pub release_type: Option<String>,

    /// Prerelease suffix, e.g. beta
    #[arg(value_name = "SUFFIX")]
    pub suffix: Option<String>,
}

impl Cli {
    pub fn to_options(&self) -> std::result::Result<Options, String> {
        Ok(Options {
            release_type: self.release_type.clone(),
            suffix: self.suffix.clone(),
            pre: self.pre || self.release_type.as_deref() == Some("pre"),
            overwrite: self.overwrite,
            publish: self.publish,
            hook: self.hook.as_ref().map(PathBuf::from),
            previous_tag: empty_to_none(&self.previous_tag),
            summary: empty_to_none(&self.summary),
            tag_only: self.tag_only,
            dry_run: self.dry_run,
            show_url: self.show_url,
            skip_questions: self.skip_questions,
            branch: self.branch.clone(),
            offline: self.offline,
            no_fetch: self.no_fetch,
            assignments: parse_assignments(&self.set)?,
        })
    }

    /// Whether the invocation should open the TUI: an interactive terminal and a
    /// completely bare invocation. Any flag that shapes a release keeps the run
    /// on the CLI path, so an agent in a PTY can never be trapped in the TUI.
    pub fn prefers_tui(&self) -> bool {
        use std::io::IsTerminal;

        if std::env::var("RELEASE_NO_TUI")
            .map(|value| value == "1")
            .unwrap_or(false)
        {
            return false;
        }

        let shaping = self.json
            || self.release_type.is_some()
            || self.suffix.is_some()
            || !self.set.is_empty()
            || !self.summary.trim().is_empty()
            || !self.previous_tag.trim().is_empty()
            || self.hook.is_some()
            || self.branch.is_some()
            || self.dry_run
            || self.publish
            || self.overwrite
            || self.tag_only
            || self.offline
            || self.no_fetch
            || self.show_url
            || self.skip_questions
            || self.pre;

        if shaping {
            return false;
        }

        std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
    }
}

fn empty_to_none(value: &str) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// Parses `--set hash=type` values.
pub fn parse_assignments(values: &[String]) -> Result<Vec<(String, String)>, String> {
    let mut assignments = Vec::new();

    for value in values {
        let Some((hash, change_type)) = value.split_once('=') else {
            return Err(format!(
                "Could not parse --set \"{value}\". Expected <hash>=<type>."
            ));
        };

        let hash = hash.trim().to_string();
        let change_type = change_type.trim().to_string();

        if hash.is_empty() {
            return Err(format!("--set \"{value}\" is missing a commit hash."));
        }

        if !matches!(change_type.as_str(), "major" | "minor" | "patch" | "ignore") {
            return Err(format!(
                "--set \"{value}\" uses an unknown change type. Use major, minor, patch or ignore."
            ));
        }

        assignments.push((hash, change_type));
    }

    Ok(assignments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_assignment_flag() {
        let parsed = parse_assignments(&["abc123=minor".to_string()]).unwrap();
        assert_eq!(parsed, vec![("abc123".to_string(), "minor".to_string())]);
    }

    #[test]
    fn rejects_malformed_assignments() {
        assert!(parse_assignments(&["abc123".to_string()]).is_err());
        assert!(parse_assignments(&["=minor".to_string()]).is_err());
        assert!(parse_assignments(&["abc123=banana".to_string()]).is_err());
    }

    #[test]
    fn maps_flags_to_options() {
        let cli = Cli::parse_from(["release", "pre", "beta", "--tag-only", "--summary", "short"]);

        let options = cli.to_options().unwrap();

        assert_eq!(options.release_type.as_deref(), Some("pre"));
        assert_eq!(options.suffix.as_deref(), Some("beta"));
        assert!(options.pre);
        assert!(options.tag_only);
        assert_eq!(options.summary.as_deref(), Some("short"));
    }

    #[test]
    fn keeps_the_previous_tag_flag_name() {
        let cli = Cli::parse_from(["release", "-t", "v1.0.0", "-s", "-u"]);
        let options = cli.to_options().unwrap();

        assert_eq!(options.previous_tag.as_deref(), Some("v1.0.0"));
        assert!(options.skip_questions);
        assert!(options.show_url);
    }
}
