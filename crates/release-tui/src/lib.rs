//! Interactive terminal frontend.
//!
//! The TUI is a renderer of the shared plan: every action it offers exists as a
//! flag on the CLI. It never performs side effects itself — work happens in
//! `release_core::apply`, on a worker thread, with progress forwarded here.

pub mod app;
pub mod ui;

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use release_core::apply::{self, Outcome};
use release_core::plan::{self, EffectKind as PlanEffect, Options, Plan, PlanInputs};
use release_core::{Context, Error, Progress, Result, Warning};

use crate::app::{App, Mode};

type Tui = Terminal<CrosstermBackend<std::io::Stdout>>;

pub fn run(options: Options) -> i32 {
    match run_inner(options) {
        Ok(()) => 0,
        Err(error) => {
            restore_terminal();
            eprintln!("Error! {}", error.message);

            if let Some(remediation) = &error.remediation {
                eprintln!("Fix: {remediation}");
            }

            error.code.exit_code()
        }
    }
}

fn run_inner(options: Options) -> Result<()> {
    let mut terminal = setup_terminal()?;

    let _guard = TerminalGuard;

    let result = event_loop(&mut terminal, options);

    let _ = disable_raw_mode();
    let _ = execute!(std::io::stdout(), LeaveAlternateScreen);

    result
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

fn setup_terminal() -> Result<Tui> {
    use std::io::IsTerminal;

    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(Error::usage(
            "The interactive interface needs a terminal. Use the command line instead, for example: release patch --dry-run",
        ));
    }

    enable_raw_mode().map_err(Error::from)?;

    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(Error::from)?;

    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).map_err(Error::from)
}

fn resume_terminal() {
    let _ = execute!(std::io::stdout(), EnterAlternateScreen);
    let _ = enable_raw_mode();
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
}

fn suspend_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
}

enum ProgressEvent {
    Start(String),
    Succeed(Option<String>),
    Warn(String),
}

enum Msg {
    Loaded(Result<(Arc<Context>, PlanInputs, Vec<String>)>),
    Applied(Result<Outcome>),
    Progress(ProgressEvent),
}

struct TuiProgress {
    tx: Mutex<Sender<Msg>>,
}

impl TuiProgress {
    fn new(tx: Sender<Msg>) -> Self {
        Self { tx: Mutex::new(tx) }
    }

    fn send(&self, event: ProgressEvent) {
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.send(Msg::Progress(event));
        }
    }
}

impl Progress for TuiProgress {
    fn start(&self, message: &str) {
        self.send(ProgressEvent::Start(message.to_string()));
    }

    fn update(&self, message: &str) {
        self.send(ProgressEvent::Start(message.to_string()));
    }

    fn succeed(&self, message: Option<&str>) {
        self.send(ProgressEvent::Succeed(message.map(str::to_string)));
    }

    fn warn(&self, message: &str) {
        self.send(ProgressEvent::Warn(message.to_string()));
    }
}

enum Action {
    None,
    Quit,
    Apply,
    /// Re-read git and fetch from the remote.
    Reload,
    /// Re-read git only. Used when the commit range changes because the bump
    /// changed; the entry fetch already happened.
    ReloadLocal,
    Edit,
    OpenUrl(String),
}

