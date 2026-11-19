//! Collects commits with `git rev-list`.
//!
//! Fields are separated with control characters that cannot appear in commit
//! metadata, which avoids the timestamp-delimiter trick and the guesswork the
//! previous implementation needed when parsing `--header` output.

use crate::git::Git;
use crate::tags::Tag;
use crate::{Error, ErrorCode, Result};

const FIELD: char = '\u{1}';
const RECORD: char = '\u{2}';

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub title: String,
    pub description: String,
    pub author: String,
}

#[derive(Debug, Clone, Default)]
pub struct Commits {
    pub all: Vec<Commit>,
    pub latest: Option<Commit>,
}

/// Loads the commits between the previous tag and the release tag. When either
/// side is missing the whole history is loaded, matching the previous
/// behaviour for a first release.
pub fn load(git: &Git, tags: &[Tag], include_latest: bool) -> Result<Commits> {
    let release = tags.first();
    let parent = tags.get(1);

    let range = match (release, parent) {
        (Some(release), Some(parent)) => {
            if release.hash.is_empty() || parent.hash.is_empty() {
                None
            } else {
                Some(format!("{}..{}", parent.hash, release.hash))
            }
        }
        _ => None,
    };

    let format = format!("--format=%H{FIELD}%s{FIELD}%ae{FIELD}%b{RECORD}");
    let mut all = match range {
        Some(range) => parse(&run(git, &[&format, &range])?),
        None => parse(&run(git, &[&format, "HEAD"])?),
    };

    let latest = release.and_then(|tag| all.iter().find(|commit| commit.hash == tag.hash).cloned());

    if !include_latest {
        if let Some(latest) = &latest {
            all.retain(|commit| commit.hash != latest.hash);
        }
    }

    Ok(Commits { all, latest })
}

fn run(git: &Git, args: &[&str]) -> Result<String> {
    // `git log --format=` prints only the format output. `rev-list --pretty`
    // would prepend a `commit <hash>` header to every record.
    let mut full: Vec<&str> = vec!["log"];
    full.extend_from_slice(args);

    let output = git.raw(&full)?;

    if !output.success {
        return Err(Error::new(ErrorCode::Other, "Not able to collect commits."));
    }

    Ok(output.stdout)
}

/// Parses the record stream produced by [`load`]. Also used by tests and the
/// TUI, which never re-reads git for already loaded commits.
pub fn parse(output: &str) -> Vec<Commit> {
    output
        .split(RECORD)
        .filter_map(|record| {
            let trimmed = record.trim();

            if trimmed.is_empty() {
                return None;
            }

            let mut fields = trimmed.split(FIELD);
            let hash = fields.next()?.trim().to_string();
            let title = fields.next().unwrap_or("").trim().to_string();
            let author = fields.next().unwrap_or("").trim().to_string();
            let description = fields.next().unwrap_or("").trim().to_string();

            if hash.is_empty() {
                return None;
            }

            Some(Commit {
                hash,
                title,
                description,
                author,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_records_with_multiline_bodies() {
        let output = "abc\u{1}Add feature\u{1}dev@example.com\u{1}Body line one\nBody line two\u{2}\ndef\u{1}Fix bug\u{1}other@example.com\u{1}\u{2}";
        let commits = parse(output);

        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].hash, "abc");
        assert_eq!(commits[0].title, "Add feature");
        assert_eq!(commits[0].author, "dev@example.com");
        assert_eq!(commits[0].description, "Body line one\nBody line two");
        assert_eq!(commits[1].title, "Fix bug");
        assert_eq!(commits[1].description, "");
    }

    #[test]
    fn skips_empty_records() {
        assert!(parse("\u{2}\n\u{2}").is_empty());
    }

    #[test]
    fn parses_real_git_output_without_a_commit_header() {
        let dir = std::env::temp_dir().join(format!(
            "release-commits-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let git = Git::new(&dir);
        git.run(&["init", "-q"]).unwrap();
        git.run(&["config", "user.email", "test@example.com"])
            .unwrap();
        git.run(&["config", "user.name", "Test"]).unwrap();

        std::fs::write(dir.join("a.txt"), "a").unwrap();
        git.add_all().unwrap();
        git.commit("Add a shiny feature (#12)").unwrap();

        std::fs::write(dir.join("b.txt"), "b").unwrap();
        git.add_all().unwrap();
        git.commit("Fix the crash (patch)").unwrap();

        let commits = load(&git, &[], false).unwrap();

        assert_eq!(commits.all.len(), 2);

        for commit in &commits.all {
            assert_eq!(commit.hash.len(), 40, "hash was {:?}", commit.hash);
            assert!(commit
                .hash
                .chars()
                .all(|character| character.is_ascii_hexdigit()));
            assert_eq!(commit.author, "test@example.com");
        }

        assert_eq!(commits.all[0].title, "Fix the crash (patch)");
        assert_eq!(commits.all[1].title, "Add a shiny feature (#12)");
    }
}
