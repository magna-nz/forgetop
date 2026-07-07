//! All rendering. Immediate-mode: we redraw the whole frame from `App` each tick.

use chrono::{DateTime, Utc};
use forgetop_core::domain::*;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState, Tabs, Wrap,
};
use ratatui::Frame;

use crate::app::{App, ConfigView, DiffFocus, DiffView, PipelineView, PrView, Screen, WiView, PR_TABS, TABS};
use crate::diff::{cursor_line_label, pending_marks};
use crate::overlay::Overlay;
use crate::theme::{check_icon, pipeline_icon, Theme};
use crate::wizard::{Prompt, PromptKind};

/// Shown in empty sections when nothing is configured yet.
const FIRST_RUN_HINT: &str = "No connections yet — press n to add one, or C for config.";

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
            Constraint::Length(1), // spacer (bottom padding under connections)
            Constraint::Length(1), // footer
        ])
        .split(area);

    render_tabs(frame, rows[0], app);
    render_content(frame, rows[1], app);
    render_health(frame, rows[2], app);
    render_footer(frame, rows[4], app);

    if app.wizard.is_some() {
        render_wizard(frame, area, app);
    } else if app.overlay.is_some() {
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

    let vis = app.visible_indices();
    let titles: Vec<Line> = vis
        .iter()
        .map(|&i| {
            let count = match i {
                0 => app.prs.len(),
                1 => app.wis.len(),
                _ => app.pipes.len(),
            };
            Line::from(format!(" {} {count} ", TABS[i]))
        })
        .collect();
    let selected = vis.iter().position(|&i| i == app.active).unwrap_or(0);

    let tabs = Tabs::new(titles)
        .select(selected)
        .divider(Span::styled("  ", Style::default().fg(theme.dim)))
        .padding("", "")
        .style(Style::default().fg(theme.dim).bg(theme.bg))
        .highlight_style(Style::default().fg(theme.bg).bg(theme.accent).add_modifier(Modifier::BOLD))
        .block(block);

    frame.render_widget(tabs, area);
}

fn render_content(frame: &mut Frame, area: Rect, app: &mut App) {
    match &app.screen {
        Screen::Pipeline(view) => {
            render_pipeline(frame, area, &app.theme, view);
            return;
        }
        Screen::Config(view) => {
            render_config(frame, area, &app.theme, view);
            return;
        }
        Screen::PrView(_) | Screen::WiView(_) | Screen::List => {}
    }
    // The full-screen views report how far they can scroll, so the key handler can clamp.
    if matches!(app.screen, Screen::PrView(_)) {
        let max = if let Screen::PrView(view) = &app.screen { render_pr_view(frame, area, &app.theme, view) } else { 0 };
        app.detail_scroll_max = max;
        return;
    }
    if matches!(app.screen, Screen::WiView(_)) {
        let max = if let Screen::WiView(view) = &app.screen { render_wi_view(frame, area, &app.theme, view) } else { 0 };
        app.detail_scroll_max = max;
        return;
    }
    render_table(frame, area, app);
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

// ---- inline list helpers ----

/// Truncates/pads a string to exactly `w` display columns (approx: char count).
fn cell(s: &str, w: usize) -> String {
    let mut t: String = s.chars().take(w).collect();
    let n = t.chars().count();
    if n < w {
        t.push_str(&" ".repeat(w - n));
    }
    t
}

/// Applies the selected-row highlight (background + bold) to a whole line.
fn mark_selected(line: &mut Line, theme: &Theme) {
    let hl = Style::default().bg(theme.sel_bg).add_modifier(Modifier::BOLD);
    for span in &mut line.spans {
        span.style = span.style.patch(hl);
    }
}

/// Renders a section as a fixed column header + a scrollable body of rows, with the
/// selected row highlighted. Enter opens a full-screen view for the row.
fn render_inline_list(frame: &mut Frame, area: Rect, app: &mut App, title: &str, header: Line<'static>, rows: Vec<Line<'static>>) {
    let block = section_block(&app.theme, title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);
    frame.render_widget(Paragraph::new(header), parts[0]);
    app.content_h = parts[1].height;

    let selected = app.selected().unwrap_or(0);
    let scroll = app.list_scroll;
    let lines: Vec<Line> = rows
        .into_iter()
        .enumerate()
        .map(|(i, mut row)| {
            if i == selected {
                mark_selected(&mut row, &app.theme);
            }
            row
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), parts[1]);
}

/// Left indent inside the section, and the gap between columns — for breathing room.
const COL_LEAD: usize = 1;
const COL_GAP: usize = 3;

/// Joins column cells into a line, each padded to its column width, then pads the
/// whole line to the full body width (so a selected row highlights edge-to-edge).
fn cells_line(cells: &[(String, Style)], widths: &[usize], inner_w: usize) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")]; // COL_LEAD

    for (i, (text, style)) in cells.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" ".repeat(COL_GAP)));
        }
        spans.push(Span::styled(cell(text, widths[i]), *style));
    }
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    if used < inner_w {
        spans.push(Span::raw(" ".repeat(inner_w - used)));
    }
    Line::from(spans)
}

/// Sizes each column to the widest value in it (header or any row), clamping the
/// `flex` column so the whole row still fits. Returns the header line + row lines.
fn columnize(
    header_style: Style,
    headers: &[&str],
    rows: &[Vec<(String, Style)>],
    flex: usize,
    inner_w: usize,
    sort: Option<(usize, bool)>,
) -> (Line<'static>, Vec<Line<'static>>) {
    let ncol = headers.len();
    // The sorted column's header gets a direction arrow.
    let headers: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| match sort {
            Some((col, desc)) if col == i => format!("{h}{}", if desc { " ▼" } else { " ▲" }),
            _ => h.to_string(),
        })
        .collect();

    let mut w: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, (text, _)) in row.iter().enumerate().take(ncol) {
            w[i] = w[i].max(text.chars().count());
        }
    }
    // Clamp the flexible column so status/counts/dates stay pinned next to it.
    let padding = COL_LEAD + COL_GAP * ncol.saturating_sub(1);
    let fixed: usize = (0..ncol).filter(|&i| i != flex).map(|i| w[i]).sum::<usize>() + padding;
    w[flex] = w[flex].min(inner_w.saturating_sub(fixed)).max(3);

    let header_cells: Vec<(String, Style)> = headers.iter().map(|h| (h.clone(), header_style)).collect();
    let header = cells_line(&header_cells, &w, inner_w);
    let lines = rows.iter().map(|r| cells_line(r, &w, inner_w)).collect();
    (header, lines)
}

/// Maps a section's active sort to the header column index that gets the arrow.
fn sort_header_col(section: usize, key: &str) -> Option<usize> {
    match section {
        0 => match key {
            "status" => Some(0),
            "number" => Some(1),
            "title" => Some(2),
            "author" => Some(3),
            "checks" => Some(4),
            "updated" => Some(6),
            _ => None,
        },
        1 => match key {
            "state" => Some(0),
            "title" => Some(2),
            "type" => Some(3),
            "assignee" => Some(4),
            "updated" => Some(5),
            _ => None,
        },
        _ => match key {
            "status" => Some(0),
            "provider" => Some(1),
            "pipeline" => Some(2),
            "branch" => Some(4),
            "started" => Some(5),
            _ => None,
        },
    }
}

