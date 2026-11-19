//! `release` entry point.
//!
//! Dispatch rules:
//!   release                → TUI on a bare interactive invocation
//!   release <type> [...]   → CLI (flags act as the answers, so agents never block)
//!   release tui            → TUI, explicitly
//!   release help           → usage
//!   release --json         → CLI, machine-readable
//!
//! `RELEASE_NO_TUI=1` forces the CLI path even on a terminal.

use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use release_cli::args::Cli;
use release_core::plan::Options;

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().collect();
    let command = raw.get(1).map(|value| value.to_string());

    match command.as_deref() {
        Some("help") => {
            let mut command = Cli::command();
            let _ = command.print_help();
            println!();
            ExitCode::SUCCESS
        }
        Some("tui") => {
            let cli = Cli::parse_from(strip_tui_word(raw));

            match options_from(&cli) {
                Ok(options) => ExitCode::from(release_tui::run(options) as u8),
                Err(code) => code,
            }
        }
        _ => {
            let cli = Cli::parse_from(&raw);

            if cli.prefers_tui() {
                return match options_from(&cli) {
                    Ok(options) => ExitCode::from(release_tui::run(options) as u8),
                    Err(code) => code,
                };
            }

            ExitCode::from(release_cli::run(cli) as u8)
        }
    }
}

fn options_from(cli: &Cli) -> std::result::Result<Options, ExitCode> {
    cli.to_options().map_err(|message| {
        eprintln!("error: {message}");
        ExitCode::from(2)
    })
}

fn strip_tui_word(raw: Vec<String>) -> Vec<String> {
    let mut arguments = raw;
    arguments.remove(1);
    arguments
}
