//! All rendering. Immediate-mode: we redraw the whole frame from `App` each tick.

use chrono::{DateTime, Utc};
use forgetop_core::domain::*;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Tabs, Wrap};
use ratatui::Frame;

use crate::app::{App, PipeRow, TABS};
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
}

fn render_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let clock = app.last_refresh.format("%H:%M:%S");
    let right = format!("{} · {}{} ", theme.name, if app.loading { "⟳ " } else { "" }, clock);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.dim))
        .title(Span::styled(" forgetop ", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)))
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
        .divider(Span::styled("│", Style::default().fg(theme.dim)))
        .padding("", "")
        .style(Style::default().fg(theme.dim))
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
        .border_style(Style::default().fg(theme.dim))
        .title(Span::styled(format!(" {title} "), Style::default().fg(theme.fg)))
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
            PullRequestStatus::Merged => ("✦ merged", theme.accent),
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
    let block = section_block(theme, "Pull Requests");
    if app.prs.is_empty() {
        let msg = if app.loading { "Loading pull requests…" } else { "No pull requests. Press r to refresh." };
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
        .highlight_symbol("▌");
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
        .highlight_symbol("▌");
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
                Cell::from(Span::styled(format!("{} · {}", p.provider.as_str(), p.connection), Style::default().fg(theme.blue))),
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
        .highlight_symbol("▌");
    frame.render_stateful_widget(table, area, &mut app.pipe_state);
}

// ---- detail panel ----

fn render_detail(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let Some(idx) = app.selected() else { return };
    let block = section_block(theme, "Detail");

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

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(10), Constraint::Length(app.status.len().min(60) as u16 + 1)])
        .split(area);

    let keys = Line::from(vec![
        Span::styled(" ↑↓", Style::default().fg(theme.accent)),
        Span::styled(" move  ", Style::default().fg(theme.dim)),
        Span::styled("←→", Style::default().fg(theme.accent)),
        Span::styled(" tabs  ", Style::default().fg(theme.dim)),
        Span::styled("↵", Style::default().fg(theme.accent)),
        Span::styled(" detail  ", Style::default().fg(theme.dim)),
        Span::styled("r", Style::default().fg(theme.accent)),
        Span::styled(" refresh  ", Style::default().fg(theme.dim)),
        Span::styled("t", Style::default().fg(theme.accent)),
        Span::styled(" theme  ", Style::default().fg(theme.dim)),
        Span::styled("q", Style::default().fg(theme.accent)),
        Span::styled(" quit", Style::default().fg(theme.dim)),
    ]);
    frame.render_widget(Paragraph::new(keys), cols[0]);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(app.status.clone(), Style::default().fg(theme.dim))).right_aligned()),
        cols[1],
    );
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
