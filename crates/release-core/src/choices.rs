//! Which change types are worth offering, given the previous release.

use crate::semver_ext::{self, DiffKind};
use crate::tags::Tag;
use crate::ChangeType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub name: String,
    pub value: String,
    pub short: String,
}

/// A release can only be as large as the gap since the last one, so a patch
/// release does not offer "major". Mirrors the previous behaviour exactly.
pub fn choices(change_types: &[ChangeType], tags: &[Tag]) -> Vec<Choice> {
    let mut offered: Vec<ChangeType> = change_types.to_vec();

    let mut not_needed = 0;

    if tags.len() >= 2 {
        not_needed = match semver_ext::diff(&tags[1].version, &tags[0].version) {
            Some(DiffKind::Minor) => 1,
            Some(DiffKind::Patch) => 2,
            _ => 0,
        };
    }

    if not_needed > 0 {
        let removed = not_needed.min(offered.len());
        offered.drain(0..removed);
    }

    let mut list: Vec<Choice> = offered
        .iter()
        .map(|change_type| Choice {
            name: change_type.label(),
            value: change_type.handle.to_string(),
            short: change_type.short(),
        })
        .collect();

    list.push(Choice {
        name: "Ignore".to_string(),
        value: "ignore".to_string(),
        short: "(ignored)".to_string(),
    });

    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semver_ext::parse_lenient;

    fn tag(value: &str) -> Tag {
        Tag {
            tag: format!("v{value}"),
            version: parse_lenient(value).unwrap(),
            hash: "abc".to_string(),
        }
    }

    fn values(choices: &[Choice]) -> Vec<String> {
        choices.iter().map(|choice| choice.value.clone()).collect()
    }

    #[test]
    fn offers_every_change_type_for_an_initial_release() {
        let choices = choices(&crate::change_types(), &[tag("0.0.1")]);

        assert_eq!(values(&choices), vec!["major", "minor", "patch", "ignore"]);
    }

    #[test]
    fn offers_only_patch_and_ignore_for_a_patch_release() {
        let choices = choices(&crate::change_types(), &[tag("0.0.2"), tag("0.0.1")]);

        assert_eq!(values(&choices), vec!["patch", "ignore"]);
    }

    #[test]
    fn offers_only_minor_and_below_after_a_minor_release() {
        let choices = choices(&crate::change_types(), &[tag("0.1.0"), tag("0.0.2")]);

        assert_eq!(values(&choices), vec!["minor", "patch", "ignore"]);
    }

    #[test]
    fn offers_everything_after_a_major_release() {
        let choices = choices(&crate::change_types(), &[tag("1.0.0"), tag("0.9.0")]);

        assert_eq!(values(&choices), vec!["major", "minor", "patch", "ignore"]);
    }

    #[test]
    fn offers_everything_between_prereleases() {
        let choices = choices(
            &crate::change_types(),
            &[tag("1.0.0-canary.2"), tag("1.0.0-canary.1")],
        );

        assert_eq!(values(&choices), vec!["major", "minor", "patch", "ignore"]);
    }
}
