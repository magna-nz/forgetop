//! All rendering. Immediate-mode: we redraw the whole frame from `App` each tick.

use chrono::{DateTime, Utc};
use forgetop_core::domain::*;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table, Tabs, Wrap};
use ratatui::Frame;

use crate::app::{App, PipeRow, TABS};
use crate::overlay::Overlay;
use crate::theme::{check_icon, pipeline_icon, Theme};

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let theme = &app.theme;

    // Base background.
    frame.render_widget(Block::default().style(Style::default().bg(theme.bg).fg(theme.fg)), area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // tab bar
            Constraint::Min(3),    // content
            Constraint::Length(1), // connections
            Constraint::Length(1), // footer
        ])
        .split(area);

    render_tabs(frame, rows[0], app);
    render_content(frame, rows[1], app);
    render_health(frame, rows[2], app);
    render_footer(frame, rows[3], app);

    if app.overlay.is_some() {
        render_overlay(frame, area, app);
    }
}

fn render_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let clock = app.last_refresh.format("%H:%M:%S");
    let right = format!("{} · {}{} ", theme.name, if app.loading { "⟳ " } else { "" }, clock);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.bg))
        .title(Span::styled(" ▟ forgetop ", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)))
        .title_top(Line::from(Span::styled(right, Style::default().fg(theme.dim))).right_aligned());

    let titles: Vec<Line> = TABS
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let count = match i {
                0 => app.prs.len(),
                1 => app.wis.len(),
                _ => app.pipes.len(),
            };
            Line::from(format!(" {t} {count} "))
        })
        .collect();

    let tabs = Tabs::new(titles)
        .select(app.active)
        .divider(Span::styled("  ", Style::default().fg(theme.dim)))
        .padding("", "")
        .style(Style::default().fg(theme.dim).bg(theme.bg))
        .highlight_style(Style::default().fg(theme.bg).bg(theme.accent).add_modifier(Modifier::BOLD))
        .block(block);

    frame.render_widget(tabs, area);
}

fn render_content(frame: &mut Frame, area: Rect, app: &mut App) {
    if app.show_detail && app.selected().is_some() {
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(12)])
            .split(area);
        render_table(frame, split[0], app);
        render_detail(frame, split[1], app);
    } else {
        render_table(frame, area, app);
    }
}

fn render_table(frame: &mut Frame, area: Rect, app: &mut App) {
    match app.active {
        0 => render_prs(frame, area, app),
        1 => render_wis(frame, area, app),
        _ => render_pipes(frame, area, app),
    }
}

fn section_block<'a>(theme: &Theme, title: &'a str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.bg))
        .title(Span::styled(format!(" {title} "), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)))
}

fn header_row(theme: &Theme, cols: &[&'static str]) -> Row<'static> {
    Row::new(cols.iter().map(|c| Cell::from(*c)).collect::<Vec<_>>())
        .style(Style::default().fg(theme.dim).add_modifier(Modifier::BOLD))
}

fn empty(frame: &mut Frame, area: Rect, theme: &Theme, msg: &str, block: Block) {
    let p = Paragraph::new(Line::from(Span::styled(msg, Style::default().fg(theme.dim))))
        .block(block)
        .wrap(Wrap { trim: true });
    frame.render_widget(p, area);
}

fn highlight(theme: &Theme) -> Style {
    Style::default().bg(theme.sel_bg).add_modifier(Modifier::BOLD)
}

// ---- Pull Requests ----

fn pr_status_span(theme: &Theme, pr: &PullRequest) -> Span<'static> {
    let (icon, color) = if pr.is_draft {
        ("◌ draft", theme.dim)
    } else {
        match pr.status {
            PullRequestStatus::Open => ("● open", theme.green),
            PullRequestStatus::Merged => ("✦ merged", theme.magenta),
            PullRequestStatus::Closed => ("✗ closed", theme.red),
            PullRequestStatus::Draft => ("◌ draft", theme.dim),
        }
    };
    Span::styled(icon, Style::default().fg(color))
}

fn checks_span(theme: &Theme, pr: &PullRequest) -> Span<'static> {
    let text = match &pr.check_summary {
        Some(s) => format!("{} {}/{}", check_icon(pr.checks), s.successful, s.successful + s.failed + s.in_progress + s.neutral),
        None => format!("{} —", check_icon(pr.checks)),
    };
    Span::styled(text, Style::default().fg(theme.check_color(pr.checks)))
}

