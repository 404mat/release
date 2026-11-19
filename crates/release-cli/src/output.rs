//! Rendering of plans, outcomes and errors.

use std::io::IsTerminal;

use release_core::apply::Outcome;
use release_core::plan::Plan;
use release_core::{open, Error, Warning};

#[derive(Debug, Clone, Copy)]
pub struct Style {
    color: bool,
}

impl Style {
    pub fn detect(json: bool) -> Self {
        let color =
            !json && std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal();

        Self { color }
    }

    fn wrap(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    pub fn bold(&self, text: &str) -> String {
        self.wrap("1", text)
    }

    pub fn red(&self, text: &str) -> String {
        self.wrap("31", text)
    }

    pub fn green(&self, text: &str) -> String {
        self.wrap("32", text)
    }

    pub fn bg_red(&self, text: &str) -> String {
        self.wrap("41;97", text)
    }
}

pub fn print_warnings(warnings: &[Warning]) {
    for warning in warnings {
        eprintln!("warning: {}", warning.message);
    }
}

/// The `--dry-run` report: what would happen, without any side effects.
pub fn print_dry_run(plan: &Plan, style: Style) {
    let action = if plan.existing_release.is_some() {
        "update"
    } else {
        "create"
    };

    let kind = if plan.tag_only {
        "annotated Git tag"
    } else {
        "GitHub Release"
    };

    let target = plan
        .target_hash
        .as_ref()
        .map(|hash| format!(" targeting {hash}"))
        .unwrap_or_default();

    println!(
        "\n{} Would {action} {kind} {}{target}",
        style.bold("Dry run"),
        style.bold(&plan.tag)
    );

    if plan.inferred_version {
        println!("Version bump inferred from selected change types.");
    }

    if let Some(id) = plan.existing_release {
        println!("Would update existing release {id}");
    }

    println!("{}\n", style.bold("\nRelease notes"));
    println!("{}", plan.changelog);

    if let Some(summary) = &plan.summary {
        println!("\n{}\n", style.bold("Update summary"));
        println!("{summary}");
    }

    println!("\n{}", style.bold("Planned effects"));
    for effect in &plan.effects {
        println!("  - {}", effect.kind.describe());
    }
}

/// The success report after work has been done.
pub fn print_outcome(outcome: &Outcome, style: Style, show_url: bool) {
    let plan = &outcome.plan;

    match (&outcome.release_url, show_url) {
        (Some(url), false) => {
            if open::open_url(url) {
                println!("\n{} Opened release in browser...", style.bold("Done!"));
                return;
            }

            println!("\n{} {url}", style.bold("Done!"));
        }
        (Some(url), true) => {
            println!("\n{} {url}", style.bold("Done!"));
        }
        (None, _) => {
            if plan.tag_only {
                println!("\n{} Tagged {}", style.bold("Done!"), style.bold(&plan.tag));
            } else {
                println!("\n{}", style.bold("Done!"));
            }
        }
    }
}

/// Renders an error for humans or for machines. Returns the text to print.
pub fn render_error(error: &Error, json: bool, style: Style) -> String {
    if json {
        let value = serde_json::json!({
            "ok": false,
            "error": {
                "code": error.code.as_str(),
                "message": &error.message,
                "remediation": &error.remediation,
            }
        });

        return serde_json::to_string_pretty(&value).unwrap_or_else(|_| error.message.clone());
    }

    let mut rendered = format!("{} {}", style.red("Error!"), error.message);

    if let Some(remediation) = &error.remediation {
        rendered.push_str(&format!("\n{} {remediation}", style.bold("Fix:")));
    }

    rendered
}

pub fn print_update_notice(version: &str, style: Style) {
    println!(
        "{} The latest version of `release` is {version}",
        style.bg_red("UPDATE AVAILABLE")
    );
}

pub fn announce_start(plan: &Plan, style: Style) {
    if plan.dry_run {
        return;
    }

    let kind = if plan.tag_only {
        "annotated Git tag"
    } else {
        "GitHub Release"
    };

    println!(
        "\n{} {kind} {}{}",
        style.bold("Preparing"),
        style.bold(&plan.tag),
        if plan.draft && !plan.tag_only {
            " (draft)"
        } else {
            ""
        }
    );
}

pub fn describe_commit_line(hash: &str, change_type: &str, title: &str) -> String {
    let short: String = hash.chars().take(7).collect();
    format!("{short}  {change_type:<6} {title}")
}