fn event_loop(terminal: &mut Tui, options: Options) -> Result<()> {
    let (tx, rx): (Sender<Msg>, Receiver<Msg>) = channel();
    let mut app = App::new(options.clone());

    let mut context: Option<Arc<Context>> = None;

    spawn_initial_load(tx.clone(), options);

    loop {
        terminal
            .draw(|frame| ui::draw(frame, &app))
            .map_err(Error::from)?;

        while let Ok(message) = rx.try_recv() {
            match message {
                Msg::Progress(event) => apply_progress(&mut app, event),
                Msg::Loaded(result) => match result {
                    Ok((ctx, inputs, branches)) => {
                        context = Some(ctx);
                        app.loaded(inputs, branches);
                    }
                    Err(error) => {
                        app.error = Some(error);

                        if app.inputs.is_some() {
                            // A reload failed (for example an invalid option
                            // combination): keep the current plan and stay usable.
                            app.mode = Mode::Editing;
                            app.status.clear();
                        } else {
                            app.mode = Mode::Failed;
                        }
                    }
                },
                Msg::Applied(result) => match result {
                    Ok(outcome) => {
                        app.release_url = outcome.release_url.clone();
                        app.status = "Release finished.".to_string();
                        app.plan = Some(outcome.plan);
                        app.mode = Mode::Done;
                    }
                    Err(error) => {
                        app.error = Some(error);
                        app.mode = Mode::Failed;
                    }
                },
            }
        }

        if app.quit {
            break;
        }

        if !event::poll(Duration::from_millis(120)).map_err(Error::from)? {
            continue;
        }

        let Event::Key(key) = event::read().map_err(Error::from)? else {
            continue;
        };

        if key.kind != KeyEventKind::Press {
            continue;
        }

        let action = handle_key(&mut app, key);

        match action {
            Action::None => {}
            Action::Quit => app.quit = true,
            Action::OpenUrl(url) => {
                release_core::open::open_url(&url);
            }
            Action::Reload => {
                if let Some(ctx) = &context {
                    app.mode = Mode::Loading;
                    app.status = "Reloading…".to_string();
                    spawn_reload(tx.clone(), Arc::clone(ctx), app.options.clone(), true);
                } else {
                    // The initial load failed (for example because gh was not
                    // authenticated yet); retry the whole setup.
                    app.error = None;
                    app.mode = Mode::Loading;
                    app.status = "Retrying…".to_string();
                    spawn_initial_load(tx.clone(), app.options.clone());
                }
            }
            Action::ReloadLocal => {
                if let Some(ctx) = &context {
                    app.mode = Mode::Loading;
                    app.status = "Reloading…".to_string();
                    spawn_reload(tx.clone(), Arc::clone(ctx), app.options.clone(), false);
                }
            }
            Action::Edit => {
                if let Some(plan) = &app.plan {
                    let original = plan.changelog.clone();

                    suspend_terminal();
                    let edited = edit_in_editor(&original);
                    resume_terminal();
                    let _ = terminal.clear();

                    if let Some(edited) = edited {
                        app.set_changelog_override(edited);
                    }
                }
            }
            Action::Apply => {
                let (Some(ctx), Some(plan)) = (&context, app.plan.clone()) else {
                    app.error = Some(Error::other("There is no plan to apply yet."));
                    continue;
                };

                if !plan.dry_run
                    && plan.effects.iter().any(|effect| {
                        matches!(
                            &effect.kind,
                            PlanEffect::Push
                                | PlanEffect::PushTags
                                | PlanEffect::ReleaseCreate { .. }
                                | PlanEffect::ReleaseUpdate { .. }
                        )
                    })
                {
                    // Hand the terminal back so git can ask for credentials or an
                    // SSH passphrase, which cannot be answered inside raw mode.
                    let probe = ctx.git.prompting(true);

                    suspend_terminal();
                    let reachable =
                        probe.remote_reachable(&ctx.config.remote, ctx.config.fetch_timeout);
                    resume_terminal();
                    let _ = terminal.clear();

                    if !reachable {
                        app.status =
                            "The remote did not accept credentials; the push may fail.".to_string();
                    }
                }

                app.mode = Mode::Applying;
                app.status = "Applying the release…".to_string();
                spawn_apply(tx.clone(), Arc::clone(ctx), plan);
            }
        }
    }

    Ok(())
}

fn apply_progress(app: &mut App, event: ProgressEvent) {
    match event {
        ProgressEvent::Start(message) => app.status = message,
        ProgressEvent::Succeed(message) => {
            if let Some(message) = message {
                app.status = message;
            }
        }
        ProgressEvent::Warn(message) => {
            app.warnings.push(Warning::new("runtime", message));
        }
    }
}

