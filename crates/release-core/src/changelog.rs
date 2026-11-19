//! Renders the release notes.

use std::collections::BTreeMap;

use crate::definitions::{self, reference_number};
use crate::group::GroupedChange;
use crate::ChangeType;

#[derive(Debug, Clone, Default)]
pub struct Rendered {
    pub changelog: Option<String>,
    pub credits: Vec<String>,
}

/// Renders grouped changes into markdown.
///
/// `authors` maps pull request numbers to GitHub logins; `release_author` is
/// excluded from the credits because they made the release, not a change in it.
pub fn render(
    grouped: &[(String, Vec<GroupedChange>)],
    change_types: &[ChangeType],
    skip_questions: bool,
    escape_html: bool,
    authors: &BTreeMap<u64, Option<String>>,
    release_author: Option<&str>,
) -> Rendered {
    let mut text = String::new();
    let mut credits: Vec<String> = Vec::new();

    for (handle, changes) in grouped {
        if changes.is_empty() {
            continue;
        }

        let mut list = String::new();
        let last = changes.len().saturating_sub(1);

        for (index, change) in changes.iter().enumerate() {
            let cleaned = clean_title(&change.message, change_types, escape_html);

            let target = match &cleaned.reference {
                Some(reference) => reference.clone(),
                None => change.hash.clone(),
            };

            list.push_str(&format!("- {}: {}\n", cleaned.content, target));

            if let Some(reference) = &cleaned.reference {
                if let Some(number) = reference_number(reference) {
                    if let Some(Some(login)) = authors.get(&number) {
                        if !login.contains("[bot]") {
                            push_unique(&mut credits, login);
                        }
                    }
                }
            }

            if index == last {
                list.push('\n');
            }
        }

        if !skip_questions {
            if let Some(change_type) = change_types.iter().find(|item| item.handle == handle) {
                text.push_str(&format!("### {} \n\n", change_type.plural_name));
            }
        }

        text.push_str(&list);
    }

    if let Some(release_author) = release_author {
        credits.retain(|credit| credit != release_author);
    }

    if !credits.is_empty() {
        text.push_str("### Credits \n\n");
        text.push_str("Huge thanks to ");

        let total = credits.len();

        for (index, credit) in credits.iter().enumerate() {
            text.push('@');
            text.push_str(credit);

            let position = index + 1;

            if total > 1 && position == total - 1 {
                if total > 2 {
                    text.push(',');
                }

                text.push_str(" and ");
            } else if position != total {
                text.push_str(", ");
            }
        }

        text.push_str(" for helping!");
        text.push('\n');
    }

    let trimmed = text.trim_end().to_string();

    Rendered {
        changelog: if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        },
        credits,
    }
}

#[derive(Debug, Clone)]
struct CleanedTitle {
    content: String,
    reference: Option<String>,
}

fn clean_title(title: &str, change_types: &[ChangeType], escape_html: bool) -> CleanedTitle {
    let declared_type = definitions::type_of(title, change_types);
    let reference = definitions::reference(title);

    let mut content = title.to_string();

    if let Some(declared_type) = &declared_type {
        content = content.replace(&format!("({declared_type})"), "");
    }

    if let Some(reference) = &reference {
        content = content.replace(&format!("({reference})"), "");
    }

    if escape_html {
        content = html_escape(&content);
    }

    CleanedTitle {
        content: capitalize(content.trim()),
        reference,
    }
}

fn capitalize(value: &str) -> String {
    let mut characters = value.chars();

    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

fn html_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());

    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }

    escaped
}

