//! TUI state: rows, decisions, options and the composed plan.
//!
//! The TUI never computes a release itself. It edits decisions and options, then
//! asks the same `plan::compose` the CLI uses for the resulting plan.

use std::collections::BTreeMap;

use release_core::group::Classification;
use release_core::plan::{self, Options, Plan, PlanInputs};
use release_core::{Error, Warning};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Loading,
    Editing,
    Confirming,
    Applying,
    Done,
    Failed,
    Help,
    PickBranch,
    Summary,
}

impl Mode {
    pub fn is_overlay(self) -> bool {
        matches!(self, Self::Help | Self::PickBranch | Self::Confirming)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Editing => "editing",
            Self::Confirming => "confirm",
            Self::Applying => "applying",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Help => "help",
            Self::PickBranch => "branch",
            Self::Summary => "summary",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Row {
    pub hash: String,
    pub title: String,
    pub change_type: String,
}

pub struct App {
    pub options: Options,
    pub inputs: Option<PlanInputs>,
    pub plan: Option<Plan>,
    pub decisions: BTreeMap<String, Classification>,
    pub rows: Vec<Row>,
    pub focus: usize,
    pub preview_scroll: u16,
    pub status: String,
    pub error: Option<Error>,
    pub warnings: Vec<Warning>,
    pub mode: Mode,
    pub branches: Vec<String>,
    pub picker_index: usize,
    pub summary_buffer: String,
    pub changelog_override: Option<String>,
    pub release_url: Option<String>,
    /// Whether the shortcut band is expanded to its second line.
    pub keys_expanded: bool,
    pub quit: bool,
}

impl App {
    pub fn new(options: Options) -> Self {
        Self {
            options,
            inputs: None,
            plan: None,
            decisions: BTreeMap::new(),
            rows: Vec::new(),
            focus: 0,
            preview_scroll: 0,
            status: String::new(),
            error: None,
            warnings: Vec::new(),
            mode: Mode::Loading,
            branches: Vec::new(),
            picker_index: 0,
            summary_buffer: String::new(),
            changelog_override: None,
            release_url: None,
            keys_expanded: false,
            quit: false,
        }
    }

    pub fn loaded(&mut self, inputs: PlanInputs, branches: Vec<String>) {
        // Keep the cursor on the same commit across reloads; a bump toggle
        // reloads the commit range and resetting to the top feels like a jump.
        let focused = self.rows.get(self.focus).map(|row| row.hash.clone());

        self.inputs = Some(inputs);
        self.branches = branches;
        self.rebuild_rows();

        self.focus = focused
            .and_then(|hash| self.rows.iter().position(|row| row.hash == hash))
            .unwrap_or(0);
        self.preview_scroll = 0;
        self.status.clear();
        self.mode = Mode::Editing;
        self.recompose();
    }

    pub fn recompose(&mut self) {
        self.rebuild_rows();

        let Some(inputs) = self.inputs.clone() else {
            return;
        };

        match plan::compose(&inputs, &self.options, &self.decisions, false) {
            Ok(mut plan) => {
                if let Some(override_text) = &self.changelog_override {
                    plan.changelog = override_text.clone();
                }

                self.warnings = plan.warnings.clone();
                self.plan = Some(plan);
                self.error = None;
            }
            Err(error) => {
                self.error = Some(error);
                self.plan = None;
            }
        }
    }

    /// Re-runs composition with the hook enabled, producing the changelog that
    /// will actually be published. Called when the user asks to review.
    pub fn finalize(&mut self) {
        let Some(inputs) = self.inputs.clone() else {
            return;
        };

        match plan::compose(&inputs, &self.options, &self.decisions, true) {
            Ok(mut plan) => {
                if let Some(override_text) = &self.changelog_override {
                    plan.changelog = override_text.clone();
                }

                self.warnings = plan.warnings.clone();
                self.plan = Some(plan);
                self.error = None;
            }
            Err(error) => {
                self.error = Some(error);
                self.plan = None;
            }
        }
    }

    fn rebuild_rows(&mut self) {
        let Some(inputs) = &self.inputs else {
            self.rows.clear();
            return;
        };

        let mut rows = Vec::new();

        for commit in &inputs.commits.all {
            let Some(message) = inputs.presets.messages.get(&commit.hash) else {
                continue;
            };

            let change_type = self
                .decisions
                .get(&commit.hash)
                .map(|decision| decision.change_type.clone())
                .or_else(|| {
                    inputs
                        .presets
                        .classified
                        .get(&commit.hash)
                        .map(|preset| preset.change_type.clone())
                })
                .unwrap_or_else(|| "patch".to_string());

            rows.push(Row {
                hash: commit.hash.clone(),
                title: message.clone(),
                change_type,
            });
        }

        self.rows = rows;

        if self.focus >= self.rows.len() {
            self.focus = self.rows.len().saturating_sub(1);
        }
    }

    pub fn move_focus(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }

        let next = self.focus as isize + delta;

        self.focus = next.clamp(0, (self.rows.len() - 1) as isize) as usize;
    }

    pub fn assign(&mut self, change_type: &str) {
        let Some(row) = self.rows.get(self.focus) else {
            return;
        };

        let hash = row.hash.clone();
        let message = row.title.clone();

        self.decisions.insert(
            hash,
            Classification {
                change_type: change_type.to_string(),
                message,
            },
        );

        self.recompose();
    }

    /// Toggles the annotated-tag mode. A tag-only release stores the changelog
    /// in a tag, which needs a new version, so enabling it without a chosen bump
    /// selects the smallest one. Returns whether git state must be reloaded,
    /// because changing the bump changes the commit range.
    pub fn toggle_tag_only(&mut self) -> bool {
        if self.options.tag_only {
            self.options.tag_only = false;
            // A summary only exists to be stored in an annotated tag.
            self.options.summary = None;
            self.recompose();
            return false;
        }

        self.options.tag_only = true;
        let reload = self.select_bump_if_missing();
        self.recompose();
        reload
    }

    /// The tag transition this plan performs: `old → new`, or just the tag when
    /// there is no earlier release to compare against. Which side is "new" comes
    /// from the chosen version decision, so a bump shows `v1.2.1 → v1.2.2` and a
    /// plain release shows `v1.2.0 → v1.2.1`.
    pub fn tag_display(&self) -> String {
        match &self.plan {
            Some(plan) => match &plan.previous_tag {
                Some(previous) => format!("{} → {}", previous, plan.tag),
                None => plan.tag.clone(),
            },
            None => "-".to_string(),
        }
    }

    /// Guarantees a version bump is selected. Returns true when one was added.
    fn select_bump_if_missing(&mut self) -> bool {
        if self.options.release_type.is_some() {
            return false;
        }

        self.options.release_type = Some("patch".to_string());
        self.options.pre = false;
        true
    }

    /// Cycles the version decision: none → patch → minor → major → pre → none.
    /// Changing it changes the commit range, so the caller reloads git state.
    pub fn cycle_release_type(&mut self) {
        let next = match self.options.release_type.as_deref() {
            Some("patch") => Some("minor".to_string()),
            Some("minor") => Some("major".to_string()),
            Some("major") => Some("pre".to_string()),
            Some("pre") => None,
            _ => Some("patch".to_string()),
        };

        self.options.pre = next.as_deref() == Some("pre");

        // "No version bump" means releasing the tag that already exists, which
        // cannot be combined with an annotated tag-only release. Dry runs are
        // exempt: there the version is inferred from the change types.
        if next.is_none() && self.options.tag_only && !self.options.dry_run {
            self.options.tag_only = false;
            self.options.summary = None;
        }

        self.options.release_type = next;
    }

    pub fn toggle_dry_run(&mut self) {
        self.options.dry_run = !self.options.dry_run;
        self.recompose();
    }

    /// Toggles between a draft release and a published one. The stored option is
    /// `publish` (it mirrors `--publish`), so draft is its inverse; the UI calls
    /// this "draft" everywhere so the shortcut and the reported flag agree.
    pub fn toggle_draft(&mut self) {
        self.options.publish = !self.options.publish;
        self.recompose();
    }

    pub fn toggle_overwrite(&mut self) {
        self.options.overwrite = !self.options.overwrite;
        self.recompose();
    }

    pub fn toggle_skip_questions(&mut self) {
        self.options.skip_questions = !self.options.skip_questions;
        self.recompose();
    }

    pub fn open_picker(&mut self) {
        if self.branches.is_empty() {
            return;
        }

        let mut index = 0;

        if let Some(inputs) = &self.inputs {
            if let Some(position) = self.branches.iter().position(|name| name == &inputs.branch) {
                index = position;
            }
        }

        self.picker_index = index;
        self.mode = Mode::PickBranch;
    }

    pub fn move_picker(&mut self, delta: isize) {
        if self.branches.is_empty() {
            return;
        }

        let next = self.picker_index as isize + delta;
        self.picker_index = next.clamp(0, (self.branches.len() - 1) as isize) as usize;
    }

    pub fn selected_branch(&self) -> Option<String> {
        self.branches.get(self.picker_index).cloned()
    }

    pub fn open_summary(&mut self) {
        self.summary_buffer = self.options.summary.clone().unwrap_or_default();
        self.mode = Mode::Summary;
    }

    /// Stores the summary, which implies an annotated tag and therefore a
    /// version bump. Returns whether git state must be reloaded.
    pub fn commit_summary(&mut self) -> bool {
        let value = self.summary_buffer.trim().to_string();
        self.options.summary = if value.is_empty() { None } else { Some(value) };

        self.mode = Mode::Editing;

        let reload = if self.options.summary.is_some() {
            self.options.tag_only = true;
            self.select_bump_if_missing()
        } else {
            false
        };

        self.recompose();
        reload
    }

    /// Expands or collapses the shortcut band's second line.
    pub fn toggle_keys(&mut self) {
        self.keys_expanded = !self.keys_expanded;
    }

    pub fn scroll_preview(&mut self, delta: isize) {
        let next = self.preview_scroll as isize + delta;
        self.preview_scroll = next.max(0) as u16;
    }

    pub fn set_changelog_override(&mut self, text: String) {
        self.changelog_override = Some(text);
        self.recompose();
    }

    /// The flags state, spelled out rather than summarised, on one row so it fits
    /// an 80-column terminal. `prerelease` is not repeated here: it always
    /// mirrors `type pre`, which the line above already shows.
    /// The flags as `(label, value)` pairs. Rendering uses these so the label and
    /// the value can be styled differently.
    pub fn flag_pairs(&self) -> Vec<(&'static str, &'static str)> {
        let on = |value: bool| if value { "on" } else { "off" };

        vec![
            ("tag-only", on(self.options.tag_only)),
            ("dry-run", on(self.options.dry_run)),
            // `publish` is the CLI flag; the UI speaks in drafts.
            ("draft", on(!self.options.publish)),
            ("overwrite", on(self.options.overwrite)),
            ("headings", on(!self.options.skip_questions)),
        ]
    }

    /// The same flags as a single line, used for width calculations and tests.
    pub fn flags_line(&self) -> String {
        self.flag_pairs()
            .iter()
            .map(|(label, value)| format!("{label} {value}"))
            .collect::<Vec<_>>()
            .join(" • ")
    }
}

/// Builds the `PlanInputs` a load would produce, for tests in this crate.
#[cfg(test)]
pub(crate) fn fixture_inputs(titles: &[(&str, &str)]) -> PlanInputs {
    use release_core::commits::{Commit, Commits};
    use release_core::plan::presets;
    use release_core::tags::Tag;
    use semver::Version;
    use std::path::PathBuf;

    let commits: Vec<Commit> = titles
        .iter()
        .map(|(hash, title)| Commit {
            hash: hash.to_string(),
            title: title.to_string(),
            description: String::new(),
            author: "dev@example.com".to_string(),
        })
        .collect();

    let change_types = release_core::change_types();
    let resolved = presets(&commits, &change_types, false);

    PlanInputs {
        root: PathBuf::from("."),
        hook: None,
        branch: "main".to_string(),
        tags: vec![
            Tag {
                tag: "v1.0.1".to_string(),
                version: Version::parse("1.0.1").unwrap(),
                hash: "bbb".to_string(),
            },
            Tag {
                tag: "v1.0.0".to_string(),
                version: Version::parse("1.0.0").unwrap(),
                hash: "aaa".to_string(),
            },
        ],
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
        presets: resolved,
        warnings: Vec::new(),
        offline: false,
        preview_infers_version: false,
        previous_tag: Some("v1.0.0".to_string()),
        branch_synced: true,
        remote_available: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assigning_a_type_updates_the_plan() {
        let mut app = App::new(Options::default());
        app.loaded(fixture_inputs(&[("aaa", "Do a thing")]), Vec::new());

        app.assign("minor");

        assert_eq!(app.rows[0].change_type, "minor");
        assert!(app
            .plan
            .as_ref()
            .unwrap()
            .changelog
            .contains("### Minor Changes"));
    }

    #[test]
    fn moving_focus_stays_in_bounds() {
        let mut app = App::new(Options::default());
        app.loaded(
            fixture_inputs(&[("a", "One"), ("b", "Two"), ("c", "Three")]),
            Vec::new(),
        );

        app.move_focus(-1);
        assert_eq!(app.focus, 0);

        app.move_focus(10);
        assert_eq!(app.focus, 2);
    }

    #[test]
    fn toggling_flags_recomposes() {
        let mut app = App::new(Options::default());
        app.loaded(fixture_inputs(&[("aaa", "Do a thing")]), Vec::new());

        app.toggle_dry_run();
        assert!(app.plan.as_ref().unwrap().dry_run);

        // Releases start as drafts; toggling draft turns that off.
        assert!(app.plan.as_ref().unwrap().draft);

        app.toggle_draft();
        assert!(!app.plan.as_ref().unwrap().draft);

        app.toggle_overwrite();
        assert!(app.plan.as_ref().unwrap().overwrite);
    }

    #[test]
    fn summary_input_is_trimmed() {
        let mut app = App::new(Options {
            tag_only: true,
            ..Options::default()
        });
        app.loaded(fixture_inputs(&[("aaa", "Do a thing")]), Vec::new());

        app.open_summary();
        app.summary_buffer = "  hello  ".to_string();
        app.commit_summary();

        assert_eq!(app.options.summary.as_deref(), Some("hello"));
        assert!(app.options.tag_only);
    }

    #[test]
    fn cycles_through_release_types() {
        let mut app = App::new(Options::default());

        app.cycle_release_type();
        assert_eq!(app.options.release_type.as_deref(), Some("patch"));

        app.cycle_release_type();
        assert_eq!(app.options.release_type.as_deref(), Some("minor"));

        app.cycle_release_type();
        assert_eq!(app.options.release_type.as_deref(), Some("major"));

        app.cycle_release_type();
        assert_eq!(app.options.release_type.as_deref(), Some("pre"));
        assert!(app.options.pre);

        app.cycle_release_type();
        assert_eq!(app.options.release_type, None);
        assert!(!app.options.pre);
    }

    #[test]
    fn enabling_tag_only_selects_a_bump_and_asks_for_a_reload() {
        let mut app = App::new(Options::default());
        app.loaded(fixture_inputs(&[("aaa", "Do a thing")]), Vec::new());

        assert_eq!(app.options.release_type, None);

        let reload = app.toggle_tag_only();

        assert!(reload, "a new bump changes the commit range");
        assert!(app.options.tag_only);
        assert_eq!(app.options.release_type.as_deref(), Some("patch"));
    }

    #[test]
    fn enabling_tag_only_keeps_an_existing_bump() {
        let mut app = App::new(Options {
            release_type: Some("minor".to_string()),
            ..Options::default()
        });
        app.loaded(fixture_inputs(&[("aaa", "Do a thing")]), Vec::new());

        let reload = app.toggle_tag_only();

        assert!(!reload);
        assert_eq!(app.options.release_type.as_deref(), Some("minor"));
    }

    #[test]
    fn saving_a_summary_selects_a_bump() {
        let mut app = App::new(Options::default());
        app.loaded(fixture_inputs(&[("aaa", "Do a thing")]), Vec::new());

        app.open_summary();
        app.summary_buffer = "notes".to_string();

        let reload = app.commit_summary();

        assert!(reload);
        assert!(app.options.tag_only);
        assert_eq!(app.options.release_type.as_deref(), Some("patch"));
    }

    #[test]
    fn cycling_to_no_bump_drops_tag_only() {
        let mut app = App::new(Options {
            release_type: Some("pre".to_string()),
            tag_only: true,
            summary: Some("notes".to_string()),
            ..Options::default()
        });

        app.cycle_release_type();

        assert_eq!(app.options.release_type, None);
        assert!(!app.options.tag_only, "no bump cannot be tag-only");
        assert!(app.options.summary.is_none());
    }

    #[test]
    fn cycling_to_no_bump_keeps_tag_only_in_a_dry_run() {
        let mut app = App::new(Options {
            release_type: Some("pre".to_string()),
            tag_only: true,
            dry_run: true,
            ..Options::default()
        });

        app.cycle_release_type();

        assert_eq!(app.options.release_type, None);
        assert!(app.options.tag_only, "dry runs infer the version");
    }

    #[test]
    fn shows_the_tag_transition() {
        let mut app = App::new(Options::default());
        app.loaded(fixture_inputs(&[("aaa", "Do a thing")]), Vec::new());

        assert_eq!(app.tag_display(), "v1.0.0 → v1.0.1");
    }

    #[test]
    fn shows_a_dash_before_a_plan_exists() {
        let app = App::new(Options::default());
        assert_eq!(app.tag_display(), "-");
    }

    #[test]
    fn flag_pairs_expose_label_and_state() {
        let mut app = App::new(Options {
            tag_only: true,
            publish: true,
            ..Options::default()
        });
        app.loaded(fixture_inputs(&[("aaa", "Do a thing")]), Vec::new());

        let pairs = app.flag_pairs();

        assert_eq!(pairs.len(), 5);
        assert_eq!(pairs[0], ("tag-only", "on"));
        assert_eq!(pairs[2], ("draft", "off"), "publish = true means no draft");
    }

    #[test]
    fn flags_are_spelled_out_instead_of_summarised() {
        let mut app = App::new(Options::default());
        app.loaded(fixture_inputs(&[("aaa", "Do a thing")]), Vec::new());

        let flags = app.flags_line();

        assert_eq!(
            flags,
            "tag-only off • dry-run off • draft on • overwrite off • headings on"
        );
        assert!(!flags.contains("defaults"));
        assert!(
            flags.chars().count() <= 73,
            "must fit 80 columns next to the label"
        );
    }

    #[test]
    fn flags_report_the_toggled_state() {
        let mut app = App::new(Options {
            tag_only: true,
            dry_run: true,
            publish: true,
            overwrite: true,
            pre: true,
            skip_questions: true,
            ..Options::default()
        });
        app.loaded(fixture_inputs(&[("aaa", "Do a thing")]), Vec::new());

        let flags = app.flags_line();

        assert_eq!(
            flags,
            "tag-only on • dry-run on • draft off • overwrite on • headings off"
        );
        assert!(flags.contains(" • "), "flags are separated by a bullet");
    }

    #[test]
    fn dropping_tag_only_drops_the_summary() {
        let mut app = App::new(Options {
            tag_only: true,
            summary: Some("short".to_string()),
            ..Options::default()
        });
        app.loaded(fixture_inputs(&[("aaa", "Do a thing")]), Vec::new());

        app.toggle_tag_only();

        assert!(!app.options.tag_only);
        assert!(app.options.summary.is_none());
    }
}
