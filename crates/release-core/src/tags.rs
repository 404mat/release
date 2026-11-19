//! Reads release tags from `git log` decorations.

use semver::Version;

use crate::git::Git;
use crate::{semver_ext, Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub tag: String,
    pub version: Version,
    pub hash: String,
}

impl Tag {
    pub fn prefix(&self) -> String {
        semver_ext::tag_prefix(&self.tag)
    }
}

const FORMAT: &str = "--pretty=%d;%H;%ci";

/// Every semver tag in the repository, newest version first.
pub fn all(git: &Git) -> Result<Vec<Tag>> {
    let output = git.raw(&["log", "--no-walk", "--tags", FORMAT, "--decorate=short"])?;

    if !output.success {
        return Err(Error::new(
            crate::ErrorCode::NotARepo,
            "Directory is not a Git repository.",
        ));
    }

    Ok(parse_output(&output.stdout))
}

/// The release tags for the branch being released: `[latest, previous]`, or
/// `[latest]` when there is no earlier release.
pub fn list(git: &Git, previous_tag: Option<&str>, rev: &str) -> Result<Vec<Tag>> {
    let output = git.raw(&[
        "log",
        "--simplify-by-decoration",
        FORMAT,
        "--decorate=short",
        "--first-parent",
        rev,
    ])?;

    if !output.success {
        return Err(Error::new(
            crate::ErrorCode::NotARepo,
            "Directory is not a Git repository.",
        ));
    }

    let tags = parse_output(&output.stdout);
    let Some(latest) = tags.first().cloned() else {
        return Ok(Vec::new());
    };

    for candidate in &tags {
        let matches = match previous_tag {
            Some(requested) => candidate.version.to_string() == requested,
            None => candidate.version < latest.version,
        };

        if matches {
            return Ok(vec![latest, candidate.clone()]);
        }
    }

    Ok(vec![latest])
}

/// Parses `%d;%H;%ci` lines. A decoration looks like
/// ` (HEAD -> main, tag: v1.2.3, origin/main)`.
pub fn parse_output(output: &str) -> Vec<Tag> {
    let mut tags: Vec<Tag> = Vec::new();

    for line in output.lines() {
        let mut parts = line.split(';');
        let refs = match parts.next() {
            Some(value) => value,
            None => continue,
        };
        let hash = match parts.next() {
            Some(value) => value.trim(),
            None => continue,
        };
        let date = parts.next();

        if date.is_none() || hash.is_empty() {
            continue;
        }

        for name in decoration_tags(refs) {
            if let Some(version) = semver_ext::parse_lenient(&name) {
                tags.push(Tag {
                    tag: name,
                    version,
                    hash: hash.to_string(),
                });
            }
        }
    }

    tags.sort_by(|a, b| b.version.cmp(&a.version));
    tags
}

fn decoration_tags(decoration: &str) -> Vec<String> {
    decoration
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .split(", ")
        .filter_map(|part| part.trim().strip_prefix("tag: ").map(str::to_string))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_decorated_tags() {
        let output = " (HEAD -> main, tag: v1.2.3, origin/main);abc123;Thu Nov 18 12:00:00 2026 +0100\n (tag: v1.2.2);def456;Wed Nov 17 12:00:00 2026 +0100\n";
        let tags = parse_output(output);

        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].tag, "v1.2.3");
        assert_eq!(tags[0].hash, "abc123");
        assert_eq!(tags[0].version.to_string(), "1.2.3");
        assert_eq!(tags[1].tag, "v1.2.2");
    }

    #[test]
    fn sorts_versions_descending_regardless_of_prefix() {
        let output = " (tag: 1.10.0);a;x\n (tag: v1.9.9);b;y\n (tag: v2.0.0-rc.1);c;z\n";
        let tags = parse_output(output);

        assert_eq!(
            tags.iter()
                .map(|tag| tag.version.to_string())
                .collect::<Vec<_>>(),
            vec!["2.0.0-rc.1", "1.10.0", "1.9.9"]
        );
    }

    #[test]
    fn ignores_non_semver_tags() {
        let output = " (tag: latest, tag: v0.1.0);a;x\n";
        let tags = parse_output(output);

        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].tag, "v0.1.0");
    }

    #[test]
    fn ignores_lines_without_hash_or_date() {
        let output = "nonsense\n (tag: v1.0.0);;\n";
        assert!(parse_output(output).is_empty());
    }
}
