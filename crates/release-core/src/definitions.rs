//! Recognises the `(patch)` / `(#123)` markers in commit text.

use crate::ChangeType;

/// The change type pre-declared in a commit title or description, if any.
pub fn type_of(text: &str, change_types: &[ChangeType]) -> Option<String> {
    for change_type in change_types {
        if text.contains(&change_type.short()) {
            return Some(change_type.handle.to_string());
        }
    }

    if text.contains("(ignore)") {
        return Some("ignore".to_string());
    }

    None
}

/// The pull request reference in a commit title (`(#123)`), if any.
pub fn reference(title: &str) -> Option<String> {
    let bytes = title.as_bytes();

    for index in 0..bytes.len().saturating_sub(2) {
        if bytes[index] != b' ' || bytes[index + 1] != b'(' || bytes[index + 2] != b'#' {
            continue;
        }

        let mut cursor = index + 3;
        let mut has_digits = false;

        while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
            has_digits = true;
            cursor += 1;
        }

        if has_digits && cursor < bytes.len() && bytes[cursor] == b')' {
            return Some(title[index + 2..cursor].to_string());
        }
    }

    None
}

/// Extracts the numeric component of a reference (`#123` -> 123).
pub fn reference_number(reference: &str) -> Option<u64> {
    reference
        .rsplit('#')
        .next()
        .and_then(|value| value.trim_end_matches(')').parse::<u64>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change_types;

    #[test]
    fn finds_declared_types() {
        let types = change_types();
        assert_eq!(
            type_of("Error logging works now (patch)", &types).as_deref(),
            Some("patch")
        );
        assert_eq!(
            type_of("Big rewrite (major)", &types).as_deref(),
            Some("major")
        );
        assert_eq!(
            type_of("This is a commit message (ignore)", &types).as_deref(),
            Some("ignore")
        );
        assert_eq!(type_of("Nothing declared here", &types), None);
    }

    #[test]
    fn finds_references() {
        assert_eq!(reference("Fix things (#42)").as_deref(), Some("#42"));
        assert_eq!(reference("Fix things (#abc)"), None);
        assert_eq!(reference("Fix things (issue 42)"), None);
        assert_eq!(reference("No reference"), None);
    }

    #[test]
    fn extracts_reference_numbers() {
        assert_eq!(reference_number("#42"), Some(42));
        assert_eq!(reference_number("#-3"), None);
    }
}