fn spawn_initial_load(tx: Sender<Msg>, options: Options) {
    thread::spawn(move || {
        let progress = TuiProgress::new(tx.clone());

        let result = (|| -> Result<(Arc<Context>, PlanInputs, Vec<String>)> {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            // The TUI must open even without a usable GitHub remote: locally
            // it can still classify commits, preview the notes and create a
            // tag-only release. Missing GitHub is surfaced as a warning, and
            // only fails at the moment a GitHub release would be uploaded.
            let ctx = Arc::new(Context::open(cwd, options.offline, false, &progress)?);
            let inputs = plan::load(&ctx, &options, &progress)?;
            let branches = ctx.git.local_branches().unwrap_or_default();

            Ok((ctx, inputs, branches))
        })();

        let _ = tx.send(Msg::Loaded(result));
    });
}

fn spawn_reload(tx: Sender<Msg>, ctx: Arc<Context>, mut options: Options, fetch: bool) {
    if !fetch {
        // The entry fetch already ran; an internal reload should not hit the
        // network again.
        options.no_fetch = true;
    }

    thread::spawn(move || {
        let progress = TuiProgress::new(tx.clone());

        let result = plan::load(&ctx, &options, &progress).map(|inputs| {
            (
                Arc::clone(&ctx),
                inputs,
                ctx.git.local_branches().unwrap_or_default(),
            )
        });

        let _ = tx.send(Msg::Loaded(result));
    });
}

fn spawn_apply(tx: Sender<Msg>, ctx: Arc<Context>, plan: Plan) {
    thread::spawn(move || {
        let progress = TuiProgress::new(tx.clone());
        let result = apply::apply(&plan, &ctx, &progress);

        let _ = tx.send(Msg::Applied(result));
    });
}

fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Action::Quit;
    }

    // Expands the shortcut band when it does not fit on one line. Works in every
    // mode so the rest of the keys are always reachable.
    if key.code == KeyCode::Tab {
        app.toggle_keys();
        return Action::None;
    }

    match app.mode {
        Mode::Help => {
            app.mode = Mode::Editing;
            Action::None
        }
        Mode::Summary => {
            let mut reload = false;

            match key.code {
                KeyCode::Enter => reload = app.commit_summary(),
                KeyCode::Esc => app.mode = Mode::Editing,
                KeyCode::Backspace => {
                    app.summary_buffer.pop();
                }
                KeyCode::Char(character) if app.summary_buffer.chars().count() < 120 => {
                    app.summary_buffer.push(character);
                }
                _ => {}
            }

            if reload {
                Action::ReloadLocal
            } else {
                Action::None
            }
        }
        Mode::PickBranch => {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => app.move_picker(1),
                KeyCode::Char('k') | KeyCode::Up => app.move_picker(-1),
                KeyCode::Enter => {
                    if let Some(branch) = app.selected_branch() {
                        app.options.branch = Some(branch);
                        app.mode = Mode::Editing;
                        return Action::Reload;
                    }
                }
                KeyCode::Esc => app.mode = Mode::Editing,
                _ => {}
            }

            Action::None
        }
        Mode::Confirming => match key.code {
            KeyCode::Char('y') | KeyCode::Enter => Action::Apply,
            KeyCode::Char('n') | KeyCode::Esc => {
                app.mode = Mode::Editing;
                Action::None
            }
            _ => Action::None,
        },
        Mode::Applying | Mode::Loading => Action::None,
        Mode::Done => match key.code {
            KeyCode::Char('o') => {
                if let Some(url) = &app.release_url {
                    return Action::OpenUrl(url.clone());
                }

                Action::None
            }
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter => Action::Quit,
            _ => Action::None,
        },
        Mode::Failed => match key.code {
            KeyCode::Char('r') => Action::Reload,
            KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
            _ => Action::None,
        },
        Mode::Editing => match key.code {
            KeyCode::Char('q') => Action::Quit,
            KeyCode::Char('j') | KeyCode::Down => {
                app.move_focus(1);
                Action::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                app.move_focus(-1);
                Action::None
            }
            KeyCode::Char('J') => {
                app.scroll_preview(1);
                Action::None
            }
            KeyCode::Char('K') => {
                app.scroll_preview(-1);
                Action::None
            }
            // Numeric shortcuts follow the order patch → minor → major → exclude.
            KeyCode::Char('p') | KeyCode::Char('1') => {
                app.assign("patch");
                Action::None
            }
            KeyCode::Char('i') | KeyCode::Char('2') => {
                app.assign("minor");
                Action::None
            }
            KeyCode::Char('m') | KeyCode::Char('3') => {
                app.assign("major");
                Action::None
            }
            KeyCode::Char('x') | KeyCode::Char('4') | KeyCode::Char('0') => {
                app.assign("ignore");
                Action::None
            }
            KeyCode::Char('t') => {
                if app.toggle_tag_only() {
                    Action::ReloadLocal
                } else {
                    Action::None
                }
            }
            KeyCode::Char('v') => {
                app.cycle_release_type();
                // Only the commit range changes, so no need to hit the network.
                Action::ReloadLocal
            }
            KeyCode::Char('d') => {
                app.toggle_dry_run();
                Action::None
            }
            KeyCode::Char('f') => {
                app.toggle_draft();
                Action::None
            }
            KeyCode::Char('o') => {
                app.toggle_overwrite();
                Action::None
            }
            KeyCode::Char('S') => {
                app.open_summary();
                Action::None
            }
            KeyCode::Char('b') => {
                app.open_picker();
                Action::None
            }
            KeyCode::Char('e') => Action::Edit,
            KeyCode::Char('r') => Action::Reload,
            KeyCode::Char('?') => {
                app.mode = Mode::Help;
                Action::None
            }
            KeyCode::Esc => Action::Quit,
            KeyCode::Enter => {
                app.finalize();

                if app.plan.is_some() && app.error.is_none() {
                    app.mode = Mode::Confirming;
                }

                Action::None
            }
            _ => Action::None,
        },
    }
}

