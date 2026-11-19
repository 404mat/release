//! Core logic shared by the `release` CLI and TUI frontends.

pub mod apply;
pub mod auth;
pub mod bump;
pub mod changelog;
pub mod choices;
pub mod commits;
pub mod config;
pub mod context;
pub mod definitions;
pub mod error;
pub mod git;
pub mod github;
pub mod group;
pub mod hook;
pub mod open;
pub mod plan;
pub mod progress;
pub mod semver_ext;
pub mod tags;
pub mod update;
pub mod warning;

pub use context::{Context, Forge};
pub use error::{Error, ErrorCode, Result};
pub use progress::{Progress, Silent};
pub use warning::Warning;

/// A semantic change type a commit can be classified as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeType {
    pub handle: &'static str,
    pub name: &'static str,
    pub plural_name: &'static str,
    pub description: &'static str,
}

impl ChangeType {
    pub fn label(&self) -> String {
        format!("{} ({})", self.name, self.description)
    }

    pub fn short(&self) -> String {
        format!("({})", self.handle)
    }
}

/// The change types offered for classification, in semantic order.
pub fn change_types() -> Vec<ChangeType> {
    vec![
        ChangeType {
            handle: "major",
            name: "Major Change",
            plural_name: "Major Changes",
            description: "incompatible API change",
        },
        ChangeType {
            handle: "minor",
            name: "Minor Change",
            plural_name: "Minor Changes",
            description: "backwards-compatible functionality",
        },
        ChangeType {
            handle: "patch",
            name: "Patch",
            plural_name: "Patches",
            description: "backwards-compatible bug fix",
        },
    ]
}

/// The semver bump types accepted as a positional argument.
pub const BUMP_TYPES: [&str; 4] = ["major", "minor", "patch", "pre"];

pub fn is_bump_type(value: &str) -> bool {
    BUMP_TYPES.contains(&value)
}
