//! SemVer helpers.
//!
//! The `semver` crate parses and compares, but it deliberately has no
//! equivalent of npm's `inc`/`diff`. Release depends on npm's tolerances (a
//! leading `v` on tags, prerelease increments that keep an existing suffix), so
//! both are reimplemented here and covered by tests.

use semver::{BuildMetadata, Prerelease, Version};

use crate::{Error, Result};

/// Parses a version, tolerating the leading `v`/`V` that release tags carry and
/// surrounding whitespace. Mirrors `semver.valid()`, which also normalises.
pub fn parse_lenient(input: &str) -> Option<Version> {
    let trimmed = input.trim();
    let stripped = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);

    Version::parse(stripped).ok()
}

/// Splits a tag into its prefix and version. `v1.2.3` -> `("v", 1.2.3)`.
pub fn split_tag(tag: &str) -> Option<(String, Version)> {
    let trimmed = tag.trim();
    let version = parse_lenient(trimmed)?;
    let len = trimmed.len();
    let version_len = version.to_string().len();

    if version_len > len {
        return None;
    }

    Some((trimmed[..len - version_len].to_string(), version))
}

/// The tag prefix used by a versioned tag (`v` in `v1.2.3`).
pub fn tag_prefix(tag: &str) -> String {
    split_tag(tag).map(|(prefix, _)| prefix).unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Major,
    Minor,
    Patch,
    Prerelease,
}

/// Semantic difference between two versions, matching `semver.diff()` for the
/// cases release cares about.
pub fn diff(from: &Version, to: &Version) -> Option<DiffKind> {
    if from == to {
        return None;
    }

    if from.major != to.major {
        return Some(DiffKind::Major);
    }

    if from.minor != to.minor {
        return Some(DiffKind::Minor);
    }

    if from.patch != to.patch {
        return Some(DiffKind::Patch);
    }

    Some(DiffKind::Prerelease)
}

fn with_prerelease(mut version: Version, identifiers: &[String]) -> Version {
    let joined = identifiers.join(".");
    version.pre = Prerelease::new(&joined).unwrap_or(Prerelease::EMPTY);
    version.build = BuildMetadata::EMPTY;
    version
}

fn is_numeric(identifier: &str) -> bool {
    !identifier.is_empty() && identifier.chars().all(|c| c.is_ascii_digit())
}

fn increment_prerelease(version: &Version, suffix: Option<&str>) -> Option<Version> {
    let existing: Vec<String> = if version.pre.is_empty() {
        Vec::new()
    } else {
        version
            .pre
            .as_str()
            .split('.')
            .map(|part| part.to_string())
            .collect()
    };

    match suffix {
        // `inc(version, "pre")`: keep the existing identifiers and bump the
        // trailing numeric one.
        None => {
            let mut identifiers = existing;

            if let Some(position) = identifiers.iter().rposition(|part| is_numeric(part)) {
                let current: u64 = identifiers[position].parse().unwrap_or(0);
                identifiers[position] = (current + 1).to_string();
                identifiers.truncate(position + 1);
            } else {
                identifiers.push("0".to_string());
            }

            Some(with_prerelease(
                Version::new(version.major, version.minor, version.patch),
                &identifiers,
            ))
        }
        // `inc(version, "pre", suffix)`: keep counting when the suffix matches,
        // otherwise restart at `.0`.
        Some(suffix) => {
            if existing.first().map(String::as_str) == Some(suffix) {
                let mut identifiers = existing;

                if let Some(position) = identifiers.iter().rposition(|part| is_numeric(part)) {
                    let current: u64 = identifiers[position].parse().unwrap_or(0);
                    identifiers[position] = (current + 1).to_string();
                    identifiers.truncate(position + 1);
                } else {
                    identifiers.push("0".to_string());
                }

                return Some(with_prerelease(
                    Version::new(version.major, version.minor, version.patch),
                    &identifiers,
                ));
            }

            Some(with_prerelease(
                Version::new(version.major, version.minor, version.patch),
                &[suffix.to_string(), "0".to_string()],
            ))
        }
    }
}

