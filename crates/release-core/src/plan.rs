//! Planning a release.
//!
//! A plan is a complete, inspectable description of what a release will do:
//! version, tag, changelog, and the exact list of side effects. Both frontends
//! render the same plan and both apply it through the same code, which is what
//! keeps the TUI and the CLI at feature parity by construction.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use semver::Version;
use serde_json::{json, Value};

use crate::bump::{self, Bump};
use crate::changelog;
use crate::commits::{self, Commit, Commits};
use crate::context::Context;
use crate::definitions::{self, reference, reference_number};
use crate::group::{self, Classification};
use crate::hook::{self, HookCommit, HookGroup, HookPayload};
use crate::tags::{self, Tag};
use crate::{semver_ext, ChangeType, Error, ErrorCode, Progress, Result, Warning};

/// Everything the user asked for on the command line. Fields mirror the
/// historical flag names one-for-one.
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub release_type: Option<String>,
    pub suffix: Option<String>,
    pub pre: bool,
    pub overwrite: bool,
    pub publish: bool,
    pub hook: Option<PathBuf>,
    pub previous_tag: Option<String>,
    pub summary: Option<String>,
    pub tag_only: bool,
    pub dry_run: bool,
    pub show_url: bool,
    pub skip_questions: bool,
    pub branch: Option<String>,
    pub offline: bool,
    pub no_fetch: bool,
    pub assignments: Vec<(String, String)>,
}

impl Options {
    pub fn validate(&self) -> Result<()> {
        if let Some(release_type) = &self.release_type {
            if !crate::is_bump_type(release_type) {
                return Err(Error::usage(
                    "Version type not SemVer-compatible (\"major\", \"minor\", \"patch\" or \"pre\")",
                ));
            }
        }

        if let Some(summary) = &self.summary {
            if summary.contains('\n') || summary.contains('\r') {
                return Err(Error::usage("The update summary must fit on one line."));
            }

            if !summary.trim().is_empty() && !self.tag_only {
                return Err(Error::usage(
                    "The \"--summary\" option requires \"--tag-only\".",
                ));
            }
        }

        if self.tag_only && self.release_type.is_none() && !self.dry_run {
            return Err(Error::usage(
                "The \"--tag-only\" option requires a version bump.",
            ));
        }

        if self.offline && !self.dry_run {
            return Err(Error::usage(
                "The \"--offline\" option can only be used with \"--dry-run\".",
            ));
        }

        Ok(())
    }