/// The `(header_col, desc)` arrow marker for a section's active sort, if any.
fn sort_marker(app: &App, section: usize) -> Option<(usize, bool)> {
    let s = app.sort_for(section)?;
    Some((sort_header_col(section, &s.key)?, s.desc))
}

/// Appends the active quick-filter to a section title (e.g. `Pipelines · /deploy`).
fn list_title(base: String, filter: &str) -> String {
    if filter.is_empty() {
        base
    } else {
        format!("{base} · /{filter}")
    }
}

// ---- Pull Requests ----

fn pr_status(theme: &Theme, pr: &PullRequest) -> (&'static str, ratatui::style::Color) {
    if pr.is_draft {
        return ("◌ draft", theme.dim);
    }
    match pr.status {
        PullRequestStatus::Open => ("● open", theme.green),
        PullRequestStatus::Merged => ("✦ merged", theme.magenta),
        PullRequestStatus::Closed => ("✗ closed", theme.red),
        PullRequestStatus::Draft => ("◌ draft", theme.dim),
    }
}

fn pr_checks(theme: &Theme, pr: &PullRequest) -> (String, ratatui::style::Color) {
    let text = match &pr.check_summary {
        Some(s) => format!("{} {}/{}", check_icon(pr.checks), s.successful, s.successful + s.failed + s.in_progress + s.neutral),
        None => format!("{} —", check_icon(pr.checks)),
    };
    (text, theme.check_color(pr.checks))
}

fn render_prs(frame: &mut Frame, area: Rect, app: &mut App) {
    let theme = &app.theme;
    let idxs = app.filtered_pr_indices();
    let title = list_title(format!("Pull Requests · {}", app.pr_filter_label()), &app.filters[0]);
    if idxs.is_empty() {
        let msg = if !app.filters[0].is_empty() {
            "No matches. Esc clears the filter."
        } else if app.health.is_empty() {
            FIRST_RUN_HINT
        } else if app.loading {
            "Loading pull requests…"
        } else {
            "No pull requests. Press f to change filter, r to refresh."
        };
        empty(frame, area, theme, msg, section_block(theme, &title));
        return;
    }

    let inner_w = area.width.saturating_sub(2) as usize;
    let dim = Style::default().fg(theme.dim).add_modifier(Modifier::BOLD);
    let headers = ["", "#", "Title", "Author", "Checks", "±", "Updated"];
    let cells: Vec<Vec<(String, Style)>> = idxs
        .iter()
        .map(|&i| &app.prs[i])
        .map(|pr| {
            let (st, stc) = pr_status(theme, pr);
            let (ck, ckc) = pr_checks(theme, pr);
            vec![
                (st.to_string(), Style::default().fg(stc)),
                (pr.number.map(|n| format!("#{n}")).unwrap_or_default(), Style::default().fg(theme.dim)),
                (pr.title.clone(), Style::default().fg(theme.fg)),
                (pr.author.display_name.clone(), Style::default().fg(theme.blue)),
                (ck, Style::default().fg(ckc)),
                (format!("+{} -{}", pr.additions, pr.deletions), Style::default().fg(theme.dim)),
                (rel_age(pr.updated_at), Style::default().fg(theme.dim)),
            ]
        })
        .collect();

    let (header, rows) = columnize(dim, &headers, &cells, 2, inner_w, sort_marker(app, 0));
    render_inline_list(frame, area, app, &title, header, rows);
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
    let idxs = app.filtered_wi_indices();
    let hidden_in_view = app.hidden_states_in_view();
    let base = if hidden_in_view == 0 {
        "Work Items".to_string()
    } else {
        format!("Work Items · {hidden_in_view} state(s) hidden")
    };
    let title = list_title(base, &app.filters[1]);
    if idxs.is_empty() {
        let msg = if !app.filters[1].is_empty() {
            "No matches. Esc clears the filter."
        } else if hidden_in_view > 0 {
            "All present states are hidden. Press f to choose states."
        } else if app.health.is_empty() {
            FIRST_RUN_HINT
        } else if app.loading {
            "Loading work items…"
        } else {
            "No work items. Press r to refresh."
        };
        empty(frame, area, theme, msg, section_block(theme, &title));
        return;
    }

    let inner_w = area.width.saturating_sub(2) as usize;
    let dim = Style::default().fg(theme.dim).add_modifier(Modifier::BOLD);
    let headers = ["State", "ID", "Title", "Type", "Assignee", "Updated"];
    let cells: Vec<Vec<(String, Style)>> = idxs
        .iter()
        .map(|&i| &app.wis[i])
        .map(|wi| {
            vec![
                (format!("● {}", wi.state), Style::default().fg(wi_state_color(theme, wi.state_category))),
                (wi.identifier.clone().unwrap_or_default(), Style::default().fg(theme.dim)),
                (wi.title.clone(), Style::default().fg(theme.fg)),
                (wi.work_item_type.clone().unwrap_or_default(), Style::default().fg(theme.dim)),
                (wi.assignee.as_ref().map(|a| a.display_name.clone()).unwrap_or_else(|| "—".into()), Style::default().fg(theme.blue)),
                (rel_age(wi.updated_at), Style::default().fg(theme.dim)),
            ]
        })
        .collect();

    let (header, rows) = columnize(dim, &headers, &cells, 2, inner_w, sort_marker(app, 1));
    render_inline_list(frame, area, app, &title, header, rows);
}

// ---- Pipelines ----

fn render_pipes(frame: &mut Frame, area: Rect, app: &mut App) {
    let theme = &app.theme;
    let idxs = app.filtered_pipe_indices();
    let title = list_title("Pipelines".to_string(), &app.filters[2]);
    if idxs.is_empty() {
        let msg = if !app.filters[2].is_empty() {
            "No matches. Esc clears the filter."
        } else if app.health.is_empty() {
            FIRST_RUN_HINT
        } else if app.loading {
            "Loading pipeline runs…"
        } else {
            "No pipeline runs. Press r to refresh."
        };
        empty(frame, area, theme, msg, section_block(theme, &title));
        return;
    }

    let inner_w = area.width.saturating_sub(2) as usize;
    let dim = Style::default().fg(theme.dim).add_modifier(Modifier::BOLD);
    let headers = ["", "Provider", "Pipeline", "#", "Branch", "Started"];
    let cells: Vec<Vec<(String, Style)>> = idxs
        .iter()
        .map(|&i| &app.pipes[i])
        .map(|p| {
            let color = theme.pipeline_color(p.run.status);
            vec![
                (format!("{} {:?}", pipeline_icon(p.run.status), p.run.status), Style::default().fg(color)),
                (format!("{} · {}", p.provider.as_str(), p.connection), Style::default().fg(theme.cyan)),
                (p.run.name.clone().unwrap_or_else(|| p.run.definition_id.clone()), Style::default().fg(theme.fg)),
                (p.run.number.map(|n| format!("#{n}")).unwrap_or_default(), Style::default().fg(theme.dim)),
                (p.run.branch.clone().unwrap_or_default(), Style::default().fg(theme.dim)),
                (rel_age(p.run.started_at), Style::default().fg(theme.dim)),
            ]
        })
        .collect();

    let (header, rows) = columnize(dim, &headers, &cells, 2, inner_w, sort_marker(app, 2));
    render_inline_list(frame, area, app, &title, header, rows);
}

