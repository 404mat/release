//! Applies a plan. Every side effect in the process happens here.

use serde_json::json;

use crate::context::Context;
use crate::plan::{Effect, EffectKind, EffectStatus, Plan};
use crate::{Error, ErrorCode, Progress, Result};

#[derive(Debug, Clone)]
pub struct Outcome {
    pub plan: Plan,
    pub release_url: Option<String>,
}

/// Executes the plan's effects in order, stopping at the first failure.
///
/// Local work happens before anything leaves the machine, so a failure always
/// names the exact step that broke and leaves the repository in a state the user
/// can inspect. A dry run performs no work at all and returns the plan as-is.
pub fn apply(plan: &Plan, ctx: &Context, progress: &dyn Progress) -> Result<Outcome> {
    if plan.dry_run {
        return Ok(Outcome {
            plan: plan.clone(),
            release_url: None,
        });
    }

    crate::plan::ensure_pushable(plan, &ctx.config.remote)?;

    let mut applied: Vec<Effect> = Vec::with_capacity(plan.effects.len());
    let mut release_url = None;

    for effect in &plan.effects {
        apply_effect(effect, plan, ctx, progress, &mut release_url)?;

        applied.push(Effect {
            kind: effect.kind.clone(),
            status: EffectStatus::Applied,
        });
    }

    let mut finished = plan.clone();
    finished.effects = applied;
    finished.release_url = release_url.clone();

    Ok(Outcome {
        plan: finished,
        release_url,
    })
}

fn apply_effect(
    effect: &Effect,
    plan: &Plan,
    ctx: &Context,
    progress: &dyn Progress,
    release_url: &mut Option<String>,
) -> Result<()> {
    let git = &ctx.git;

    match &effect.kind {
        EffectKind::WriteFile { path } => {
            let bump = plan.bump.as_ref().ok_or_else(|| {
                Error::other("The plan wanted to write a file but carries no bump information.")
            })?;

            let (target, contents) = bump
                .writes
                .iter()
                .find(|(candidate, _)| candidate.display().to_string() == *path)
                .ok_or_else(|| Error::other(format!("No prepared contents for {path}")))?;

            std::fs::write(target, contents).map_err(|error| {
                Error::new(
                    ErrorCode::WriteFailed,
                    format!("Couldn't write to \"{}\": {error}", target.display()),
                )
            })?;
        }
        EffectKind::Commit { message } => {
            progress.start("Creating release commit");
            git.add_all()?;
            git.commit(message)?;
            progress.update("Created release commit");
        }
        EffectKind::Tag { name, annotated } => {
            progress.start("Tagging commit");

            if *annotated {
                let mut messages = vec![plan.changelog.clone()];

                if let Some(summary) = &plan.summary {
                    messages.push(format!("Update-Summary: {summary}"));
                }

                git.tag_annotated(name, &messages)?;
            } else {
                git.tag_lightweight(name)?;
            }

            progress.update("Tagged commit");
        }
        EffectKind::Push => {
            progress.start("Pushing everything to remote");

            // Never let git prompt inside the alternate screen; the TUI probes
            // the remote while suspended before starting the apply.
            git.prompting(false).push()?;
            progress.succeed(Some("Pushed everything to remote"));
        }
        EffectKind::PushTags => {
            git.prompting(false).push_tags()?;
        }
        EffectKind::ReleaseCreate { draft, prerelease } => {
            let forge = ctx.forge()?;
            let label = if plan.prerelease || *prerelease {
                "Uploading prerelease"
            } else {
                "Uploading release"
            };

            progress.start(label);

            let payload = json!({
                "tag_name": &plan.tag,
                "target_commitish": plan.target_hash.clone(),
                "body": &plan.changelog,
                "draft": draft,
                "prerelease": prerelease,
            });

            let release = forge.github.create_release(&forge.coordinates, &payload)?;

            progress.succeed(None);
            *release_url = Some(release.html_url);
        }
        EffectKind::ReleaseUpdate { id } => {
            let forge = ctx.forge()?;
            let label = if plan.prerelease {
                "Uploading prerelease"
            } else {
                "Uploading release"
            };

            progress.start(label);

            let payload = json!({
                "tag_name": &plan.tag,
                "target_commitish": plan.target_hash.clone(),
                "body": &plan.changelog,
                "draft": plan.draft,
                "prerelease": plan.prerelease,
            });

            let release = forge
                .github
                .update_release(&forge.coordinates, *id, &payload)?;

            progress.succeed(None);
            *release_url = Some(release.html_url);
        }
    }

    Ok(())
}