    pub fn summary_text(&self) -> Option<String> {
        self.summary
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectStatus {
    Planned,
    Applied,
}

impl EffectStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Applied => "applied",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectKind {
    WriteFile { path: String },
    Commit { message: String },
    Tag { name: String, annotated: bool },
    Push,
    PushTags,
    ReleaseCreate { draft: bool, prerelease: bool },
    ReleaseUpdate { id: u64 },
}

impl EffectKind {
    pub fn code(&self) -> &'static str {
        match self {
            Self::WriteFile { .. } => "write_file",
            Self::Commit { .. } => "commit",
            Self::Tag { .. } => "tag",
            Self::Push => "push",
            Self::PushTags => "push_tags",
            Self::ReleaseCreate { .. } => "release_create",
            Self::ReleaseUpdate { .. } => "release_update",
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::WriteFile { path } => format!("write {path}"),
            Self::Commit { message } => format!("commit \"{message}\""),
            Self::Tag { name, annotated } => {
                if *annotated {
                    format!("annotated tag {name}")
                } else {
                    format!("tag {name}")
                }
            }
            Self::Push => "push to remote".to_string(),
            Self::PushTags => "push tags to remote".to_string(),
            Self::ReleaseCreate { draft, .. } => {
                if *draft {
                    "create draft GitHub Release".to_string()
                } else {
                    "publish GitHub Release".to_string()
                }
            }
            Self::ReleaseUpdate { id } => format!("update existing release {id}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effect {
    pub kind: EffectKind,
    pub status: EffectStatus,
}

impl Effect {
    pub fn planned(kind: EffectKind) -> Self {
        Self {
            kind,
            status: EffectStatus::Planned,
        }
    }

    pub fn to_json(&self) -> Value {
        let mut value = json!({
            "kind": self.kind.code(),
            "status": self.status.as_str(),
        });

        if let Some(object) = value.as_object_mut() {
            match &self.kind {
                EffectKind::WriteFile { path } => {
                    object.insert("path".into(), json!(path));
                }
                EffectKind::Commit { message } => {
                    object.insert("message".into(), json!(message));
                }
                EffectKind::Tag { name, annotated } => {
                    object.insert("name".into(), json!(name));
                    object.insert("annotated".into(), json!(annotated));
                }
                EffectKind::ReleaseCreate { draft, prerelease } => {
                    object.insert("draft".into(), json!(draft));
                    object.insert("prerelease".into(), json!(prerelease));
                }
                EffectKind::ReleaseUpdate { id } => {
                    object.insert("id".into(), json!(id));
                }
                EffectKind::Push | EffectKind::PushTags => {}
            }
        }

        value
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedCommit {
    pub hash: String,
    pub change_type: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub tag: String,
    pub version: Option<Version>,
    pub previous_tag: Option<String>,
    pub branch: String,
    pub release_type: Option<String>,
    pub changelog: String,
    pub summary: Option<String>,
    pub target_hash: Option<String>,
    pub commits: Vec<PlannedCommit>,
    pub effects: Vec<Effect>,
    pub warnings: Vec<Warning>,
    pub dry_run: bool,
    pub tag_only: bool,
    pub draft: bool,
    pub prerelease: bool,
    pub overwrite: bool,
    pub existing_release: Option<u64>,
    pub release_url: Option<String>,
    pub inferred_version: bool,
    pub bump: Option<Bump>,
    pub offline: bool,
    pub branch_synced: bool,
    pub remote_available: bool,
}

impl Plan {
    /// Whether applying this plan would push to the remote.
    pub fn needs_push(&self) -> bool {
        self.effects
            .iter()
            .any(|effect| matches!(effect.kind, EffectKind::Push | EffectKind::PushTags))
    }

    pub fn version_string(&self) -> Option<String> {
        self.version.as_ref().map(|version| version.to_string())
    }

    pub fn to_json(&self) -> Value {
        json!({
            "ok": true,
            "tag": &self.tag,
            "version": self.version_string(),
            "previous_tag": &self.previous_tag,
            "branch": &self.branch,
            "release_type": &self.release_type,
            "dry_run": self.dry_run,
            "tag_only": self.tag_only,
            "draft": self.draft,
            "prerelease": self.prerelease,
            "summary": &self.summary,
            "release_url": &self.release_url,
            "changelog": &self.changelog,
            "commits": self.commits.iter().map(|commit| json!({
                "hash": &commit.hash,
                "type": &commit.change_type,
                "title": &commit.title,
            })).collect::<Vec<_>>(),
            "effects": self.effects.iter().map(Effect::to_json).collect::<Vec<_>>(),
            "warnings": self.warnings.iter().map(Warning::to_json).collect::<Vec<_>>(),
        })
    }
}

/// Commit classification resolved before any user input is considered.
#[derive(Debug, Clone, Default)]
pub struct Presets {
    /// Display message per commit hash. Commits without a usable message are
    /// absent and never appear in the changelog.
    pub messages: BTreeMap<String, String>,
    /// Commits whose type was declared in the message itself.
    pub classified: BTreeMap<String, Classification>,
    /// Commits that still need a decision, in display order.
    pub pending: Vec<String>,
}

/// Everything read from git and GitHub, before decisions are applied.
#[derive(Debug, Clone)]
pub struct PlanInputs {
    pub root: PathBuf,
    pub hook: Option<PathBuf>,
    pub branch: String,
    pub tags: Vec<Tag>,
    pub commits: Commits,
    pub bump: Option<Bump>,
    pub existing_release: Option<u64>,
    pub existing_url: Option<String>,
    pub authors: BTreeMap<u64, Option<String>>,
    pub release_author: Option<String>,
    pub change_types: Vec<ChangeType>,
    pub presets: Presets,
    pub warnings: Vec<Warning>,
    pub offline: bool,
    pub preview_infers_version: bool,
    pub previous_tag: Option<String>,
    /// Whether the branch can be pushed as-is. Only enforced when a plan pushes.
    pub branch_synced: bool,
    pub remote_available: bool,
}

/// Reads repository state and the data needed to render a changelog.
pub fn load(ctx: &Context, opts: &Options, progress: &dyn Progress) -> Result<PlanInputs> {
    opts.validate()?;

    let git = &ctx.git;

    if !git.is_repo() {
        return Err(Error::new(
            ErrorCode::NotARepo,
            "Directory is not a Git repository.",
        ));
    }

    let mut warnings = ctx.warnings.clone();

    let target_branch = opts.branch.clone().or_else(|| ctx.config.branch.clone());
    let current_branch = git.current_branch()?;

    if let Some(target) = &target_branch {
        if target != &current_branch {
            progress.start(&format!("Switching to {target}"));
            git.switch_branch(target)?;
            progress.succeed(Some(&format!("Switched to {target}")));
        }
    }

    let branch = git.current_branch()?;

    if !opts.offline && !opts.no_fetch && ctx.remote_available {
        progress.start(&format!("Fetching from {}", ctx.config.remote));

        match git.fetch(&ctx.config.remote, ctx.config.fetch_timeout) {
            Ok(()) => progress.succeed(Some("Fetched the latest changes")),
            Err(error) => {
                let message = format!(
                    "Could not reach \"{}\": {}",
                    ctx.config.remote,
                    error.short()
                );

                progress.warn(&message);
                warnings.push(Warning::new("fetch_failed", message));
            }
        }
    }

    let mut baseline: Option<Tag> = None;

    let bump = match opts.release_type.as_deref() {
        Some(kind) => {
            progress.start("Bumping version tag");

            let listed = tags::list(git, None, &branch)?;
            baseline = listed.first().cloned();

            let suffix = opts
                .suffix
                .as_deref()
                .or(Some(ctx.config.prerelease_suffix.as_str()));

            let computed = bump::compute(&ctx.cwd, kind, suffix, baseline.as_ref())?;

            progress.succeed(Some(&format!("Bumped version tag to {}", computed.tag)));

            Some(computed)
        }
        None => None,
    };

    let (planned_tags, preview_infers_version) = match &bump {
        Some(bump) => {
            let release = Tag {
                tag: bump.tag.clone(),
                version: bump.version.clone(),
                hash: git.head_hash()?,
            };

            let mut tags = vec![release];

            // The baseline for the range is the current latest tag; the release
            // commit does not exist yet, so no commit has to be excluded.
            if let Some(previous) = &baseline {
                tags.push(previous.clone());
            }

            (tags, false)
        }
        None => {
            let listed = tags::list(git, opts.previous_tag.as_deref(), &branch)?;

            if listed.is_empty() {
                return Err(Error::new(
                    ErrorCode::NoChanges,
                    "No tags available for release.",
                ));
            }

            if opts.dry_run && opts.tag_only {
                let previous = listed[0].clone();
                let head = git.head_hash()?;

                (
                    vec![
                        Tag {
                            tag: "HEAD".to_string(),
                            version: previous.version.clone(),
                            hash: head,
                        },
                        previous,
                    ],
                    true,
                )
            } else {
                (listed, false)
            }
        }
    };

    let previous_tag = planned_tags.get(1).map(|tag| tag.tag.clone());

    let include_latest = bump.is_some() || opts.tag_only;
    let mut loaded = commits::load(git, &planned_tags, include_latest)?;

    loaded
        .all
        .retain(|commit| semver_ext::parse_lenient(&commit.title).is_none());

    if loaded.all.is_empty() {
        return Err(Error::new(
            ErrorCode::NoChanges,
            "No changes happened since the last release.",
        ));
    }

    // A diverged branch only matters when the plan will push. Record it instead
    // of failing: the TUI must open on any repository, and the check is enforced
    // through `ensure_pushable` before anything is applied.
    let branch_synced = if opts.dry_run || !ctx.remote_available {
        true
    } else {
        git.branch_synced()
    };

    let mut existing_release = None;
    let mut existing_url = None;

    if let Some(forge) = &ctx.forge {
        progress.start("Checking if release already exists");

        match forge.github.list_releases(&forge.coordinates) {
            Ok(releases) => {
                for release in releases {
                    if release.tag_name == planned_tags[0].tag {
                        existing_release = Some(release.id);
                        existing_url = Some(release.html_url);
                        break;
                    }
                }

                progress.succeed(None);
            }
            Err(error) => {
                let message = format!("Could not check existing releases: {}", error.short());
                progress.warn(&message);
                warnings.push(Warning::new("release_check_failed", message));
            }
        }
    } else if !warnings
        .iter()
        .any(|warning| warning.code == "no_remote" || warning.code == "github_unavailable")
    {
        // Only fall back to the generic message when the context has not
        // already explained why GitHub is missing.
        warnings.push(Warning::new(
            "offline",
            "GitHub is unavailable, so the release state and credits were not checked.",
        ));
    }

    let mut numbers = BTreeSet::new();

    for commit in loaded.all.iter().chain(loaded.latest.iter()) {
        if let Some(reference) = reference(&commit.title) {
            if let Some(number) = reference_number(&reference) {
                numbers.insert(number);
            }
        }
    }

    let mut authors = BTreeMap::new();
    let mut release_author = None;

    if let Some(forge) = &ctx.forge {
        if !numbers.is_empty() {
            let list: Vec<u64> = numbers.into_iter().collect();
            authors = forge.github.pull_authors(&forge.coordinates, &list);
        }

        if let Some(latest) = &loaded.latest {
            release_author = forge.github.username_for_email(&latest.author);
        }
    }

    let change_types = crate::change_types();
    let presets = presets(&loaded.all, &change_types, opts.skip_questions);

    Ok(PlanInputs {
        root: ctx.cwd.clone(),
        hook: ctx.config.hook.clone(),
        branch,
        tags: planned_tags,
        commits: loaded,
        bump,
        existing_release,
        existing_url,
        authors,
        release_author,
        change_types,
        presets,
        warnings,
        offline: ctx.forge.is_none(),
        preview_infers_version,
        previous_tag,
        branch_synced,
        remote_available: ctx.remote_available,
    })
}

/// Resolves display messages and pre-declared types for every commit. Mirrors
/// the original message-selection rules, including multi-line bodies.
pub fn presets(commits: &[Commit], change_types: &[ChangeType], skip_questions: bool) -> Presets {
    let mut resolved = Presets::default();

    for commit in commits {
        let declared = definitions::type_of(&commit.title, change_types)
            .or_else(|| definitions::type_of(&commit.description, change_types));

        let mut message = commit.title.clone();

        if message.is_empty() {
            for line in commit.description.split('\n') {
                if line.is_empty() {
                    continue;
                }

                let cleaned = line.replace("* ", "");
                let cleaned = cleaned.trim();

                if cleaned.is_empty() {
                    continue;
                }

                if cleaned.chars().count() > 1 {
                    message = cleaned.to_string();
                    break;
                }
            }
        }

        if message.trim().is_empty() {
            continue;
        }

        resolved
            .messages
            .insert(commit.hash.clone(), message.clone());

        match declared {
            Some(change_type) => {
                resolved.classified.insert(
                    commit.hash.clone(),
                    Classification {
                        change_type,
                        message,
                    },
                );
            }
            None => {
                if skip_questions {
                    resolved.classified.insert(
                        commit.hash.clone(),
                        Classification {
                            change_type: "patch".to_string(),
                            message,
                        },
                    );
                } else {
                    resolved.pending.push(commit.hash.clone());
                }
            }
        }
    }

    resolved
}

/// Turns loaded state plus decisions into an inspectable plan.
pub fn compose(
    inputs: &PlanInputs,
    opts: &Options,
    decisions: &BTreeMap<String, Classification>,
    run_hook: bool,
) -> Result<Plan> {
    if !opts.tag_only && inputs.existing_release.is_some() && !opts.overwrite && !opts.dry_run {
        let url = inputs.existing_url.clone().unwrap_or_default();

        let mut error = Error::new(
            ErrorCode::ReleaseExists,
            if url.is_empty() {
                "Release already exists.".to_string()
            } else {
                format!("Release already exists: {url}")
            },
        )
        .with_remediation("Pass --overwrite to replace the existing release.");

        if !url.is_empty() {
            error = error.with_url(url);
        }

        return Err(error);
    }

    let mut warnings = inputs.warnings.clone();
    let mut changes: Vec<(String, String, String)> = Vec::new();
    let mut planned_commits: Vec<PlannedCommit> = Vec::new();
    let mut defaulted = 0usize;

    for commit in &inputs.commits.all {
        let Some(message) = inputs.presets.messages.get(&commit.hash) else {
            continue;
        };

        let change_type = decisions
            .get(&commit.hash)
            .map(|decision| decision.change_type.clone())
            .or_else(|| {
                inputs
                    .presets
                    .classified
                    .get(&commit.hash)
                    .map(|preset| preset.change_type.clone())
            })
            .unwrap_or_else(|| {
                defaulted += 1;
                "patch".to_string()
            });

        if change_type == "ignore" {
            continue;
        }

        changes.push((commit.hash.clone(), change_type.clone(), message.clone()));

        planned_commits.push(PlannedCommit {
            hash: commit.hash.clone(),
            change_type,
            title: message.clone(),
        });
    }

    if defaulted > 0 {
        warnings.push(Warning::new(
            "unclassified_commits",
            format!(
                "{defaulted} commit(s) had no change type and were treated as patch. Use --set <hash>=<type> to classify them."
            ),
        ));
    }

    let grouped = group::group(&changes, &inputs.change_types);

    let mut tag = inputs
        .tags
        .first()
        .map(|tag| tag.tag.clone())
        .unwrap_or_else(|| "HEAD".to_string());
    let mut version = inputs.bump.as_ref().map(|bump| bump.version.clone());
    let mut release_type = opts.release_type.clone();

    if inputs.preview_infers_version {
        if let Some(previous) = inputs.tags.get(1) {
            let mut inferred: Option<String> = None;

            for handle in ["major", "minor", "patch"] {
                let present = grouped
                    .iter()
                    .any(|(name, list)| name.as_str() == handle && !list.is_empty());

                if present {
                    inferred = Some(handle.to_string());
                    break;
                }
            }

            if let Some(inferred) = inferred {
                if let Ok(next) = semver_ext::inc(&previous.version, &inferred, None) {
                    tag = format!("{}{}", previous.prefix(), next);
                    version = Some(next);
                    release_type = Some(inferred);
                }
            }
        }
    }

    let resolved_hook = hook::resolve(&inputs.root, opts.hook.as_deref(), inputs.hook.as_deref());

    let escape_html = resolved_hook.is_none();

    let rendered = changelog::render(
        &grouped,
        &inputs.change_types,
        opts.skip_questions,
        escape_html,
        &inputs.authors,
        inputs.release_author.as_deref(),
    );

    let mut changelog = rendered
        .changelog
        .unwrap_or_else(|| "Initial release".to_string());

    let mut summary = opts.summary_text();

    if run_hook {
        if let Some(path) = &resolved_hook {
            let mut payload =
                HookPayload::new(changelog.clone(), tag.clone(), inputs.branch.clone())
                    .with_change_types(&inputs.change_types);

            payload.version = version.as_ref().map(|value| value.to_string());
            payload.authors = rendered.credits.clone();
            payload.commits = planned_commits
                .iter()
                .map(|commit| {
                    let source = inputs
                        .commits
                        .all
                        .iter()
                        .find(|candidate| candidate.hash == commit.hash);

                    HookCommit {
                        hash: commit.hash.clone(),
                        title: commit.title.clone(),
                        description: source
                            .map(|candidate| candidate.description.clone())
                            .unwrap_or_default(),
                        author: source
                            .map(|candidate| candidate.author.clone())
                            .unwrap_or_default(),
                    }
                })
                .collect();
            payload.grouped = grouped
                .iter()
                .map(|(handle, list)| HookGroup {
                    change_type: handle.clone(),
                    commits: list
                        .iter()
                        .map(|change| HookCommit {
                            hash: change.hash.clone(),
                            title: change.message.clone(),
                            description: String::new(),
                            author: String::new(),
                        })
                        .collect(),
                })
                .collect();

            let output = hook::run(path, &payload)?;

            changelog = output.changelog;

            if summary.is_none() {
                summary = output.summary;
            }
        }
    }

    let draft = !opts.publish;
    let effects = build_effects(
        inputs.bump.as_ref(),
        &tag,
        opts.tag_only,
        draft,
        opts.pre,
        inputs.existing_release,
    );

    let pushes = effects
        .iter()
        .any(|effect| matches!(effect.kind, EffectKind::Push | EffectKind::PushTags));

    if pushes && !inputs.branch_synced && inputs.remote_available && !opts.dry_run {
        warnings.push(Warning::new(
            "branch_not_synced",
            "Your branch needs to be up-to-date with origin.",
        ));
    }

    Ok(Plan {
        tag,
        version,
        previous_tag: inputs.previous_tag.clone(),
        branch: inputs.branch.clone(),
        release_type,
        changelog,
        summary,
        target_hash: inputs.tags.first().map(|tag| tag.hash.clone()),
        commits: planned_commits,
        effects,
        warnings,
        dry_run: opts.dry_run,
        tag_only: opts.tag_only,
        draft,
        prerelease: opts.pre,
        overwrite: opts.overwrite,
        existing_release: inputs.existing_release,
        release_url: None,
        inferred_version: inputs.preview_infers_version,
        bump: inputs.bump.clone(),
        offline: inputs.offline,
        branch_synced: inputs.branch_synced,
        remote_available: inputs.remote_available,
    })
}

/// Pre-flight check for plans that push. Never fails a preview, and never fails
/// when there is no remote to be out of date with.
pub fn ensure_pushable(plan: &Plan, remote: &str) -> Result<()> {
    if plan.dry_run || !plan.needs_push() || !plan.remote_available || plan.branch_synced {
        return Ok(());
    }

    Err(Error::new(
        ErrorCode::BranchNotSynced,
        "Your branch needs to be up-to-date with origin.",
    )
    .with_remediation(format!("git pull --rebase {remote} {}", plan.branch)))
}

fn build_effects(
    bump: Option<&Bump>,
    tag: &str,
    tag_only: bool,
    draft: bool,
    prerelease: bool,
    existing: Option<u64>,
) -> Vec<Effect> {
    let mut effects = Vec::new();

    if let Some(bump) = bump {
        for (path, _) in &bump.writes {
            effects.push(Effect::planned(EffectKind::WriteFile {
                path: path.display().to_string(),
            }));
        }

        effects.push(Effect::planned(EffectKind::Commit {
            message: bump.tag.clone(),
        }));
    }

    if tag_only {
        effects.push(Effect::planned(EffectKind::Tag {
            name: tag.to_string(),
            annotated: true,
        }));
    } else if bump.is_some() {
        effects.push(Effect::planned(EffectKind::Tag {
            name: tag.to_string(),
            annotated: false,
        }));
    }

    if bump.is_some() || tag_only {
        effects.push(Effect::planned(EffectKind::Push));
        effects.push(Effect::planned(EffectKind::PushTags));
    }

    if !tag_only {
        match existing {
            Some(id) => effects.push(Effect::planned(EffectKind::ReleaseUpdate { id })),
            None => effects.push(Effect::planned(EffectKind::ReleaseCreate {
                draft,
                prerelease,
            })),
        }
    }

    effects
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commits::Commit;

    fn commit(hash: &str, title: &str, description: &str) -> Commit {
        Commit {
            hash: hash.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            author: "dev@example.com".to_string(),
        }
    }

    #[test]
    fn reads_types_declared_in_the_message() {
        let commits = vec![commit("a", "Add thing (minor)", "")];
        let presets = presets(&commits, &crate::change_types(), false);

        assert_eq!(presets.pending.len(), 0);
        assert_eq!(presets.classified["a"].change_type, "minor");
    }

    #[test]
    fn falls_back_to_the_description_for_the_message() {
        let commits = vec![commit(
            "a",
            "",
            "\n* Implement wonderful things\n* And more",
        )];
        let presets = presets(&commits, &crate::change_types(), false);

        assert_eq!(
            presets.messages.get("a").map(String::as_str),
            Some("Implement wonderful things")
        );
    }

    #[test]
    fn drops_commits_without_a_usable_message() {
        let commits = vec![commit("a", "", "")];
        let presets = presets(&commits, &crate::change_types(), false);

        assert!(presets.messages.is_empty());
        assert!(presets.pending.is_empty());
    }

    #[test]
    fn marking_questions_as_skipped_classifies_everything_as_patch() {
        let commits = vec![commit("a", "Undocumented change", "")];
        let presets = presets(&commits, &crate::change_types(), true);

        assert!(presets.pending.is_empty());
        assert_eq!(presets.classified["a"].change_type, "patch");
    }

    fn inputs(commits: Vec<Commit>) -> PlanInputs {
        let change_types = crate::change_types();
        let presets = presets(&commits, &change_types, false);

        PlanInputs {
            root: PathBuf::from("."),
            hook: None,
            branch: "main".to_string(),
            tags: vec![Tag {
                tag: "v1.0.0".to_string(),
                version: Version::parse("1.0.0").unwrap(),
                hash: "abc".to_string(),
            }],
            commits: Commits {
                all: commits,
                latest: None,
            },
            bump: None,
            existing_release: None,
            existing_url: None,
            authors: BTreeMap::new(),
            release_author: None,
            change_types,
            presets,
            warnings: Vec::new(),
            offline: false,
            preview_infers_version: false,
            previous_tag: None,
            branch_synced: true,
            remote_available: true,
        }
    }

    #[test]
    fn compose_defaults_unclassified_commits_and_warns() {
        let plan = compose(
            &inputs(vec![commit("a", "Mystery change", "")]),
            &Options::default(),
            &BTreeMap::new(),
            false,
        )
        .unwrap();

        assert_eq!(plan.commits[0].change_type, "patch");
        assert!(plan
            .warnings
            .iter()
            .any(|warning| warning.code == "unclassified_commits"));
    }

    #[test]
    fn compose_honours_explicit_decisions() {
        let mut decisions = BTreeMap::new();
        decisions.insert(
            "a".to_string(),
            Classification {
                change_type: "minor".to_string(),
                message: "Mystery change".to_string(),
            },
        );

        let plan = compose(
            &inputs(vec![commit("a", "Mystery change", "")]),
            &Options::default(),
            &decisions,
            false,
        )
        .unwrap();

        assert_eq!(plan.commits[0].change_type, "minor");
        assert!(plan.changelog.contains("### Minor Changes"));
    }

    #[test]
    fn compose_omits_a_tag_effect_for_a_plain_release() {
        let plan = compose(
            &inputs(vec![commit("a", "Fix (patch)", "")]),
            &Options::default(),
            &BTreeMap::new(),
            false,
        )
        .unwrap();

        assert_eq!(plan.effects.len(), 1);
        assert_eq!(plan.effects[0].kind.code(), "release_create");
    }

    #[test]
    fn compose_refuses_an_existing_release_without_overwrite() {
        let mut plan_inputs = inputs(vec![commit("a", "Fix (patch)", "")]);
        plan_inputs.existing_release = Some(7);
        plan_inputs.existing_url = Some("https://example.test/release".to_string());

        let error =
            compose(&plan_inputs, &Options::default(), &BTreeMap::new(), false).unwrap_err();

        assert_eq!(error.code, ErrorCode::ReleaseExists);
        assert_eq!(error.url.as_deref(), Some("https://example.test/release"));
    }

    #[test]
    fn dry_run_plans_but_does_not_fail_on_an_existing_release() {
        let mut plan_inputs = inputs(vec![commit("a", "Fix (patch)", "")]);
        plan_inputs.existing_release = Some(7);

        let options = Options {
            dry_run: true,
            ..Options::default()
        };

        let plan = compose(&plan_inputs, &options, &BTreeMap::new(), false).unwrap();
        assert_eq!(plan.effects[0].kind.code(), "release_update");
    }

    #[test]
    fn tag_only_releases_supercede_the_github_release() {
        let mut plan_inputs = inputs(vec![commit("a", "Fix (patch)", "")]);
        plan_inputs.bump = Some(Bump {
            old_version: Version::parse("1.0.0").unwrap(),
            version: Version::parse("1.0.1").unwrap(),
            tag: "v1.0.1".to_string(),
            writes: vec![(PathBuf::from("package.json"), "{}\n".to_string())],
        });

        let options = Options {
            tag_only: true,
            release_type: Some("patch".to_string()),
            ..Options::default()
        };

        let plan = compose(&plan_inputs, &options, &BTreeMap::new(), false).unwrap();
        let codes: Vec<&str> = plan
            .effects
            .iter()
            .map(|effect| effect.kind.code())
            .collect();

        assert_eq!(
            codes,
            vec!["write_file", "commit", "tag", "push", "push_tags"]
        );
        assert!(plan.effects.iter().any(|effect| matches!(
            effect.kind,
            EffectKind::Tag {
                annotated: true,
                ..
            }
        )));
    }

    #[test]
    fn version_bumps_push_before_uploading() {
        let mut plan_inputs = inputs(vec![commit("a", "Fix (patch)", "")]);
        plan_inputs.bump = Some(Bump {
            old_version: Version::parse("1.0.0").unwrap(),
            version: Version::parse("1.0.1").unwrap(),
            tag: "v1.0.1".to_string(),
            writes: vec![(PathBuf::from("package.json"), "{}\n".to_string())],
        });

        let options = Options {
            release_type: Some("patch".to_string()),
            ..Options::default()
        };

        let plan = compose(&plan_inputs, &options, &BTreeMap::new(), false).unwrap();
        let codes: Vec<&str> = plan
            .effects
            .iter()
            .map(|effect| effect.kind.code())
            .collect();

        assert_eq!(
            codes,
            vec![
                "write_file",
                "commit",
                "tag",
                "push",
                "push_tags",
                "release_create"
            ]
        );
    }

    fn bumped(commits: Vec<Commit>) -> PlanInputs {
        let mut plan_inputs = inputs(commits);
        plan_inputs.bump = Some(Bump {
            old_version: Version::parse("1.0.0").unwrap(),
            version: Version::parse("1.0.1").unwrap(),
            tag: "v1.0.1".to_string(),
            writes: vec![(PathBuf::from("package.json"), "{}\n".to_string())],
        });
        plan_inputs
    }

    fn push_options() -> Options {
        Options {
            release_type: Some("patch".to_string()),
            ..Options::default()
        }
    }

    #[test]
    fn a_diverged_branch_blocks_a_plan_that_pushes() {
        let mut plan_inputs = bumped(vec![commit("a", "Fix (patch)", "")]);
        plan_inputs.branch_synced = false;

        let plan = compose(&plan_inputs, &push_options(), &BTreeMap::new(), false).unwrap();

        assert!(plan.needs_push());
        assert!(plan
            .warnings
            .iter()
            .any(|warning| warning.code == "branch_not_synced"));

        let error = ensure_pushable(&plan, "origin").unwrap_err();

        assert_eq!(error.code, ErrorCode::BranchNotSynced);
        assert_eq!(
            error.message,
            "Your branch needs to be up-to-date with origin."
        );
        assert_eq!(
            error.remediation.as_deref(),
            Some("git pull --rebase origin main")
        );
    }

    #[test]
    fn a_diverged_branch_does_not_block_a_release_that_only_uploads() {
        let mut plan_inputs = inputs(vec![commit("a", "Fix (patch)", "")]);
        plan_inputs.branch_synced = false;

        let plan = compose(&plan_inputs, &Options::default(), &BTreeMap::new(), false).unwrap();

        assert!(!plan.needs_push());
        assert!(ensure_pushable(&plan, "origin").is_ok());
    }

    #[test]
    fn a_diverged_branch_is_ignored_without_a_remote_or_in_a_dry_run() {
        let mut no_remote = bumped(vec![commit("a", "Fix (patch)", "")]);
        no_remote.branch_synced = false;
        no_remote.remote_available = false;

        let plan = compose(&no_remote, &push_options(), &BTreeMap::new(), false).unwrap();
        assert!(ensure_pushable(&plan, "origin").is_ok());

        let mut dry = bumped(vec![commit("a", "Fix (patch)", "")]);
        dry.branch_synced = false;

        let options = Options {
            dry_run: true,
            ..push_options()
        };
        let plan = compose(&dry, &options, &BTreeMap::new(), false).unwrap();

        assert!(ensure_pushable(&plan, "origin").is_ok());
    }

    #[test]
    fn summary_requires_tag_only() {
        let options = Options {
            summary: Some("short".to_string()),
            ..Options::default()
        };

        assert!(options.validate().is_err());
    }

    #[test]
    fn rejects_multiline_summaries() {
        let options = Options {
            summary: Some("two\nlines".to_string()),
            tag_only: true,
            ..Options::default()
        };

        assert!(options.validate().is_err());
    }

    #[test]
    fn tag_only_requires_a_bump_outside_dry_runs() {
        let options = Options {
            tag_only: true,
            ..Options::default()
        };

        assert!(options.validate().is_err());

        let dry = Options {
            tag_only: true,
            dry_run: true,
            ..Options::default()
        };

        assert!(dry.validate().is_ok());
    }

    #[test]
    fn offline_requires_a_dry_run() {
        let options = Options {
            offline: true,
            ..Options::default()
        };

        assert!(options.validate().is_err());
    }
}