// ---- full-screen PR / work-item views ----

fn field(theme: &Theme, label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<12}"), Style::default().fg(theme.dim)),
        Span::styled(value, Style::default().fg(theme.fg)),
    ])
}

fn comment_lines(theme: &Theme, threads: &[CommentThread]) -> Vec<Line<'static>> {
    let total: usize = threads.iter().map(|t| t.comments.len()).sum();
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(format!("Comments ({total})"), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))),
    ];
    if total == 0 {
        lines.push(Line::from(Span::styled("No comments.", Style::default().fg(theme.dim))));
    }
    for thread in threads {
        for c in &thread.comments {
            lines.push(Line::from(Span::styled(format!("{}:", c.author.display_name), Style::default().fg(theme.blue))));
            for l in c.body.lines() {
                lines.push(Line::from(Span::styled(format!("  {l}"), Style::default().fg(theme.fg))));
            }
        }
    }
    lines
}

/// The PR sub-tab bar rendered as a row of pills (active one lit).
fn pr_tabs_line(theme: &Theme, active: usize) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (i, name) in PR_TABS.iter().enumerate() {
        let style = if i == active {
            Style::default().fg(theme.bg).bg(theme.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.dim)
        };
        spans.push(Span::styled(format!(" {name} "), style));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled("  ←/→ tabs · Esc close", Style::default().fg(theme.dim)));
    Line::from(spans)
}

fn pr_conversation_lines(theme: &Theme, pr: &PullRequest, threads: &[CommentThread]) -> Vec<Line<'static>> {
    let mut lines = vec![
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
    if let Some(desc) = pr.description.as_ref().filter(|d| !d.is_empty()) {
        lines.push(Line::from(""));
        for l in desc.lines() {
            lines.push(Line::from(Span::styled(l.to_string(), Style::default().fg(theme.dim))));
        }
    }
    lines.extend(comment_lines(theme, threads));
    lines
}

/// The Commits tab: one line per commit (short sha · message · author · age).
fn pr_commit_lines(theme: &Theme, commits: &[Commit], sel: usize) -> Vec<Line<'static>> {
    if commits.is_empty() {
        return vec![Line::from(Span::styled("No commits.", Style::default().fg(theme.dim)))];
    }
    commits
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut line = Line::from(vec![
                Span::styled(cell(&c.sha, 9), Style::default().fg(theme.yellow)),
                Span::styled(cell(&c.message, 56), Style::default().fg(theme.fg)),
                Span::styled(cell(&c.author, 16), Style::default().fg(theme.blue)),
                Span::styled(rel_age(c.date), Style::default().fg(theme.dim)),
            ]);
            if i == sel {
                mark_selected(&mut line, theme);
            }
            line
        })
        .collect()
}

/// The Checks tab: one line per named check with its status, like the pipeline steps.
fn pr_checks_lines(theme: &Theme, checks: &[CheckRun]) -> Vec<Line<'static>> {
    if checks.is_empty() {
        return vec![Line::from(Span::styled("No checks reported for this pull request.", Style::default().fg(theme.dim)))];
    }
    checks
        .iter()
        .map(|c| {
            let color = theme.check_color(c.status);
            Line::from(vec![
                Span::styled(format!(" {} ", check_icon(c.status)), Style::default().fg(color)),
                Span::styled(cell(&c.name, 40), Style::default().fg(theme.fg)),
                Span::styled(format!("{:?}", c.status), Style::default().fg(color)),
            ])
        })
        .collect()
}

/// Renders the PR view and returns the maximum scroll offset for the current tab.
fn render_pr_view(frame: &mut Frame, area: Rect, theme: &Theme, view: &PrView) -> u16 {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(1), Constraint::Min(3)])
        .split(area);

    // Header: title + status + branch + author.
    let (st, stc) = pr_status(theme, &view.pr);
    let header = Line::from(vec![
        Span::styled(st, Style::default().fg(stc).add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("   {} → {}", view.pr.source_ref.clone().unwrap_or_default(), view.pr.target_ref.clone().unwrap_or_default()),
            Style::default().fg(theme.dim),
        ),
        Span::styled(format!("   {}", view.pr.author.display_name), Style::default().fg(theme.blue)),
    ]);
    frame.render_widget(Paragraph::new(header).block(section_block(theme, &view.label)), rows[0]);

    // Sub-tab bar.
    frame.render_widget(Paragraph::new(pr_tabs_line(theme, view.tab)), rows[1]);

    // Content.
    if view.tab == 3 {
        render_diff(frame, rows[2], theme, &view.diff, &view.pending);
        return 0; // the Diff tab manages its own scrolling
    }
    // Commits: a row cursor (Enter drills into that commit's diff), scroll follows it.
    if view.tab == 1 {
        let lines = pr_commit_lines(theme, &view.commits, view.commit_sel);
        let inner_h = rows[2].height.saturating_sub(2) as usize;
        let total = view.commits.len();
        let scroll = view.commit_sel.saturating_sub(inner_h / 2).min(total.saturating_sub(inner_h.max(1))) as u16;
        frame.render_widget(Paragraph::new(lines).block(section_block(theme, "Commits")).scroll((scroll, 0)), rows[2]);
        return 0;
    }
    let (title, lines) = match view.tab {
        0 => ("Conversation", pr_conversation_lines(theme, &view.pr, &view.diff.threads)),
        _ => ("Checks", pr_checks_lines(theme, &view.checks)),
    };
    let inner_h = rows[2].height.saturating_sub(2);
    let max = (lines.len() as u16).saturating_sub(inner_h);
    frame.render_widget(
        Paragraph::new(lines).block(section_block(theme, title)).scroll((view.scroll.min(max), 0)).wrap(Wrap { trim: false }),
        rows[2],
    );
    max
}

fn render_wi_view(frame: &mut Frame, area: Rect, theme: &Theme, view: &WiView) -> u16 {
    let wi = &view.wi;
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(area);

    let label = format!("{} {}", wi.identifier.clone().unwrap_or_default(), wi.title);
    let header = Line::from(vec![
        Span::styled(format!("● {}", wi.state), Style::default().fg(wi_state_color(theme, wi.state_category)).add_modifier(Modifier::BOLD)),
        Span::styled(format!("   {}", wi.work_item_type.clone().unwrap_or_default()), Style::default().fg(theme.dim)),
        Span::styled(
            format!("   {}", wi.assignee.as_ref().map(|a| a.display_name.clone()).unwrap_or_else(|| "—".into())),
            Style::default().fg(theme.blue),
        ),
    ]);
    frame.render_widget(Paragraph::new(header).block(section_block(theme, &label)), rows[0]);

    let mut lines = vec![
        field(theme, "State", format!("{} ({:?})", wi.state, wi.state_category)),
        field(theme, "Type", wi.work_item_type.clone().unwrap_or_else(|| "—".into())),
        field(theme, "Assignee", wi.assignee.as_ref().map(|a| a.display_name.clone()).unwrap_or_else(|| "—".into())),
    ];
    if let Some(url) = &wi.url {
        lines.push(field(theme, "URL", url.clone()));
    }
    if let Some(desc) = wi.description.as_ref().filter(|d| !d.is_empty()) {
        lines.push(Line::from(""));
        for l in desc.lines() {
            lines.push(Line::from(Span::styled(l.to_string(), Style::default().fg(theme.dim))));
        }
    }
    lines.extend(comment_lines(theme, &view.threads));
    let inner_h = rows[1].height.saturating_sub(2);
    let max = (lines.len() as u16).saturating_sub(inner_h);
    frame.render_widget(
        Paragraph::new(lines).block(section_block(theme, "Work Item")).scroll((view.scroll.min(max), 0)).wrap(Wrap { trim: false }),
        rows[1],
    );
    max
}