fn render_prs(frame: &mut Frame, area: Rect, app: &mut App) {
    let theme = &app.theme;
    let title = format!("Pull Requests · {}", app.pr_filter_label());
    let block = section_block(theme, &title);
    if app.prs.is_empty() {
        let msg = if app.loading { "Loading pull requests…" } else { "No pull requests. Press f to change filter, r to refresh." };
        empty(frame, area, theme, msg, block);
        return;
    }

    let header = header_row(theme, &["", "#", "Title", "Author", "Checks", "±", "Updated"]);
    let rows: Vec<Row> = app
        .prs
        .iter()
        .map(|pr| {
            let num = pr.number.map(|n| format!("#{n}")).unwrap_or_default();
            let diff = Span::styled(
                format!("+{} -{}", pr.additions, pr.deletions),
                Style::default().fg(theme.dim),
            );
            Row::new(vec![
                Cell::from(pr_status_span(theme, pr)),
                Cell::from(Span::styled(num, Style::default().fg(theme.dim))),
                Cell::from(Span::styled(pr.title.clone(), Style::default().fg(theme.fg))),
                Cell::from(Span::styled(pr.author.display_name.clone(), Style::default().fg(theme.blue))),
                Cell::from(checks_span(theme, pr)),
                Cell::from(diff),
                Cell::from(Span::styled(rel_age(pr.updated_at), Style::default().fg(theme.dim))),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(9),
        Constraint::Length(7),
        Constraint::Min(24),
        Constraint::Length(16),
        Constraint::Length(9),
        Constraint::Length(11),
        Constraint::Length(8),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1)
        .row_highlight_style(highlight(theme))
        .highlight_symbol("▐ ");
    frame.render_stateful_widget(table, area, &mut app.pr_state);
}

// ---- Work Items ----

fn wi_state_color(theme: &Theme, cat: WorkItemStateCategory) -> ratatui::style::Color {
    match cat {
        WorkItemStateCategory::Completed => theme.green,
        WorkItemStateCategory::Started => theme.blue,
        WorkItemStateCategory::Canceled => theme.red,
        WorkItemStateCategory::Triage => theme.yellow,
        _ => theme.dim,
    }
}

fn render_wis(frame: &mut Frame, area: Rect, app: &mut App) {
    let theme = &app.theme;
    let block = section_block(theme, "Work Items");
    if app.wis.is_empty() {
        let msg = if app.loading { "Loading work items…" } else { "No work items. Press r to refresh." };
        empty(frame, area, theme, msg, block);
        return;
    }

    let header = header_row(theme, &["State", "ID", "Title", "Type", "Assignee", "Updated"]);
    let rows: Vec<Row> = app
        .wis
        .iter()
        .map(|wi| {
            Row::new(vec![
                Cell::from(Span::styled(format!("● {}", wi.state), Style::default().fg(wi_state_color(theme, wi.state_category)))),
                Cell::from(Span::styled(wi.identifier.clone().unwrap_or_default(), Style::default().fg(theme.dim))),
                Cell::from(Span::styled(wi.title.clone(), Style::default().fg(theme.fg))),
                Cell::from(Span::styled(wi.work_item_type.clone().unwrap_or_default(), Style::default().fg(theme.dim))),
                Cell::from(Span::styled(
                    wi.assignee.as_ref().map(|a| a.display_name.clone()).unwrap_or_else(|| "—".into()),
                    Style::default().fg(theme.blue),
                )),
                Cell::from(Span::styled(rel_age(wi.updated_at), Style::default().fg(theme.dim))),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(16),
        Constraint::Length(10),
        Constraint::Min(24),
        Constraint::Length(12),
        Constraint::Length(16),
        Constraint::Length(8),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1)
        .row_highlight_style(highlight(theme))
        .highlight_symbol("▐ ");
    frame.render_stateful_widget(table, area, &mut app.wi_state);
}

// ---- Pipelines ----

fn render_pipes(frame: &mut Frame, area: Rect, app: &mut App) {
    let theme = &app.theme;
    let block = section_block(theme, "Pipelines");
    if app.pipes.is_empty() {
        let msg = if app.loading { "Loading pipeline runs…" } else { "No pipeline runs. Press r to refresh." };
        empty(frame, area, theme, msg, block);
        return;
    }

    let header = header_row(theme, &["", "Provider", "Pipeline", "#", "Branch", "Started"]);
    let rows: Vec<Row> = app
        .pipes
        .iter()
        .map(|p| {
            let color = theme.pipeline_color(p.run.status);
            let name = p.run.name.clone().unwrap_or_else(|| p.run.definition_id.clone());
            let num = p.run.number.map(|n| format!("#{n}")).unwrap_or_default();
            Row::new(vec![
                Cell::from(Span::styled(format!("{} {:?}", pipeline_icon(p.run.status), p.run.status), Style::default().fg(color))),
                Cell::from(Span::styled(format!("{} · {}", p.provider.as_str(), p.connection), Style::default().fg(theme.cyan))),
                Cell::from(Span::styled(name, Style::default().fg(theme.fg))),
                Cell::from(Span::styled(num, Style::default().fg(theme.dim))),
                Cell::from(Span::styled(p.run.branch.clone().unwrap_or_default(), Style::default().fg(theme.dim))),
                Cell::from(Span::styled(rel_age(p.run.started_at), Style::default().fg(theme.dim))),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(20),
        Constraint::Length(22),
        Constraint::Min(20),
        Constraint::Length(7),
        Constraint::Length(18),
        Constraint::Length(8),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1)
        .row_highlight_style(highlight(theme))
        .highlight_symbol("▐ ");
    frame.render_stateful_widget(table, area, &mut app.pipe_state);
}

// ---- detail panel ----

fn render_detail(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let Some(idx) = app.selected() else { return };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.magenta))
        .style(Style::default().bg(theme.panel))
        .title(Span::styled(" ▼ Detail — Esc to close ", Style::default().fg(theme.magenta).add_modifier(Modifier::BOLD)));

    let lines: Vec<Line> = match app.active {
        0 => app.prs.get(idx).map(|pr| pr_detail(theme, pr)).unwrap_or_default(),
        1 => app.wis.get(idx).map(|wi| wi_detail(theme, wi)).unwrap_or_default(),
        _ => app.pipes.get(idx).map(|p| pipe_detail(theme, p)).unwrap_or_default(),
    };

    let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    frame.render_widget(p, area);
}

fn field<'a>(theme: &Theme, label: &'a str, value: String) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{label:<11}"), Style::default().fg(theme.dim)),
        Span::styled(value, Style::default().fg(theme.fg)),
    ])
}

fn pr_detail(theme: &Theme, pr: &PullRequest) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(pr.title.clone(), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))),
        field(theme, "Author", pr.author.display_name.clone()),
        field(theme, "Branch", format!("{} → {}", pr.source_ref.clone().unwrap_or_default(), pr.target_ref.clone().unwrap_or_default())),
        field(theme, "Mergeable", format!("{:?}", pr.mergeable)),
        field(theme, "Changes", format!("{} files  +{} -{}", pr.changed_files, pr.additions, pr.deletions)),
    ];
    if !pr.reviewers.is_empty() {
        let who = pr.reviewers.iter().map(|r| format!("{} ({:?})", r.user.display_name, r.vote)).collect::<Vec<_>>().join(", ");
        lines.push(field(theme, "Reviewers", who));
    }
    if !pr.labels.is_empty() {
        lines.push(field(theme, "Labels", pr.labels.join(", ")));
    }
    if let Some(url) = &pr.url {
        lines.push(field(theme, "URL", url.clone()));
    }
    if let Some(desc) = &pr.description {
        if !desc.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(desc.clone(), Style::default().fg(theme.dim))));
        }
    }
    lines
}

