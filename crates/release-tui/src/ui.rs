//! Rendering. 16-colour ANSI only, so the TUI follows the terminal palette.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, Mode};

pub fn draw(frame: &mut Frame, app: &App) {
    let width = frame.area().width;

    let status = StatusView::build(app, width);
    let keybar = KeybarView::build(app, width);

    let status_height = if status.same_line { 3 } else { 4 };
    let keybar_height = keybar.height();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(status_height),
            Constraint::Length(keybar_height),
        ])
        .split(frame.area());

    draw_main(frame, app, chunks[0]);
    draw_status(frame, app, chunks[1], &status);
    draw_keybar(frame, &keybar, chunks[2]);

    match app.mode {
        Mode::Help => draw_help(frame),
        Mode::PickBranch => draw_branch_picker(frame, app),
        Mode::Confirming => draw_confirmation(frame, app),
        _ => {}
    }
}

/// The tag/type row, with the flags pushed to the opposite edge when the
/// terminal is wide enough. Narrow terminals fall back to a second row rather
/// than truncating the values.
struct StatusView {
    tag_spans: Vec<Span<'static>>,
    tag_width: usize,
    flags_spans: Vec<Span<'static>>,
    flags_width: usize,
    same_line: bool,
}

impl StatusView {
    fn build(app: &App, width: u16) -> Self {
        let label = Style::default().fg(Color::DarkGray);
        let value = Style::default().fg(Color::Cyan);

        let tag = app.tag_display();
        let release_type = app
            .options
            .release_type
            .clone()
            .unwrap_or_else(|| "release".to_string());

        let tag_width =
            "tag  ".len() + tag.chars().count() + "   type  ".len() + release_type.chars().count();

        let tag_spans = vec![
            Span::styled("tag  ", label),
            Span::styled(tag, value),
            Span::styled("   type  ", label),
            Span::styled(release_type, value),
        ];

        let flags_spans = flags_spans(app);
        let flags_width = "flags  ".len() + app.flags_line().chars().count();

        Self {
            tag_spans,
            tag_width,
            flags_spans,
            flags_width,
            same_line: usize::from(width) >= tag_width + 2 + flags_width,
        }
    }
}

/// Flag labels and their values get the same two colours as the tag row.
fn flags_spans(app: &App) -> Vec<Span<'static>> {
    let label = Style::default().fg(Color::DarkGray);
    let value = Style::default().fg(Color::Cyan);

    let mut spans = Vec::new();

    for (index, (name, state)) in app.flag_pairs().into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" • ", label));
        }

        spans.push(Span::styled(format!("{name} "), label));
        spans.push(Span::styled(state.to_string(), value));
    }

    spans
}

fn draw_main(frame: &mut Frame, app: &App, area: Rect) {
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    draw_commits(frame, app, panes[0]);
    draw_preview(frame, app, panes[1]);
}