// ---- connections + footer ----

fn render_health(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let mut spans = vec![Span::styled("connections  ", Style::default().fg(theme.dim))];
    if app.health.is_empty() {
        spans.push(Span::styled("none yet — press n to add a connection", Style::default().fg(theme.dim)));
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
    if let Some(wizard) = &app.wizard {
        return match wizard.current() {
            Some(Prompt { kind: PromptKind::Pick { .. }, .. }) => vec![("↑↓", "choose"), ("↵", "next"), ("Esc", "cancel")],
            Some(_) => vec![("type", "value"), ("↵", "next"), ("Esc", "cancel")],
            None => vec![("Esc", "cancel")],
        };
    }
    if let Some(overlay) = &app.overlay {
        return overlay.hint();
    }
    if let Screen::PrView(v) = &app.screen {
        return if v.tab == 3 {
            if v.diff.focus == DiffFocus::Patch {
                let mut keys = vec![("↑↓", "line"), ("c", "comment")];
                if !v.pending.is_empty() {
                    keys.push(("s", "submit review"));
                }
                keys.extend([("PgUp/Dn", "jump"), ("Esc", "files"), ("o", "open")]);
                keys
            } else {
                vec![("←→", "tabs"), ("↑↓", "file"), ("↵", "open file"), ("PgUp/Dn", "scroll"), ("a", "approve"), ("m", "merge"), ("o", "open"), ("Esc", "back")]
            }
        } else if v.tab == 1 {
            vec![("←→", "tabs"), ("↑↓", "commit"), ("↵", "commit diff"), ("a", "approve"), ("m", "merge"), ("o", "open"), ("Esc", "back")]
        } else {
            vec![("←→", "tabs"), ("PgUp/Dn", "scroll"), ("a", "approve"), ("x", "reject"), ("m", "merge"), ("c", "comment"), ("o", "open"), ("Esc", "back")]
        };
    }
    if matches!(app.screen, Screen::WiView(_)) {
        return vec![("PgUp/Dn", "scroll"), ("s", "state"), ("c", "comment"), ("o", "open"), ("Esc", "back"), ("q", "quit")];
    }
    if let Screen::Pipeline(v) = &app.screen {
        return if v.logs.is_some() {
            vec![("↑↓", "scroll"), ("PgUp/Dn", "jump"), ("Esc", "close logs")]
        } else {
            vec![("↑↓", "move"), ("↵", "expand"), ("L", "logs"), ("T", "trigger"), ("o", "open job"), ("Esc", "back"), ("q", "quit")]
        };
    }
    if matches!(app.screen, Screen::Config(_)) {
        return vec![
            ("↑↓", "move"),
            ("a", "add"),
            ("p", "bind-PR"),
            ("w", "bind-WI"),
            ("s", "pipelines"),
            ("x", "remove"),
            ("Esc", "back"),
            ("q", "quit"),
        ];
    }
    let mut keys = vec![("↑↓", "move"), ("←→", "tabs")];
    match app.active {
        0 => keys.extend([("↵", "open"), ("f", "filter"), ("S", "sort"), ("o", "browser")]),
        1 => keys.extend([("↵", "open"), ("s", "state"), ("f", "states"), ("S", "sort"), ("c", "comment"), ("o", "browser")]),
        2 => keys.extend([("↵", "drill-in"), ("S", "sort"), ("T", "trigger"), ("o", "open")]),
        _ => {}
    }
    keys.push(("/", "find"));
    keys.extend([("v", "tabs"), ("C", "config"), ("r", "refresh"), ("t", "theme"), ("?", "help"), ("q", "quit")]);
    keys
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    // A subtle bar background so the glossary reads as a distinct strip.
    let bar = Style::default().bg(theme.panel);

    // While the quick-filter input is open, the footer becomes that input.
    if app.filtering {
        let spans = vec![
            Span::styled(" / ", bar.fg(theme.bg).bg(theme.accent).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {}", app.active_filter()), bar.fg(theme.fg).add_modifier(Modifier::BOLD)),
            Span::styled("▌", bar.fg(theme.accent)),
            Span::styled("   ↵ apply · Esc clear", bar.fg(theme.dim)),
        ];
        frame.render_widget(Paragraph::new(Line::from(spans)).style(bar), area);
        return;
    }

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

// ---- diff view ----

fn kind_badge(theme: &Theme, kind: FileChangeKind) -> Span<'static> {
    let (letter, color) = match kind {
        FileChangeKind::Added => ("A", theme.green),
        FileChangeKind::Modified => ("M", theme.yellow),
        FileChangeKind::Deleted => ("D", theme.red),
        FileChangeKind::Renamed => ("R", theme.blue),
    };
    Span::styled(letter, Style::default().fg(color).add_modifier(Modifier::BOLD))
}

fn render_diff(frame: &mut Frame, area: Rect, theme: &Theme, diff: &DiffView, pending: &[LineComment]) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(44), Constraint::Min(20)])
        .split(area);

    let thread_h = if diff.threads.is_empty() { 3 } else { 12 };
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(thread_h)])
        .split(cols[0]);

    render_diff_files(frame, left[0], theme, diff);
    render_diff_threads(frame, left[1], theme, diff);
    render_diff_patch(frame, cols[1], theme, diff, pending);
}

