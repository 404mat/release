//! Progress reporting contract.
//!
//! Core code never writes to a terminal directly. Frontends implement this
//! trait: the CLI renders a spinner on stderr, the TUI forwards events into its
//! message loop, and tests use [`Silent`].

pub trait Progress: Send + Sync {
    fn start(&self, message: &str);
    fn update(&self, message: &str);
    fn succeed(&self, message: Option<&str>);
    fn warn(&self, message: &str);

    /// Called before an interaction that needs the real terminal (an editor, or
    /// a git command that may prompt for credentials). The TUI leaves its
    /// alternate screen and restores cooked mode; the CLI does nothing.
    fn set_suspended(&self, suspended: bool) {
        let _ = suspended;
    }

    /// Whether this frontend can hand the terminal back to a child process.
    fn claims_terminal(&self) -> bool {
        false
    }
}

/// Discards every progress event.
pub struct Silent;

impl Progress for Silent {
    fn start(&self, _message: &str) {}
    fn update(&self, _message: &str) {}
    fn succeed(&self, _message: Option<&str>) {}
    fn warn(&self, _message: &str) {}
}