fn wi_detail(theme: &Theme, wi: &WorkItem) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(
            format!("{} {}", wi.identifier.clone().unwrap_or_default(), wi.title),
            Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
        )),
        field(theme, "State", format!("{} ({:?})", wi.state, wi.state_category)),
        field(theme, "Type", wi.work_item_type.clone().unwrap_or_else(|| "—".into())),
        field(theme, "Assignee", wi.assignee.as_ref().map(|a| a.display_name.clone()).unwrap_or_else(|| "—".into())),
    ];
    if let Some(url) = &wi.url {
        lines.push(field(theme, "URL", url.clone()));
    }
    if let Some(desc) = &wi.description {
        if !desc.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(desc.clone(), Style::default().fg(theme.dim))));
        }
    }
    lines
}

fn pipe_detail(theme: &Theme, p: &PipeRow) -> Vec<Line<'static>> {
    let name = p.run.name.clone().unwrap_or_else(|| p.run.definition_id.clone());
    let mut lines = vec![
        Line::from(Span::styled(name, Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))),
        field(theme, "Status", format!("{:?}", p.run.status)),
        field(theme, "Provider", format!("{} · {}", p.provider.as_str(), p.connection)),
        field(theme, "Branch", p.run.branch.clone().unwrap_or_default()),
        field(theme, "Triggered", p.run.triggered_by.as_ref().map(|u| u.display_name.clone()).unwrap_or_else(|| "—".into())),
    ];
    for stage in &p.run.stages {
        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", pipeline_icon(stage.status)), Style::default().fg(theme.pipeline_color(stage.status))),
            Span::styled(stage.name.clone(), Style::default().fg(theme.fg)),
            Span::styled(format!("  ({} jobs)", stage.jobs.len()), Style::default().fg(theme.dim)),
        ]));
    }
    lines
}