fn render_diff_files(frame: &mut Frame, area: Rect, theme: &Theme, diff: &DiffView) {
    let title = match &diff.commit_label {
        Some(l) => format!("commit {l} · files"),
        None => format!("{} · files", diff.pr_label),
    };
    let block = section_block(theme, &title);
    if diff.files.is_empty() {
        empty(frame, area, theme, "No changed files.", block);
        return;
    }

    let rows: Vec<Row> = diff
        .files
        .iter()
        .map(|f| {
            Row::new(vec![
                Cell::from(kind_badge(theme, f.kind)),
                Cell::from(Span::styled(f.path.clone(), Style::default().fg(theme.fg))),
                Cell::from(Span::styled(format!("+{} -{}", f.additions, f.deletions), Style::default().fg(theme.dim))),
            ])
        })
        .collect();

    let widths = [Constraint::Length(1), Constraint::Min(10), Constraint::Length(10)];
    let table = Table::new(rows, widths)
        .block(block)
        .column_spacing(1)
        .row_highlight_style(highlight(theme))
        .highlight_symbol("▐ ");
    let mut state = TableState::default();
    state.select(Some(diff.selected));
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_diff_threads(frame: &mut Frame, area: Rect, theme: &Theme, diff: &DiffView) {
    let title = format!("Comments ({})", diff.threads.len());
    let block = section_block(theme, &title);
    if diff.threads.is_empty() {
        empty(frame, area, theme, "No review comments.", block);
        return;
    }
    let mut lines: Vec<Line> = Vec::new();
    for t in &diff.threads {
        let loc = match (&t.file_path, t.line) {
            (Some(p), Some(l)) => format!("{p}:{l}"),
            (Some(p), None) => p.clone(),
            _ => "general".into(),
        };
        let mark = if t.is_resolved { "✓ resolved" } else { "○ open" };
        lines.push(Line::from(vec![
            Span::styled(format!("{loc} "), Style::default().fg(theme.accent)),
            Span::styled(mark.to_string(), Style::default().fg(if t.is_resolved { theme.green } else { theme.dim })),
        ]));
        for c in &t.comments {
            lines.push(Line::from(vec![
                Span::styled(format!("  {}: ", c.author.display_name), Style::default().fg(theme.blue)),
                Span::styled(c.body.clone(), Style::default().fg(theme.fg)),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: true }), area);
}

fn render_diff_patch(frame: &mut Frame, area: Rect, theme: &Theme, diff: &DiffView, pending: &[LineComment]) {
    let Some(file) = diff.current() else {
        frame.render_widget(section_block(theme, "Patch"), area);
        return;
    };

    let Some(patch) = &file.patch else {
        let title = format!("{}  (+{} -{})", file.path, file.additions, file.deletions);
        let block = section_block(theme, &title);
        empty(frame, area, theme, "No inline patch for this file (binary, or the provider didn't supply one).", block);
        return;
    };

    let patch_focus = diff.focus == DiffFocus::Patch;
    // In the line cursor, show where we are; otherwise just the file summary.
    let title = match patch_focus.then(|| cursor_line_label(patch, diff.cursor)).flatten() {
        Some(loc) => format!("{}  (+{} -{}) · {loc}", file.path, file.additions, file.deletions),
        None => format!("{}  (+{} -{})", file.path, file.additions, file.deletions),
    };
    let block = section_block(theme, &title);

    let inner_w = area.width.saturating_sub(2) as usize;
    let inner_h = area.height.saturating_sub(2) as usize;
    let total = patch.lines().count();
    // Patch lines (in this file) that carry a pending comment get a gutter bar.
    let marks = pending_marks(patch, &file.path, pending);

    // In the cursor, derive scroll so the cursor line stays roughly centred; the
    // stored scroll is only used for the free-scroll (file-list) mode.
    let scroll = if patch_focus {
        let half = inner_h / 2;
        diff.cursor.saturating_sub(half).min(total.saturating_sub(inner_h.max(1))) as u16
    } else {
        diff.scroll
    };

    let lines: Vec<Line> = patch
        .lines()
        .enumerate()
        .map(|(i, l)| {
            // A 1-col gutter: a bar when the line has a pending comment.
            let gutter = if marks.contains(&i) {
                Span::styled("▎", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))
            } else {
                Span::raw(" ")
            };
            if patch_focus && i == diff.cursor {
                // Pad to full width (minus the gutter) so the highlight spans the row.
                let mut text = l.to_string();
                let w = text.chars().count();
                if w + 1 < inner_w {
                    text.push_str(&" ".repeat(inner_w - 1 - w));
                }
                Line::from(vec![
                    gutter,
                    Span::styled(text, Style::default().fg(patch_fg(theme, l)).bg(theme.sel_bg).add_modifier(Modifier::BOLD)),
                ])
            } else {
                let mut line = patch_line(theme, l);
                line.spans.insert(0, gutter);
                line
            }
        })
        .collect();

    frame.render_widget(Paragraph::new(lines).block(block).scroll((scroll, 0)), area);
}

fn patch_fg(theme: &Theme, line: &str) -> ratatui::style::Color {
    if line.starts_with("@@") {
        theme.accent
    } else if line.starts_with("+++") || line.starts_with("---") {
        theme.dim
    } else if line.starts_with('+') {
        theme.green
    } else if line.starts_with('-') {
        theme.red
    } else {
        theme.fg
    }
}

fn patch_line(theme: &Theme, line: &str) -> Line<'static> {
    Line::from(Span::styled(line.to_string(), Style::default().fg(patch_fg(theme, line))))
}

// ---- config / connections ----

fn render_config(frame: &mut Frame, area: Rect, theme: &Theme, view: &ConfigView) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(6)])
        .split(area);

    // Connections table.
    let conns_block = section_block(theme, "Connections");
    if view.connections.is_empty() {
        empty(frame, rows[0], theme, "No connections yet. Press a to add one.", conns_block);
    } else {
        let header = header_row(theme, &["", "Name", "Provider", "Bound to"]);
        let table_rows: Vec<Row> = view
            .connections
            .iter()
            .map(|c| {
                let (dot, color) = if c.healthy { ("●", theme.green) } else { ("○", theme.red) };
                let bound = if c.bindings.is_empty() { "—".to_string() } else { c.bindings.join(", ") };
                Row::new(vec![
                    Cell::from(Span::styled(dot, Style::default().fg(color))),
                    Cell::from(Span::styled(c.display.clone(), Style::default().fg(theme.fg))),
                    Cell::from(Span::styled(c.provider.as_str().to_string(), Style::default().fg(theme.cyan))),
                    Cell::from(Span::styled(bound, Style::default().fg(theme.dim))),
                ])
            })
            .collect();
        let widths = [Constraint::Length(1), Constraint::Min(16), Constraint::Length(14), Constraint::Length(18)];
        let table = Table::new(table_rows, widths)
            .header(header)
            .block(conns_block)
            .column_spacing(1)
            .row_highlight_style(highlight(theme))
            .highlight_symbol("▐ ");
        let mut state = TableState::default();
        state.select(Some(view.selected.min(view.connections.len().saturating_sub(1))));
        frame.render_stateful_widget(table, rows[0], &mut state);
    }

    // Section bindings summary.
    let dash = || "— (unbound)".to_string();
    let subs = if view.pipeline_subs.is_empty() { dash() } else { view.pipeline_subs.join(", ") };
    let lines = vec![
        Line::from(vec![
            Span::styled("Pull Requests  ", Style::default().fg(theme.dim)),
            Span::styled(view.pr_binding.clone().unwrap_or_else(dash), Style::default().fg(theme.fg)),
        ]),
        Line::from(vec![
            Span::styled("Work Items     ", Style::default().fg(theme.dim)),
            Span::styled(view.wi_binding.clone().unwrap_or_else(dash), Style::default().fg(theme.fg)),
        ]),
        Line::from(vec![
            Span::styled("Pipelines      ", Style::default().fg(theme.dim)),
            Span::styled(subs, Style::default().fg(theme.fg)),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines).block(section_block(theme, "Section bindings")), rows[1]);
}

// ---- pipeline drill-in ----

