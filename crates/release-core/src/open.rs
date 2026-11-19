//! Opening the release in a browser. macOS and Linux only.

use std::process::{Command, Stdio};

#[cfg(target_os = "macos")]
const OPENER: &str = "open";

#[cfg(not(target_os = "macos"))]
const OPENER: &str = "xdg-open";

/// Fire-and-forget browser launch. Returns whether the opener could be started.
pub fn open_url(url: &str) -> bool {
    Command::new(OPENER)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}
