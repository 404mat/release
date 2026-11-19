use std::fmt;

/// Stable, machine-readable error categories. These map to process exit codes
/// so that callers (humans and agents alike) can branch on the outcome without
/// parsing prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    Usage,
    NotARepo,
    Auth,
    ReleaseExists,
    BranchNotSynced,
    NoChanges,
    VersionMismatch,
    WriteFailed,
    Other,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::NotARepo => "not_a_repo",
            Self::Auth => "auth_required",
            Self::ReleaseExists => "release_exists",
            Self::BranchNotSynced => "branch_not_synced",
            Self::NoChanges => "no_changes",
            Self::VersionMismatch => "version_mismatch",
            Self::WriteFailed => "write_failed",
            Self::Other => "error",
        }
    }

    pub fn exit_code(self) -> i32 {
        match self {
            Self::Usage => 2,
            Self::NotARepo => 3,
            Self::Auth => 4,
            Self::ReleaseExists => 5,
            Self::BranchNotSynced => 6,
            Self::NoChanges => 7,
            Self::VersionMismatch => 8,
            Self::WriteFailed => 9,
            Self::Other => 1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
    pub remediation: Option<String>,
    pub url: Option<String>,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            remediation: None,
            url: None,
        }
    }

    pub fn other(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Other, message)
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Usage, message)
    }

    pub fn with_remediation(mut self, remediation: impl Into<String>) -> Self {
        self.remediation = Some(remediation.into());
        self
    }

    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    pub fn with_code(mut self, code: ErrorCode) -> Self {
        self.code = code;
        self
    }

    /// Single-line form used in warnings and logs.
    pub fn short(&self) -> String {
        self.message.replace('\n', " ")
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::other(value.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::other(value.to_string())
    }
}

impl From<semver::Error> for Error {
    fn from(value: semver::Error) -> Self {
        Self::other(value.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