fn render_pipeline(frame: &mut Frame, area: Rect, theme: &Theme, view: &PipelineView) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(area);

    // Header: run identity + status + branch/trigger.
    let branch = view.branch.clone().unwrap_or_else(|| "—".into());
    let who = view.run.triggered_by.as_ref().map(|u| u.display_name.clone()).unwrap_or_else(|| "—".into());
    let header = Line::from(vec![
        Span::styled(format!("{} ", pipeline_icon(view.run.status)), Style::default().fg(theme.pipeline_color(view.run.status))),
        Span::styled(format!("{:?}", view.run.status), Style::default().fg(theme.pipeline_color(view.run.status)).add_modifier(Modifier::BOLD)),
        Span::styled(format!("   branch {branch}   triggered by {who}"), Style::default().fg(theme.dim)),
    ]);
    let header_block = section_block(theme, &view.title);
    frame.render_widget(Paragraph::new(header).block(header_block), rows[0]);

    // A log pane, when open, replaces the tree.
    if let Some(log) = &view.logs {
        let lines: Vec<Line> = log.lines.iter().map(|l| Line::from(Span::styled(l.clone(), Style::default().fg(theme.fg)))).collect();
        let inner_h = rows[1].height.saturating_sub(2);
        let max = (lines.len() as u16).saturating_sub(inner_h);
        frame.render_widget(
            Paragraph::new(lines).block(section_block(theme, &log.title)).scroll((log.scroll.min(max), 0)),
            rows[1],
        );
        return;
    }

    // Tree of stages → jobs → steps.
    let nodes = view.flatten();
    let tree_block = section_block(theme, "Stages · jobs · steps");
    if nodes.is_empty() {
        empty(frame, rows[1], theme, "No stages reported for this run.", tree_block);
        return;
    }

    let items: Vec<ListItem> = nodes
        .iter()
        .map(|n| {
            let indent = "  ".repeat(n.depth);
            let marker = match n.key {
                Some(_) if n.expanded => "▾ ",
                Some(_) => "▸ ",
                None => "· ",
            };
            let label_style = if n.depth == 0 {
                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg)
            };
            let mut spans = vec![
                Span::raw(indent),
                Span::styled(marker, Style::default().fg(theme.dim)),
                Span::styled(format!("{} ", pipeline_icon(n.status)), Style::default().fg(theme.pipeline_color(n.status))),
                Span::styled(n.label.clone(), label_style),
            ];
            if let Some(d) = &n.duration {
                spans.push(Span::styled(format!("  {d}"), Style::default().fg(theme.dim)));
            }
            if let Some(p) = &n.problem {
                spans.push(Span::styled(format!("  ⚠ {p}"), Style::default().fg(theme.red)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items).block(tree_block).highlight_style(highlight(theme)).highlight_symbol("▐ ");
    let mut state = ListState::default();
    state.select(Some(view.selected.min(nodes.len().saturating_sub(1))));
    frame.render_stateful_widget(list, rows[1], &mut state);
}

// ---- overlays ----

fn render_overlay(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let Some(overlay) = &app.overlay else { return };

    // Help is a large scrollable panel rather than the small centred card.
    if let Overlay::Help { scroll } = overlay {
        render_help(frame, area, theme, *scroll);
        return;
    }

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
        Overlay::Toggle { items, selected, .. } => (
            items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let (arrow, arrow_color) = if item.on { ("▶", theme.green) } else { ("·", theme.dim) };
                    let cursor = if i == *selected { "▐ " } else { "  " };
                    let label_style = if i == *selected {
                        Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)
                    } else if item.on {
                        Style::default().fg(theme.fg)
                    } else {
                        Style::default().fg(theme.dim)
                    };
                    Line::from(vec![
                        Span::styled(cursor, Style::default().fg(theme.accent)),
                        Span::styled(format!("{arrow} "), Style::default().fg(arrow_color)),
                        Span::styled(item.label.clone(), label_style),
                    ])
                })
                .collect(),
            theme.green,
        ),
        Overlay::Help { .. } => return, // handled above
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

/// Every keybinding, grouped by context — the content of the `?` help panel.
fn help_sections() -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    vec![
        (
            "Global",
            vec![
                ("←/→  h/l  Tab  1–3", "Switch tab"),
                ("↑/↓  k/j", "Move selection"),
                ("/", "Quick-filter the list"),
                ("S", "Sort by column (re-pick flips direction)"),
                ("o", "Open selected in browser"),
                ("n", "Add a connection (wizard)"),
                ("v", "Choose which tabs are visible"),
                ("C", "Config / connections"),
                ("r", "Refresh    t  cycle theme"),
                ("?", "This help"),
                ("q  Ctrl-C", "Quit    Esc  back / close"),
            ],
        ),
        (
            "Pull Requests (list)",
            vec![
                ("Enter", "Open the PR view (all actions live there)"),
                ("f", "Cycle filter (All / Mine / Review)"),
            ],
        ),
        (
            "PR view (after Enter)",
            vec![
                ("←/→", "Switch sub-tab"),
                ("a  x", "Approve / request changes"),
                ("m", "Merge (choose strategy)"),
                ("c", "Comment (inline on a diff line, else the PR)"),
                ("Enter (Commits)", "Drill into that commit's diff"),
                ("Enter (Diff file)", "Line cursor in the patch"),
                ("↑/↓ (line cursor)", "Move line-by-line"),
                ("s", "Submit buffered line comments as a review"),
                ("o", "Open in browser"),
                ("Esc", "Step back (line → files → close)"),
            ],
        ),
        (
            "Work Items",
            vec![
                ("Enter", "Open"),
                ("s", "Change state"),
                ("f", "Choose which states to show"),
                ("c", "Comment"),
            ],
        ),
        (
            "Pipelines",
            vec![
                ("Enter", "Drill in (stages → jobs → steps)"),
                ("Enter (in drill-in)", "Expand / collapse a node"),
                ("L", "View the selected job's logs"),
                ("o", "Open the selected job in the browser"),
                ("T", "Trigger a run"),
            ],
        ),
        (
            "Config / connections",
            vec![
                ("a", "Add a connection"),
                ("p  w", "Bind Pull Requests / Work Items"),
                ("s", "Pipeline subscriptions"),
                ("x", "Remove connection"),
            ],
        ),
    ]
}

fn render_help(frame: &mut Frame, area: Rect, theme: &Theme, scroll: u16) {
    let mut lines: Vec<Line> = Vec::new();
    for (section, keys) in help_sections() {
        lines.push(Line::from(Span::styled(section, Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))));
        for (k, d) in keys {
            lines.push(Line::from(vec![
                Span::styled(format!("  {k:<18}"), Style::default().fg(theme.yellow)),
                Span::styled(d, Style::default().fg(theme.fg)),
            ]));
        }
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled("↑↓ scroll · Esc close", Style::default().fg(theme.dim))));

    let total = lines.len() as u16;
    let width = 68.min(area.width.saturating_sub(4));
    let height = area.height.saturating_sub(4).clamp(10, total + 2);
    let rect = centered_rect(width, height, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.panel))
        .title(Span::styled(" Keybindings ", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)));

    // Clamp scroll so you can't page past the end.
    let inner_h = height.saturating_sub(2);
    let max_scroll = total.saturating_sub(inner_h);

    frame.render_widget(Clear, rect);
    frame.render_widget(Paragraph::new(lines).block(block).scroll((scroll.min(max_scroll), 0)), rect);
}

