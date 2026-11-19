//! CLI orchestration: resolve the context, load state, compose a plan, apply it.

pub mod args;
pub mod output;
pub mod progress;

use std::collections::BTreeMap;
use std::io::IsTerminal;

use release_core::apply;
use release_core::commits::Commit;
use release_core::github::RepoCoordinates;
use release_core::group::Classification;
use release_core::plan::{self, Options, PlanInputs};
use release_core::{update, Context, Error, Progress, Result};

use crate::args::Cli;
use crate::output::Style;
use crate::progress::TextProgress;

pub fn run(cli: Cli) -> i32 {
    let json = cli.json;
    let style = Style::detect(json);

    let options = match cli.to_options() {
        Ok(options) => options,
        Err(message) => {
            eprintln!(
                "{}",
                output::render_error(&Error::usage(message), json, style)
            );

            return release_core::ErrorCode::Usage.exit_code();
        }
    };

    let progress = TextProgress::new(false);

    match execute(options, json, cli.show_url, &progress) {
        Ok(()) => 0,
        Err(error) => {
            if let Some(url) = &error.url {
                if !cli.show_url {
                    release_core::open::open_url(url);
                }
            }

            eprintln!("{}", output::render_error(&error, json, style));
            error.code.exit_code()
        }
    }
}

pub fn execute(
    options: Options,
    json: bool,
    show_url: bool,
    progress: &dyn Progress,
) -> Result<()> {
    options.validate()?;

    let cwd = std::env::current_dir()?;
    let style = Style::detect(json);
    let ctx = Context::open(cwd, options.offline, !options.tag_only, progress)?;

    maybe_notice_update(&ctx, &options, json, style);

    let inputs = plan::load(&ctx, &options, progress)?;
    let decisions = resolve_decisions(&inputs, &options)?;
    let plan = plan::compose(&inputs, &options, &decisions, true)?;
    plan::ensure_pushable(&plan, &ctx.config.remote)?;

    output::print_warnings(&plan.warnings);

    if plan.dry_run {
        if json {
            println!("{}", serde_json::to_string_pretty(&plan.to_json())?);
        } else {
            output::print_dry_run(&plan, style);
        }

        return Ok(());
    }

    output::announce_start(&plan, style);

    let outcome = apply::apply(&plan, &ctx, progress)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&outcome.plan.to_json())?);
    } else {
        output::print_outcome(&outcome, style, show_url);
    }

    Ok(())
}

fn maybe_notice_update(ctx: &Context, options: &Options, json: bool, style: Style) {
    if json || options.offline || options.no_fetch || !std::io::stdout().is_terminal() {
        return;
    }

    if std::env::var("RELEASE_NO_UPDATE_CHECK")
        .map(|value| value == "1")
        .unwrap_or(false)
    {
        return;
    }

    let Some(forge) = &ctx.forge else {
        return;
    };

    let upstream = RepoCoordinates {
        owner: "404mat".to_string(),
        repo: "release".to_string(),
    };

    if let Some(latest) = update::check(&forge.github, &upstream) {
        output::print_update_notice(&latest, style);
    }
}

/// Turns `--set` values into classification decisions, accepting unique hash
/// prefixes the way git tooling does.
fn resolve_decisions(
    inputs: &PlanInputs,
    options: &Options,
) -> Result<BTreeMap<String, Classification>> {
    let mut decisions = BTreeMap::new();

    for (hash, change_type) in &options.assignments {
        let matches: Vec<&Commit> = inputs
            .commits
            .all
            .iter()
            .filter(|commit| commit.hash.starts_with(hash))
            .collect();

        match matches.len() {
            0 => {
                return Err(Error::usage(format!(
                    "No commit in this release matches --set {hash}."
                )));
            }
            1 => {}
            _ => {
                return Err(Error::usage(format!(
                    "--set {hash} is ambiguous; pass more characters of the commit hash."
                )));
            }
        }

        let commit = matches[0];

        let message = inputs
            .presets
            .messages
            .get(&commit.hash)
            .cloned()
            .unwrap_or_else(|| commit.title.clone());

        decisions.insert(
            commit.hash.clone(),
            Classification {
                change_type: change_type.clone(),
                message,
            },
        );
    }

    Ok(decisions)
}
