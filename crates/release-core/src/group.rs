//! Groups classified commits by change type, preserving semantic order.

use crate::ChangeType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupedChange {
    pub hash: String,
    pub message: String,
}

/// A classification decision for one commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub change_type: String,
    pub message: String,
}

/// Groups `(hash, type, message)` triples. Ignored commits are dropped and the
/// remaining groups follow the order of `change_types`.
pub fn group(
    changes: &[(String, String, String)],
    change_types: &[ChangeType],
) -> Vec<(String, Vec<GroupedChange>)> {
    let mut grouped: Vec<(String, Vec<GroupedChange>)> = change_types
        .iter()
        .map(|change_type| (change_type.handle.to_string(), Vec::new()))
        .collect();

    for (hash, change_type, message) in changes {
        if change_type == "ignore" {
            continue;
        }

        if let Some((_, list)) = grouped.iter_mut().find(|(handle, _)| handle == change_type) {
            list.push(GroupedChange {
                hash: hash.clone(),
                message: message.clone(),
            });
        }
    }

    grouped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change_types;

    #[test]
    fn groups_in_semantic_order_and_drops_ignored() {
        let changes = vec![
            ("a".to_string(), "patch".to_string(), "A fix".to_string()),
            (
                "b".to_string(),
                "major".to_string(),
                "A rewrite".to_string(),
            ),
            ("c".to_string(), "ignore".to_string(), "Noise".to_string()),
            ("d".to_string(), "major".to_string(), "Another".to_string()),
        ];

        let grouped = group(&changes, &change_types());

        assert_eq!(grouped[0].0, "major");
        assert_eq!(grouped[0].1.len(), 2);
        assert_eq!(grouped[1].0, "minor");
        assert!(grouped[1].1.is_empty());
        assert_eq!(grouped[2].0, "patch");
        assert_eq!(grouped[2].1.len(), 1);
    }
}