fn render_wizard(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let Some(wizard) = &app.wizard else { return };
    let Some(prompt) = wizard.current() else { return };
    let accent = theme.accent;

    let mut lines: Vec<Line> = vec![Line::from(Span::styled(prompt.label.clone(), Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)))];
    if !prompt.help.is_empty() {
        lines.push(Line::from(Span::styled(prompt.help.clone(), Style::default().fg(theme.dim))));
    }
    lines.push(Line::from(""));
    match &prompt.kind {
        PromptKind::Text { buffer, secret } => {
            let shown = if *secret { "•".repeat(buffer.chars().count()) } else { buffer.clone() };
            lines.push(Line::from(vec![
                Span::styled("> ", Style::default().fg(accent)),
                Span::styled(shown, Style::default().fg(theme.fg)),
                Span::styled("█", Style::default().fg(accent)),
            ]));
        }
        PromptKind::Pick { items, selected } => {
            for (i, item) in items.iter().enumerate() {
                if i == *selected {
                    lines.push(Line::from(vec![
                        Span::styled(" ▐ ", Style::default().fg(accent)),
                        Span::styled(item.clone(), Style::default().fg(accent).add_modifier(Modifier::BOLD)),
                    ]));
                } else {
                    lines.push(Line::from(Span::styled(format!("   {item}"), Style::default().fg(theme.fg))));
                }
            }
        }
    }

    let hint = footer_keys(app)
        .into_iter()
        .flat_map(|(k, l)| {
            [
                Span::styled(format!(" {k} "), Style::default().fg(theme.bg).bg(accent).add_modifier(Modifier::BOLD)),
                Span::styled(format!(" {l}   "), Style::default().fg(theme.dim)),
            ]
        })
        .collect::<Vec<_>>();
    lines.push(Line::from(""));
    lines.push(Line::from(hint));

    // Wider so the per-field help fits on one line; +1 row of margin for any wrap.
    let height = lines.len() as u16 + 3;
    let width = 84.min(area.width.saturating_sub(6));
    let rect = centered_rect(width, height, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent))
        .style(Style::default().bg(theme.panel))
        .title(Span::styled(format!(" Add connection · {} ", wizard.step_label()), Style::default().fg(accent).add_modifier(Modifier::BOLD)));

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

    fn pr_view(tab: usize, checks: Vec<CheckRun>, files: Vec<FileChange>) -> crate::app::PrView {
        use crate::app::DiffView;
        crate::app::PrView {
            label: "PR #42 — Add the widget".into(),
            url: Some("http://x".into()),
            pr: sample_pr(),
            tab,
            checks,
            commits: vec![Commit {
                sha: "abc1234".into(),
                message: "Add the retry policy".into(),
                author: "alice".into(),
                date: None,
                url: None,
            }],
            commit_sel: 0,
            pr_files: vec![],
            scroll: 0,
            diff: DiffView {
                pr_label: "PR #42".into(),
                url: None,
                files,
                threads: vec![],
                selected: 0,
                scroll: 0,
                focus: crate::app::DiffFocus::FileList,
                cursor: 0,
                commit_label: None,
            },
            pending: vec![],
            review_draft: None,
        }
    }

    #[test]
    fn columnize_sizes_the_flex_column_to_content() {
        let s = Style::default();
        let headers = ["A", "Title", "B"];
        let rows = vec![
            vec![("x".to_string(), s), ("short".to_string(), s), ("yy".to_string(), s)],
            vec![("xx".to_string(), s), ("longer title".to_string(), s), ("y".to_string(), s)],
        ];
        // Wide viewport: the Title (flex) column should size to its longest value (12),
        // not stretch to fill — so column B lands right after it, not at the far edge.
        let (_header, lines) = columnize(s, &headers, &rows, 1, 200, None);
        let plain: String = lines[0].spans.iter().map(|sp| sp.content.as_ref()).collect::<String>();
        // LEAD(1) + A(2) + GAP(3) + Title(12) + GAP(3) => B starts at index 21.
        assert_eq!(&plain[21..23], "yy", "B column packs right after the content-sized Title");
    }

    #[test]
    fn enter_opens_full_screen_pr_view_with_tabs() {
        use crate::app::Screen;
        let mut app = App::new("slate");
        app.screen = Screen::PrView(Box::new(pr_view(0, vec![], vec![])));
        let out = render_to_string(&mut app, 100, 24);
        // Header (label), sub-tab bar, and Conversation content.
        assert!(out.contains("Add the widget"), "PR label in header");
        assert!(out.contains("Conversation") && out.contains("Checks") && out.contains("Diff"), "sub-tab bar");
        assert!(out.contains("Alice Ng"), "Conversation shows the author");
    }

    #[test]
    fn checks_tab_lists_named_checks_with_status() {
        use crate::app::Screen;
        let checks = vec![
            CheckRun { name: "build".into(), status: CheckStatus::Passed, url: None },
            CheckRun { name: "integration".into(), status: CheckStatus::Failed, url: None },
        ];
        let mut app = App::new("slate");
        app.screen = Screen::PrView(Box::new(pr_view(2, checks, vec![])));
        let out = render_to_string(&mut app, 100, 24);
        assert!(out.contains("build") && out.contains("Passed"), "named check + status");
        assert!(out.contains("integration") && out.contains("Failed"), "failed check shown by name");
    }

    #[test]
    fn commits_tab_lists_commits() {
        use crate::app::Screen;
        let mut app = App::new("slate");
        app.screen = Screen::PrView(Box::new(pr_view(1, vec![], vec![])));
        let out = render_to_string(&mut app, 120, 24);
        assert!(out.contains("abc1234"), "short sha");
        assert!(out.contains("Add the retry policy"), "commit message");
        assert!(out.contains("alice"), "commit author");
    }

    #[test]
    fn diff_tab_renders_the_diff_screen() {
        use crate::app::Screen;
        let files = vec![FileChange {
            path: "src/retry.rs".into(),
            kind: FileChangeKind::Added,
            additions: 2,
            deletions: 0,
            patch: Some("@@ -0,0 +1,2 @@\n+one\n+two\n".into()),
        }];
        let mut app = App::new("slate");
        app.screen = Screen::PrView(Box::new(pr_view(3, vec![], files)));
        let out = render_to_string(&mut app, 120, 30);
        assert!(out.contains("src/retry.rs"), "diff file list");
        assert!(out.contains("one"), "diff patch content (same as the d screen)");
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
    fn pr_write_actions_live_in_the_view_not_the_list() {
        // The PR list footer offers only browse/open — no write actions.
        let mut app = App::new("slate");
        app.prs.push(sample_pr());
        app.pr_state.select(Some(0));
        let list = render_to_string(&mut app, 140, 24);
        assert!(list.contains("open") && list.contains("filter"), "list keeps open/filter");
        for gone in ["approve", "reject", "merge"] {
            assert!(!list.contains(gone), "PR list footer should not advertise '{gone}'");
        }

        // Opening a PR (Enter) surfaces the write actions in the PR view footer.
        app.screen = Screen::PrView(Box::new(pr_view(0, vec![], vec![])));
        let view = render_to_string(&mut app, 140, 24);
        for label in ["approve", "reject", "merge", "comment"] {
            assert!(view.contains(label), "PR view footer should advertise '{label}'");
        }
    }

    #[test]
    fn help_overlay_lists_all_sections() {
        let mut app = App::new("slate");
        app.overlay = Some(crate::overlay::Overlay::Help { scroll: 0 });
        let out = render_to_string(&mut app, 100, 44);
        for expected in ["Keybindings", "Global", "Pull Requests", "PR view", "Pipelines", "Merge (choose strategy)"] {
            assert!(out.contains(expected), "help should show '{expected}'");
        }
    }

    fn sample_run() -> PipelineRun {
        PipelineRun {
            id: "r1".into(),
            definition_id: "ci".into(),
            number: Some(101),
            name: Some("CI".into()),
            status: PipelineRunStatus::Running,
            triggered_by: Some(User { id: "u".into(), display_name: "Dana".into(), handle: None, avatar_url: None }),
            branch: Some("main".into()),
            commit_sha: None,
            started_at: None,
            finished_at: None,
            url: None,
            stages: vec![PipelineStage {
                name: "Build".into(),
                status: PipelineRunStatus::Succeeded,
                jobs: vec![PipelineJob {
                    id: "j1".into(),
                    name: "compile".into(),
                    status: PipelineRunStatus::Succeeded,
                    started_at: None,
                    finished_at: None,
                    steps: vec![PipelineStep {
                        name: "cargo build".into(),
                        status: PipelineRunStatus::Succeeded,
                        started_at: None,
                        finished_at: None,
                    }],
                    url: None,
                    problem: None,
                }],
            }],
        }
    }

    #[test]
    fn pipeline_view_flattens_stage_job_step() {
        let view = PipelineView::new("CI #101".into(), sample_run(), "demo".into(), "ci".into(), Some("main".into()));
        let flat = view.flatten();
        assert_eq!(flat.len(), 3, "one stage + one job + one step");
        assert_eq!(flat[0].depth, 0);
        assert_eq!(flat[2].depth, 2);
    }

    #[test]
    fn pipeline_drill_in_renders_tree_and_keys() {
        use crate::app::Screen;
        let mut app = App::new("slate");
        app.screen = Screen::Pipeline(Box::new(PipelineView::new(
            "CI #101".into(),
            sample_run(),
            "demo".into(),
            "ci".into(),
            Some("main".into()),
        )));
        let out = render_to_string(&mut app, 120, 30);
        assert!(out.contains("Build"), "stage name");
        assert!(out.contains("compile"), "job name");
        assert!(out.contains("cargo build"), "step name");
        assert!(out.contains("expand") && out.contains("trigger"), "drill-in footer keys");
    }

    #[test]
    fn pipelines_footer_lists_drillin_and_trigger() {
        let mut app = App::new("slate");
        app.active = 2;
        let out = render_to_string(&mut app, 120, 24);
        assert!(out.contains("drill-in") && out.contains("trigger"), "pipelines footer");
    }

    #[test]
    fn config_screen_renders_connections_and_bindings() {
        use crate::app::{ConfigView, ConnRow, Screen};
        let mut app = App::new("slate");
        app.screen = Screen::Config(Box::new(ConfigView {
            connections: vec![ConnRow {
                id: "gh-1".into(),
                display: "My GitHub".into(),
                provider: ProviderType::GitHub,
                healthy: true,
                bindings: vec!["PR", "Pipe"],
            }],
            pr_binding: Some("My GitHub".into()),
            wi_binding: None,
            pipeline_subs: vec!["My GitHub".into()],
            selected: 0,
        }));
        let out = render_to_string(&mut app, 100, 24);
        assert!(out.contains("Connections"), "connections panel");
        assert!(out.contains("My GitHub"), "connection name");
        assert!(out.contains("Section bindings"), "bindings panel");
        assert!(out.contains("unbound"), "unbound work items shown");
        for label in ["add", "remove", "bind-PR"] {
            assert!(out.contains(label), "config footer should list '{label}'");
        }
    }

    #[test]
    fn empty_state_prompts_to_add_a_connection_on_first_run() {
        let mut app = App::new("slate"); // no connections/health, no data
        let out = render_to_string(&mut app, 100, 24);
        assert!(out.contains("add one"), "empty section should point to adding a connection");
        assert!(out.contains("add a connection"), "health bar should prompt to add a connection");
    }

    #[test]
    fn tab_bar_hides_hidden_sections() {
        let mut app = App::new("slate");
        app.apply_hidden_sections(&[forgetop_core::domain::Section::WorkItems]);
        let out = render_to_string(&mut app, 100, 24);
        assert!(out.contains("Pull Requests"), "PR tab shown");
        assert!(out.contains("Pipelines"), "Pipelines tab shown");
        assert!(!out.contains("Work Items"), "hidden Work Items tab absent");
    }

    #[test]
    fn visible_tabs_toggle_overlay_renders() {
        use crate::overlay::{Overlay, ToggleItem, ToggleKind};
        let mut app = App::new("slate");
        app.overlay = Some(Overlay::Toggle {
            title: "Visible tabs".into(),
            kind: ToggleKind::Sections,
            min_one: true,
            items: vec![
                ToggleItem { id: "0".into(), label: "Pull Requests".into(), on: true },
                ToggleItem { id: "1".into(), label: "Work Items".into(), on: false },
            ],
            selected: 0,
        });
        let out = render_to_string(&mut app, 100, 24);
        assert!(out.contains("Visible tabs"), "toggle title");
        assert!(out.contains("▶"), "green arrow marks a visible section");
        assert!(out.contains("toggle"), "toggle footer hint");
    }

    #[test]
    fn wizard_popup_renders_provider_choices() {
        use crate::wizard::Wizard;
        let mut app = App::new("slate");
        app.wizard = Some(Wizard::new());
        let out = render_to_string(&mut app, 100, 24);
        assert!(out.contains("Add connection"), "wizard title");
        assert!(out.contains("Provider"), "prompt label");
        assert!(out.contains("GitHub") && out.contains("Linear"), "provider options");
        assert!(out.contains("choose") && out.contains("cancel"), "wizard footer hints");
    }

    #[test]
    fn wizard_shows_per_field_help() {
        use crate::app::Key;
        use crate::wizard::Wizard;
        let mut w = Wizard::new();
        w.handle(Key::Enter); // pick provider (defaults to GitHub)
        w.handle(Key::Enter); // display name (pre-filled)
        w.handle(Key::Enter); // repository (optional, empty)
        // Now on the token field.
        let mut app = App::new("slate");
        app.wizard = Some(w);
        let out = render_to_string(&mut app, 120, 24);
        assert!(out.contains("Personal access token"), "token label");
        assert!(out.contains("github.com"), "help says where to create the token");
    }

    #[test]
    fn work_items_footer_lists_state_and_comment_not_pr_keys() {
        let mut app = App::new("slate");
        app.active = 1;
        let out = render_to_string(&mut app, 120, 24);
        assert!(out.contains("state") && out.contains("comment"), "WI footer should list state + comment");
        assert!(!out.contains("approve") && !out.contains("diff"), "PR-only keys hidden on WI tab");
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
