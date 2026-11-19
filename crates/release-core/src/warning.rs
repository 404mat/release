use serde_json::{json, Value};

/// A non-fatal condition worth telling the user about: a failed fetch while
/// offline, commits that had to be defaulted, GitHub lookups that were skipped.
/// Warnings never abort a run and are always surfaced in `--json` output.
#[derive(Debug, Clone)]
pub struct Warning {
    pub code: String,
    pub message: String,
}

impl Warning {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({ "code": &self.code, "message": &self.message })
    }
}