/// npm-compatible version increment.
pub fn inc(version: &Version, kind: &str, suffix: Option<&str>) -> Result<Version> {
    let has_pre = !version.pre.is_empty();

    let next = match kind {
        "major" => {
            if has_pre && version.minor == 0 && version.patch == 0 {
                Version::new(version.major, 0, 0)
            } else {
                Version::new(version.major + 1, 0, 0)
            }
        }
        "minor" => {
            if has_pre && version.patch == 0 {
                Version::new(version.major, version.minor, 0)
            } else {
                Version::new(version.major, version.minor + 1, 0)
            }
        }
        "patch" => {
            if has_pre {
                Version::new(version.major, version.minor, version.patch)
            } else {
                Version::new(version.major, version.minor, version.patch + 1)
            }
        }
        "pre" => {
            if has_pre {
                increment_prerelease(version, suffix)
                    .ok_or_else(|| Error::other("Could not increment prerelease version"))?
            } else {
                let base = Version::new(version.major, version.minor, version.patch + 1);
                let identifiers = match suffix {
                    Some(suffix) => vec![suffix.to_string(), "0".to_string()],
                    None => vec!["0".to_string()],
                };

                with_prerelease(base, &identifiers)
            }
        }
        other => {
            return Err(Error::other(format!("Unknown version type \"{other}\"")));
        }
    };

    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(value: &str) -> Version {
        Version::parse(value).unwrap()
    }

    #[test]
    fn parses_tags_with_and_without_prefix() {
        assert_eq!(split_tag("v1.2.3").unwrap().0, "v");
        assert_eq!(split_tag("1.2.3").unwrap().0, "");
        // Like npm's `semver.valid`, only a leading `v` is tolerated.
        assert!(split_tag("app-v1.2.3").is_none());
        assert!(split_tag("not-a-version").is_none());
    }

    #[test]
    fn increments_plain_versions() {
        assert_eq!(
            inc(&version("1.2.3"), "major", None).unwrap().to_string(),
            "2.0.0"
        );
        assert_eq!(
            inc(&version("1.2.3"), "minor", None).unwrap().to_string(),
            "1.3.0"
        );
        assert_eq!(
            inc(&version("1.2.3"), "patch", None).unwrap().to_string(),
            "1.2.4"
        );
    }

    #[test]
    fn increments_prereleases_like_npm() {
        assert_eq!(
            inc(&version("1.2.3"), "pre", Some("canary"))
                .unwrap()
                .to_string(),
            "1.2.4-canary.0"
        );
        assert_eq!(
            inc(&version("1.2.4-canary.0"), "pre", Some("canary"))
                .unwrap()
                .to_string(),
            "1.2.4-canary.1"
        );
        assert_eq!(
            inc(&version("1.2.4-canary.1"), "pre", Some("beta"))
                .unwrap()
                .to_string(),
            "1.2.4-beta.0"
        );
        assert_eq!(
            inc(&version("1.2.4-beta.1"), "pre", None)
                .unwrap()
                .to_string(),
            "1.2.4-beta.2"
        );
        assert_eq!(
            inc(&version("1.2.3"), "pre", None).unwrap().to_string(),
            "1.2.4-0"
        );
    }

    #[test]
    fn promotes_prereleases_without_bumping_further() {
        assert_eq!(
            inc(&version("2.0.0-canary.3"), "major", None)
                .unwrap()
                .to_string(),
            "2.0.0"
        );
        assert_eq!(
            inc(&version("1.3.0-canary.3"), "minor", None)
                .unwrap()
                .to_string(),
            "1.3.0"
        );
        assert_eq!(
            inc(&version("1.2.4-canary.3"), "patch", None)
                .unwrap()
                .to_string(),
            "1.2.4"
        );
    }

    #[test]
    fn classifies_differences() {
        assert_eq!(
            diff(&version("1.0.0"), &version("2.0.0")),
            Some(DiffKind::Major)
        );
        assert_eq!(
            diff(&version("1.0.0"), &version("1.1.0")),
            Some(DiffKind::Minor)
        );
        assert_eq!(
            diff(&version("1.0.0"), &version("1.0.1")),
            Some(DiffKind::Patch)
        );
        assert_eq!(
            diff(&version("1.0.0-canary.1"), &version("1.0.0-canary.2")),
            Some(DiffKind::Prerelease)
        );
        assert_eq!(diff(&version("1.0.0"), &version("1.0.0")), None);
    }

    #[test]
    fn rejects_unknown_bump_types() {
        assert!(inc(&version("1.0.0"), "banana", None).is_err());
    }
}