fn edit_in_editor(current: &str) -> Option<String> {
    let editor = release_core::config::default_editor();
    let path = std::env::temp_dir().join(format!("release-notes-{}.md", std::process::id()));

    std::fs::write(&path, current).ok()?;

    let mut parts = editor.split_whitespace();
    let program = parts.next()?;

    let status = std::process::Command::new(program)
        .args(parts)
        .arg(&path)
        .status()
        .ok()?;

    let edited = if status.success() {
        std::fs::read_to_string(&path).ok()
    } else {
        None
    };

    let _ = std::fs::remove_file(&path);

    edited.filter(|text| !text.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn v_cycles_the_version_and_reloads_without_fetching() {
        let mut app = App::new(Options::default());
        app.mode = Mode::Editing;

        let action = handle_key(&mut app, press(KeyCode::Char('v')));

        assert!(matches!(action, Action::ReloadLocal));
        assert_eq!(app.options.release_type.as_deref(), Some("patch"));
    }

    #[test]
    fn number_keys_classify_the_focused_commit() {
        let cases = [
            (KeyCode::Char('1'), "patch"),
            (KeyCode::Char('p'), "patch"),
            (KeyCode::Char('2'), "minor"),
            (KeyCode::Char('i'), "minor"),
            (KeyCode::Char('3'), "major"),
            (KeyCode::Char('m'), "major"),
            (KeyCode::Char('4'), "ignore"),
            (KeyCode::Char('x'), "ignore"),
            (KeyCode::Char('0'), "ignore"),
        ];

        for (key, expected) in cases {
            let mut app = App::new(Options::default());
            app.loaded(
                crate::app::fixture_inputs(&[("aaa", "Do a thing")]),
                Vec::new(),
            );
            app.mode = Mode::Editing;

            handle_key(&mut app, press(key));

            assert_eq!(
                app.rows[0].change_type, expected,
                "key {key:?} should classify as {expected}"
            );
        }
    }

    #[test]
    fn uppercase_r_is_no_longer_a_shortcut() {
        let mut app = App::new(Options::default());
        app.mode = Mode::Editing;

        let action = handle_key(&mut app, press(KeyCode::Char('R')));

        assert!(matches!(action, Action::None));
        assert_eq!(app.options.release_type, None);
    }
}
