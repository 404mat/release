//! Authentication through the GitHub CLI.
//!
//! Release does not manage tokens, does not run an OAuth callback service and
//! does not store credentials. `gh` must be installed and authenticated before
//! release will touch the GitHub API.

use std::process::Command;
use std::time::Duration;

use crate::git::run_command;
use crate::{Error, ErrorCode, Result};

const GH_TIMEOUT: Duration = Duration::from_secs(10);

fn gh(args: &[&str]) -> Result<crate::git::CommandOutput> {
    let mut command = Command::new("gh");
    command.args(args);
    run_command(command, Some(GH_TIMEOUT))
}

/// Verifies that `gh` exists and holds a valid token.
pub fn ensure_authed() -> Result<()> {
    let output = match gh(&["auth", "status"]) {
        Ok(output) => output,
        Err(_) => {
            return Err(
                Error::new(ErrorCode::Auth, "The GitHub CLI (gh) is not available.")
                    .with_remediation(
                        "Install gh from https://cli.github.com and run `gh auth login`.",
                    ),
            );
        }
    };

    if output.success {
        return Ok(());
    }

    let detail = output.stderr.trim();
    let message = if detail.is_empty() {
        "The GitHub CLI is not authenticated.".to_string()
    } else {
        format!("The GitHub CLI is not authenticated: {detail}")
    };

    Err(Error::new(ErrorCode::Auth, message).with_remediation("Run `gh auth login` and try again."))
}

/// Reads the token that `gh` is already holding.
pub fn token() -> Result<String> {
    let output = gh(&["auth", "token"])?;

    if !output.success {
        return Err(
            Error::new(ErrorCode::Auth, "Could not read a token from gh.")
                .with_remediation("Run `gh auth login` and try again."),
        );
    }

    let token = output.stdout.trim().to_string();

    if token.is_empty() {
        return Err(Error::new(ErrorCode::Auth, "gh returned an empty token.")
            .with_remediation("Run `gh auth login` and try again."));
    }

    Ok(token)
}