// ---- connections + footer ----

fn render_health(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let mut spans = vec![Span::styled("connections  ", Style::default().fg(theme.dim))];
    if app.health.is_empty() {
        spans.push(Span::styled("none configured — run the setup wizard", Style::default().fg(theme.dim)));
    }
    for h in &app.health {
        let (icon, color) = if h.healthy { ("●", theme.green) } else { ("○", theme.red) };
        spans.push(Span::styled(format!("{icon} "), Style::default().fg(color)));
        spans.push(Span::styled(format!("{}  ", h.connection.display_name), Style::default().fg(theme.fg)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Context-aware key glossary for the active tab (azdo-style bar along the bottom).
/// While an overlay is open it shows that overlay's own keys instead.
fn footer_keys(app: &App) -> Vec<(&'static str, &'static str)> {
    if let Some(overlay) = &app.overlay {
        return overlay.hint();
    }
    let mut keys = vec![("↑↓", "move"), ("←→", "tabs"), ("↵", "detail")];
    if app.active == 0 {
        keys.extend([("f", "filter"), ("a", "approve"), ("x", "reject"), ("m", "merge"), ("c", "comment")]);
    }
    keys.extend([("r", "refresh"), ("t", "theme"), ("q", "quit")]);
    keys
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    // A subtle bar background so the glossary reads as a distinct strip.
    let bar = Style::default().bg(theme.panel);

    let mut spans = vec![Span::styled(" ", bar)];
    for (key, label) in footer_keys(app) {
        spans.push(Span::styled(format!(" {key} "), bar.fg(theme.bg).bg(theme.accent).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(format!(" {label}  "), bar.fg(theme.fg)));
    }

    // Right side: transient toast (highlighted) or the standing status line.
    let (right, right_style) = match &app.toast {
        Some(t) => (format!("{t} "), bar.fg(theme.yellow).add_modifier(Modifier::BOLD)),
        None => (format!("{} ", app.status), bar.fg(theme.dim)),
    };
    let right_w = right.chars().count().min(70) as u16 + 1;

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(10), Constraint::Length(right_w)])
        .split(area);

    frame.render_widget(Paragraph::new(Line::from(spans)).style(bar), cols[0]);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(right, right_style)).right_aligned()).style(bar),
        cols[1],
    );
}

// ---- overlays ----

fn render_overlay(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let Some(overlay) = &app.overlay else { return };

    let (body, hint_color): (Vec<Line>, _) = match overlay {
        Overlay::Confirm { message, .. } => (
            vec![Line::from(""), Line::from(Span::styled(message.clone(), Style::default().fg(theme.fg)))],
            theme.yellow,
        ),
        Overlay::Picker { items, selected, .. } => (
            items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    if i == *selected {
                        Line::from(vec![
                            Span::styled(" ▐ ", Style::default().fg(theme.accent)),
                            Span::styled(item.clone(), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
                        ])
                    } else {
                        Line::from(Span::styled(format!("   {item}"), Style::default().fg(theme.fg)))
                    }
                })
                .collect(),
            theme.accent,
        ),
        Overlay::Input { buffer, .. } => (
            vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("> ", Style::default().fg(theme.accent)),
                    Span::styled(buffer.clone(), Style::default().fg(theme.fg)),
                    Span::styled("█", Style::default().fg(theme.accent)),
                ]),
            ],
            theme.green,
        ),
    };

    let hint = footer_keys(app)
        .into_iter()
        .flat_map(|(k, l)| {
            [
                Span::styled(format!(" {k} "), Style::default().fg(theme.bg).bg(hint_color).add_modifier(Modifier::BOLD)),
                Span::styled(format!(" {l}   "), Style::default().fg(theme.dim)),
            ]
        })
        .collect::<Vec<_>>();

    let mut lines = body;
    lines.push(Line::from(""));
    lines.push(Line::from(hint));

    let height = lines.len() as u16 + 2;
    let width = 64.min(area.width.saturating_sub(6));
    let rect = centered_rect(width, height, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(hint_color))
        .style(Style::default().bg(theme.panel))
        .title(Span::styled(format!(" {} ", overlay.title()), Style::default().fg(hint_color).add_modifier(Modifier::BOLD)));

    frame.render_widget(Clear, rect);
    frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), rect);
}