fn push_unique(list: &mut Vec<String>, value: &str) {
    if !list.iter().any(|item| item == value) {
        list.push(value.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::group::GroupedChange;

    fn change(hash: &str, message: &str) -> GroupedChange {
        GroupedChange {
            hash: hash.to_string(),
            message: message.to_string(),
        }
    }

    fn grouped(entries: &[(&str, Vec<GroupedChange>)]) -> Vec<(String, Vec<GroupedChange>)> {
        entries
            .iter()
            .map(|(handle, changes)| (handle.to_string(), changes.clone()))
            .collect()
    }

    #[test]
    fn renders_headings_and_strips_markers() {
        let rendered = render(
            &grouped(&[
                ("major", vec![change("abc123", "Rewrite API (major)")]),
                ("patch", vec![change("def456", "Fix crash (#12)")]),
            ]),
            &crate::change_types(),
            false,
            false,
            &BTreeMap::new(),
            None,
        );

        let changelog = rendered.changelog.unwrap();

        assert_eq!(
            changelog,
            "### Major Changes \n\n- Rewrite API: abc123\n\n### Patches \n\n- Fix crash: #12"
        );
    }

    #[test]
    fn omits_headings_when_questions_were_skipped() {
        let rendered = render(
            &grouped(&[("patch", vec![change("abc123", "Fix crash")])]),
            &crate::change_types(),
            true,
            false,
            &BTreeMap::new(),
            None,
        );

        assert_eq!(rendered.changelog.unwrap(), "- Fix crash: abc123");
    }

    #[test]
    fn builds_an_oxford_comma_credit_list() {
        let mut authors = BTreeMap::new();
        authors.insert(1, Some("one".to_string()));
        authors.insert(2, Some("two".to_string()));
        authors.insert(3, Some("three".to_string()));

        let rendered = render(
            &grouped(&[(
                "patch",
                vec![
                    change("a", "First (#1)"),
                    change("b", "Second (#2)"),
                    change("c", "Third (#3)"),
                ],
            )]),
            &crate::change_types(),
            true,
            false,
            &authors,
            None,
        );

        let changelog = rendered.changelog.unwrap();
        assert!(changelog.contains("Huge thanks to @one, @two, and @three for helping!"));
    }

    #[test]
    fn uses_and_between_two_credits() {
        let mut authors = BTreeMap::new();
        authors.insert(1, Some("one".to_string()));
        authors.insert(2, Some("two".to_string()));

        let rendered = render(
            &grouped(&[(
                "patch",
                vec![change("a", "First (#1)"), change("b", "Second (#2)")],
            )]),
            &crate::change_types(),
            true,
            false,
            &authors,
            None,
        );

        assert!(rendered
            .changelog
            .unwrap()
            .contains("Huge thanks to @one and @two for helping!"));
    }

    #[test]
    fn excludes_the_release_author_and_bots() {
        let mut authors = BTreeMap::new();
        authors.insert(1, Some("author".to_string()));
        authors.insert(2, Some("dependabot[bot]".to_string()));

        let rendered = render(
            &grouped(&[(
                "patch",
                vec![change("a", "First (#1)"), change("b", "Second (#2)")],
            )]),
            &crate::change_types(),
            true,
            false,
            &authors,
            Some("author"),
        );

        assert!(!rendered.changelog.unwrap().contains("Credits"));
        assert!(rendered.credits.is_empty());
    }

    #[test]
    fn escapes_html_unless_a_hook_is_used() {
        let escaped = render(
            &grouped(&[("patch", vec![change("a", "<script> & co")])]),
            &crate::change_types(),
            true,
            true,
            &BTreeMap::new(),
            None,
        );

        assert!(escaped
            .changelog
            .unwrap()
            .contains("&lt;script&gt; &amp; co"));

        let raw = render(
            &grouped(&[("patch", vec![change("a", "<script> & co")])]),
            &crate::change_types(),
            true,
            false,
            &BTreeMap::new(),
            None,
        );

        assert!(raw.changelog.unwrap().contains("<script> & co"));
    }

    #[test]
    fn returns_none_without_changes() {
        let rendered = render(
            &grouped(&[("patch", vec![])]),
            &crate::change_types(),
            false,
            false,
            &BTreeMap::new(),
            None,
        );

        assert!(rendered.changelog.is_none());
    }
}