fn draw_commits(frame: &mut Frame, app: &App, area: Rect) {
    let branch = app
        .inputs
        .as_ref()
        .map(|inputs| inputs.branch.clone())
        .unwrap_or_else(|| "…".to_string());

    let block = Block::default().borders(Borders::ALL).title(format!(
        " {} · {} commits ",
        branch,
        app.rows.len()
    ));

    let items: Vec<ListItem> = app
        .rows
        .iter()
        .map(|row| {
            let style = match row.change_type.as_str() {
                "major" => Style::default().fg(Color::Red),
                "minor" => Style::default().fg(Color::Yellow),
                "patch" => Style::default().fg(Color::Green),
                "ignore" => Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::CROSSED_OUT),
                _ => Style::default(),
            };

            let short: String = row.hash.chars().take(7).collect();

            ListItem::new(Line::from(vec![
                Span::styled(format!("{:<7}", row.change_type), style),
                Span::styled(format!(" {short} "), Style::default().fg(Color::DarkGray)),
                Span::raw(row.title.clone()),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("› ");

    let mut state = ListState::default();
    state.select(Some(app.focus));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_preview(frame: &mut Frame, app: &App, area: Rect) {
    let body = app
        .plan
        .as_ref()
        .map(|plan| plan.changelog.clone())
        .unwrap_or_else(|| "Nothing to preview yet.".to_string());

    let preview = Paragraph::new(body)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Release notes "),
        )
        .wrap(Wrap { trim: false })
        .scroll((app.preview_scroll, 0));

    frame.render_widget(preview, area);
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect, status: &StatusView) {
    let mut lines: Vec<Line> = Vec::new();

    let mode_style = match app.mode {
        Mode::Failed => Style::default().fg(Color::Red),
        Mode::Done => Style::default().fg(Color::Green),
        Mode::Applying => Style::default().fg(Color::Yellow),
        _ => Style::default().fg(Color::Cyan),
    };

    lines.push(Line::from(vec![
        Span::styled(format!("{:<9}", app.mode.label()), mode_style),
        Span::raw(app.status.clone()),
    ]));

    let label = Style::default().fg(Color::DarkGray);

    let mut tag_line = status.tag_spans.clone();

    if status.same_line {
        let gap = usize::from(area.width).saturating_sub(status.tag_width + status.flags_width);

        tag_line.push(Span::raw(" ".repeat(gap)));
        tag_line.push(Span::styled("flags  ", label));
        tag_line.extend(status.flags_spans.clone());
    }

    lines.push(Line::from(tag_line));

    if !status.same_line {
        let mut flags_line = vec![Span::styled("flags  ", label)];
        flags_line.extend(status.flags_spans.clone());
        lines.push(Line::from(flags_line));
    }

    if let Some(error) = &app.error {
        lines.push(Line::from(Span::styled(
            error.message.clone(),
            Style::default().fg(Color::Red),
        )));
    } else if let Some(warning) = app.warnings.first() {
        lines.push(Line::from(Span::styled(
            warning.message.clone(),
            Style::default().fg(Color::Yellow),
        )));
    } else if app.mode == Mode::Done {
        let url = app.release_url.clone().unwrap_or_default();
        lines.push(Line::from(Span::styled(
            url,
            Style::default().fg(Color::Green),
        )));
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

/// The shortcut band. Never truncates mid-item: when the items do not fit, the
/// line ends with a `… more` indicator and `tab` reveals the remainder, wrapped
/// over as many lines as it needs.
struct KeybarView {
    first: String,
    rest: Vec<String>,
}

impl KeybarView {
    fn build(app: &App, width: u16) -> Self {
        let items = keybar_items(app.mode);
        let (first, rest) = layout_keybar(&items, usize::from(width), app.keys_expanded);

        Self { first, rest }
    }

    fn height(&self) -> u16 {
        1 + self.rest.len() as u16
    }
}

fn keybar_items(mode: Mode) -> Vec<&'static str> {
    match mode {
        Mode::Summary => vec!["enter save", "esc cancel", "backspace delete"],
        Mode::Confirming => vec!["y apply", "esc cancel"],
        Mode::PickBranch => vec!["j/k choose", "enter switch", "esc cancel"],
        Mode::Help => vec!["any key to close"],
        Mode::Done => vec!["o open", "q quit"],
        Mode::Failed => vec!["r retry", "q quit"],
        Mode::Applying => vec!["working…"],
        _ => vec![
            "m/i/p/x or 1-4 classify",
            "v version bump",
            "t tag-only",
            "d dry-run",
            "f draft on/off",
            "o overwrite",
            "S summary",
            "b branch",
            "e edit",
            "r reload",
            "? help",
            "enter release",
            "q quit",
        ],
    }
}

const MORE: &str = "… more (tab)";
const LESS: &str = "… less (tab)";

/// Fits as many whole items as possible. `expanded` switches the indicator from
/// "more" to "less" and returns every remaining item, wrapped into lines.
fn layout_keybar(items: &[&str], width: usize, expanded: bool) -> (String, Vec<String>) {
    let joined = items.join(" · ");

    if joined.chars().count() <= width {
        return (joined, Vec::new());
    }

    let indicator = if expanded { LESS } else { MORE };
    let reserve = 3 + indicator.chars().count();

    let mut chosen: Vec<&str> = Vec::new();
    let mut used = 0usize;

    for item in items {
        let extra = if chosen.is_empty() {
            item.chars().count()
        } else {
            3 + item.chars().count()
        };

        if used + extra + reserve > width {
            break;
        }

        used += extra;
        chosen.push(item);
    }

    let mut first = chosen.join(" · ");

    if !first.is_empty() {
        first.push_str(" · ");
    }

    first.push_str(indicator);
    let first = clamp(first, width);

    if !expanded {
        return (first, Vec::new());
    }

    (first, wrap_items(&items[chosen.len()..], width))
}

/// Wraps items without ever splitting one, so an expanded band can take more
/// than two lines when the terminal is narrow.
fn wrap_items(items: &[&str], width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();

    for item in items {
        if current.is_empty() {
            current = (*item).to_string();
        } else if current.chars().count() + 3 + item.chars().count() <= width {
            current.push_str(" · ");
            current.push_str(item);
        } else {
            lines.push(clamp(current, width));
            current = (*item).to_string();
        }
    }

    if !current.is_empty() {
        lines.push(clamp(current, width));
    }

    lines
}

/// Hard limit for pathological widths, so a line can never be cut mid-word by
/// the renderer without saying so.
fn clamp(text: String, width: usize) -> String {
    if text.chars().count() <= width {
        return text;
    }

    let mut shortened: String = text.chars().take(width.saturating_sub(1)).collect();
    shortened.push('…');
    shortened
}

fn draw_keybar(frame: &mut Frame, keybar: &KeybarView, area: Rect) {
    let style = Style::default().fg(Color::DarkGray);

    let mut lines = vec![Line::from(Span::styled(keybar.first.clone(), style))];

    for line in &keybar.rest {
        lines.push(Line::from(Span::styled(line.clone(), style)));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_help(frame: &mut Frame) {
    let area = centered(70, 70, frame.area());
    frame.render_widget(Clear, area);

    let lines = vec![
        Line::from("release — interactive release"),
        Line::from(""),
        Line::from("j / k        move between commits"),
        Line::from("m i p        major / minor / patch"),
        Line::from("1 2 3 4      patch / minor / major / exclude"),
        Line::from("x            exclude the commit"),
        Line::from("v            cycle the version bump (none/patch/minor/major/pre)"),
        Line::from("t            toggle annotated tag only"),
        Line::from("d            toggle dry run"),
        Line::from("f            draft on/off"),
        Line::from("o            toggle overwrite of an existing release"),
        Line::from("S            edit the update summary"),
        Line::from("b            switch branch"),
        Line::from("e            edit the release notes in $EDITOR"),
        Line::from("J / K        scroll the preview"),
        Line::from("r            reload from git"),
        Line::from("enter        review the plan and apply"),
        Line::from("tab          expand the shortcut band"),
        Line::from("q            quit"),
    ];

    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Help "))
        .wrap(Wrap { trim: true });

    frame.render_widget(paragraph, area);
}

fn draw_branch_picker(frame: &mut Frame, app: &App) {
    let area = centered(50, 60, frame.area());
    frame.render_widget(Clear, area);

    let items: Vec<ListItem> = app
        .branches
        .iter()
        .map(|branch| ListItem::new(branch.clone()))
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Switch branch "),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("› ");

    let mut state = ListState::default();
    state.select(Some(app.picker_index));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_confirmation(frame: &mut Frame, app: &App) {
    let area = centered(70, 60, frame.area());
    frame.render_widget(Clear, area);

    let Some(plan) = &app.plan else {
        return;
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled("tag     ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                plan.tag.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("branch  ", Style::default().fg(Color::DarkGray)),
            Span::raw(plan.branch.clone()),
        ]),
        Line::from(vec![
            Span::styled("mode    ", Style::default().fg(Color::DarkGray)),
            Span::raw(if plan.dry_run {
                "dry run (no changes)".to_string()
            } else {
                format!(
                    "{} · {}",
                    if plan.tag_only {
                        "annotated tag"
                    } else {
                        "GitHub Release"
                    },
                    if plan.draft { "draft" } else { "published" }
                )
            }),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "effects",
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ];

    for effect in &plan.effects {
        lines.push(Line::from(format!("  · {}", effect.kind.describe())));
    }

    for warning in &plan.warnings {
        lines.push(Line::from(Span::styled(
            format!("! {}", warning.message),
            Style::default().fg(Color::Yellow),
        )));
    }

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Confirm release "),
        )
        .wrap(Wrap { trim: true });

    frame.render_widget(paragraph, area);
}

fn centered(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    const ITEMS: &[&str] = &["alpha", "bravo", "charlie", "delta"];

    #[test]
    fn a_wide_band_shows_everything() {
        let (first, rest) = layout_keybar(ITEMS, 120, false);

        assert_eq!(first, "alpha · bravo · charlie · delta");
        assert!(rest.is_empty());
    }

    #[test]
    fn a_narrow_band_ends_with_the_more_indicator() {
        let (first, rest) = layout_keybar(ITEMS, 30, false);

        assert!(first.ends_with("… more (tab)"), "got {first:?}");
        assert!(first.starts_with("alpha"), "keeps whole items only");
        assert!(!first.contains("charlie"), "does not truncate mid-item");
        assert!(rest.is_empty());
    }

    #[test]
    fn expanding_reveals_every_remaining_item() {
        let (first, rest) = layout_keybar(ITEMS, 30, true);

        assert!(first.ends_with("… less (tab)"), "got {first:?}");

        let rest = rest.join(" · ");
        assert!(rest.contains("charlie"), "got {rest:?}");
        assert!(rest.contains("delta"), "got {rest:?}");
    }

    #[test]
    fn expanding_wraps_onto_more_lines_when_needed() {
        let many: Vec<&str> = vec!["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"];
        let (_, rest) = layout_keybar(&many, 24, true);

        assert!(rest.len() > 1, "got {rest:?}");
        assert!(rest.iter().all(|line| line.chars().count() <= 24));
    }

    #[test]
    fn a_pathological_width_is_clamped_not_cut() {
        let (first, rest) = layout_keybar(ITEMS, 12, false);

        assert!(first.chars().count() <= 12, "got {first:?}");
        assert!(rest.is_empty());
    }
}