/// A rectangle of the given size, centred within `area`.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect { x, y, width: width.min(area.width), height: height.min(area.height) }
}

// ---- helpers ----

fn rel_age(ts: Option<DateTime<Utc>>) -> String {
    let Some(ts) = ts else { return "—".into() };
    let secs = (Utc::now() - ts).num_seconds().max(0);
    match secs {
        s if s < 60 => "now".into(),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86400 => format!("{}h", s / 3600),
        s if s < 2_592_000 => format!("{}d", s / 86400),
        s => format!("{}mo", s / 2_592_000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn sample_pr() -> PullRequest {
        PullRequest {
            id: "1".into(),
            number: Some(42),
            title: "Add the widget".into(),
            description: Some("does the thing".into()),
            author: User { id: "u".into(), display_name: "Alice Ng".into(), handle: None, avatar_url: None },
            status: PullRequestStatus::Open,
            is_draft: false,
            source_ref: Some("feat".into()),
            target_ref: Some("main".into()),
            reviewers: vec![],
            labels: vec!["backend".into()],
            checks: CheckStatus::Passed,
            check_summary: None,
            mergeable: MergeableState::Mergeable,
            changed_files: 3,
            additions: 10,
            deletions: 2,
            created_at: None,
            updated_at: None,
            url: Some("http://x".into()),
        }
    }

    fn render_to_string(app: &mut App, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| render(f, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn detail_panel_appears_only_when_expanded() {
        let mut app = App::new("slate");
        app.prs.push(sample_pr());
        app.pr_state.select(Some(0));

        let collapsed = render_to_string(&mut app, 100, 24);
        assert!(collapsed.contains("Add the widget"), "row should render");
        assert!(!collapsed.contains("Detail"), "no detail panel while collapsed");

        app.show_detail = true;
        let expanded = render_to_string(&mut app, 100, 24);
        assert!(expanded.contains("Detail"), "detail panel should expand on Enter");
        assert!(expanded.contains("Alice Ng"), "detail should show the author");
    }

    #[test]
    fn pr_footer_shows_filter_key_and_active_filter() {
        let mut app = App::new("slate");
        app.prs.push(sample_pr());
        app.pr_state.select(Some(0));

        let out = render_to_string(&mut app, 100, 24);
        assert!(out.contains("filter"), "PR tab footer should advertise the filter key");
        assert!(out.contains("Pull Requests · all"), "PR title should show the active filter");

        // On the Work Items tab the filter key is not offered.
        app.active = 1;
        let wi = render_to_string(&mut app, 100, 24);
        assert!(!wi.contains("filter"), "filter is PR-only");
    }

    #[test]
    fn merge_picker_overlay_renders_over_the_list() {
        use crate::overlay::{Overlay, PickerKind};
        let mut app = App::new("slate");
        app.prs.push(sample_pr());
        app.pr_state.select(Some(0));
        app.overlay = Some(Overlay::Picker {
            title: "Merge PR #42 via".into(),
            items: vec!["Merge commit".into(), "Squash".into(), "Rebase".into()],
            selected: 1,
            kind: PickerKind::PrMergeStrategy,
        });

        let out = render_to_string(&mut app, 100, 24);
        assert!(out.contains("Merge PR #42 via"), "overlay title should render");
        assert!(out.contains("Squash") && out.contains("Rebase"), "strategies should render");
        assert!(out.contains("select") && out.contains("cancel"), "overlay hints in footer");
    }

    #[test]
    fn pr_footer_lists_write_action_keys() {
        let mut app = App::new("slate");
        app.prs.push(sample_pr());
        app.pr_state.select(Some(0));
        let out = render_to_string(&mut app, 120, 24);
        for label in ["approve", "reject", "merge", "comment"] {
            assert!(out.contains(label), "PR footer should advertise '{label}'");
        }
    }

    #[test]
    fn toast_renders_in_footer() {
        let mut app = App::new("slate");
        app.toast = Some("Filter: mine (1 PRs)".into());
        let out = render_to_string(&mut app, 100, 24);
        assert!(out.contains("Filter: mine"), "toast should appear in the footer");
    }

    #[test]
    fn theme_colours_are_indexed_not_truecolor() {
        // Truecolor RGB is what washed out on non-truecolor terminals; ensure we don't use it.
        for name in crate::theme::THEMES {
            let t = Theme::by_name(name);
            for c in [t.bg, t.fg, t.accent, t.green, t.red, t.sel_bg, t.magenta] {
                assert!(matches!(c, ratatui::style::Color::Indexed(_)), "{name}: {c:?} must be indexed");
            }
        }
    }
}
