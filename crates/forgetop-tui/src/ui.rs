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
use crate::highlight::{lang_for, HlKind, LineHighlighter};
use crate::overlay::Overlay;
use crate::palette::{PaletteItem, PaletteKind, Tone};
use crate::theme::{check_icon, pipeline_glyph, Theme};
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
    // A saved-views bar sits above the list when the section has more than one view.
    if matches!(app.screen, Screen::List) && app.views[app.active].len() > 1 {
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(3)])
            .split(rows[1]);
        render_view_bar(frame, split[0], app);
        render_content(frame, split[1], app);
    } else {
        render_content(frame, rows[1], app);
    }
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
    // Refresh state now shows as an animated "Refreshing…" in the footer, not up here.
    let clock = app.last_refresh.format("%H:%M:%S");
    let right = format!("{} · {} ", theme.name, clock);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.bg))
        .title(Span::styled(" ▟ forgetop ", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)))
        .title_top(Line::from(Span::styled(right, Style::default().fg(theme.dim))).right_aligned());

    let vis = app.visible_indices();
    // Launchpad is tab 0; its badge is the count of items that actually need you.
    let lp_count = app.lp.iter().filter(|e| !e.bucket.muted()).count();
    let mut titles: Vec<Line> = vec![Line::from(format!(" Launchpad ({lp_count}) "))];
    titles.extend(vis.iter().map(|&i| {
        let count = match i {
            0 => app.prs.len(),
            1 => app.wis.len(),
            _ => app.pipes.len(),
        };
        Line::from(format!(" {} ({count}) ", TABS[i]))
    }));
    let selected = if matches!(app.screen, Screen::Launchpad) {
        0
    } else {
        1 + vis.iter().position(|&i| i == app.active).unwrap_or(0)
    };

    let tabs = Tabs::new(titles)
        .select(selected)
        .divider(Span::styled("  ", Style::default().fg(theme.dim)))
        .padding("", "")
        .style(Style::default().fg(theme.dim).bg(theme.bg))
        .highlight_style(Style::default().fg(theme.bg).bg(theme.accent).add_modifier(Modifier::BOLD))
        .block(block);

    frame.render_widget(tabs, area);
}

/// A horizontal strip of the active section's saved views, the current one lit.
fn render_view_bar(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let bar = Style::default().bg(theme.panel);
    let views = &app.views[app.active];
    let active = app.view_idx[app.active];

    let mut spans = vec![Span::styled(" ", bar)];
    for (i, v) in views.iter().enumerate() {
        let style = if i == active {
            bar.fg(theme.bg).bg(theme.accent).add_modifier(Modifier::BOLD)
        } else {
            bar.fg(theme.dim)
        };
        spans.push(Span::styled(format!(" {} ", v.name), style));
        spans.push(Span::styled(" ", bar));
    }
    spans.push(Span::styled("  [ ] views", bar.fg(theme.dim)));
    frame.render_widget(Paragraph::new(Line::from(spans)).style(bar), area);
}

fn render_content(frame: &mut Frame, area: Rect, app: &mut App) {
    match &app.screen {
        Screen::Launchpad => {
            render_launchpad(frame, area, app);
            return;
        }
        Screen::Pipeline(view) => {
            render_pipeline(frame, area, &app.theme, view, app.anim);
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

/// The Launchpad: two columns of urgency-ordered buckets — left "Needs you"
/// (ripe for action), right "Your work" (your backlog + parked PRs).
fn render_launchpad(frame: &mut Frame, area: Rect, app: &mut App) {
    let theme = &app.theme;
    if app.lp.is_empty() {
        let msg = if app.health.is_empty() { FIRST_RUN_HINT } else { "✓ You're all caught up — nothing needs you." };
        empty(frame, area, theme, msg, section_block(theme, "Launchpad · what needs you"));
        return;
    }
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    render_lp_column(frame, cols[0], app, 0, "Needs you");
    render_lp_column(frame, cols[1], app, 1, "Your work");
}

/// One Launchpad column: a bordered box stacking its buckets. The focused column
/// gets an accent border + a lit selection.
fn render_lp_column(frame: &mut Frame, area: Rect, app: &App, side: usize, title: &str) {
    use crate::launchpad::Bucket;
    let theme = &app.theme;
    let focused = app.lp_side == side;
    let border = if focused { theme.accent } else { theme.dim };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .title(Span::styled(format!(" {title} "), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)));

    let col = app.lp_column(side);
    if col.is_empty() {
        empty(frame, area, theme, "— nothing here —", block);
        return;
    }

    let content_w = (area.width.saturating_sub(2) as usize).saturating_sub(2); // borders + highlight symbol
    // One shared set of column widths for the whole side, so every row lines up.
    let rows_cells: Vec<Vec<Vec<Span>>> = col.iter().map(|&i| lp_cells(theme, &app.lp[i], app.anim)).collect();
    let widths = lp_widths(&rows_cells, 3, content_w);

    let mut items: Vec<ListItem> = Vec::new();
    let mut visual_sel = 0usize;
    let mut last: Option<Bucket> = None;
    for (pos, (&i, cells)) in col.iter().zip(rows_cells.iter()).enumerate() {
        let e = &app.lp[i];
        if last != Some(e.bucket) {
            let count = col.iter().filter(|&&j| app.lp[j].bucket == e.bucket).count();
            let rank = Bucket::ORDER.iter().position(|b| *b == e.bucket).unwrap_or(0);
            // All headings share one calm grey so they read uniformly as section labels.
            let style = Style::default().fg(theme.dim).add_modifier(Modifier::BOLD);
            if !items.is_empty() {
                items.push(ListItem::new(Line::from("")));
            }
            items.push(ListItem::new(Line::from(Span::styled(format!("{}  {} ({count})", CIRCLED[rank.min(7)], e.bucket.title()), style))));
            last = Some(e.bucket);
        }
        let selected = pos == app.lp_sel[side];
        if selected {
            visual_sel = items.len();
        }
        // On the focused row, any overflowing column (title, person) scrolls so it's readable.
        let line = if selected && focused {
            let mut c = cells.clone();
            for col in [LP_TITLE_COL, LP_PERSON_COL] {
                if cell_width(&c[col]) > widths[col] {
                    let text: String = c[col].iter().map(|s| s.content.as_ref()).collect();
                    let style = c[col].first().map(|s| s.style).unwrap_or_default();
                    c[col] = vec![Span::styled(marquee_window(&text, widths[col], app.anim / 2), style)];
                }
            }
            lp_cells_line(&c, &widths, content_w)
        } else {
            lp_cells_line(cells, &widths, content_w)
        };
        items.push(ListItem::new(line));
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(if focused { highlight(theme) } else { Style::default() })
        .highlight_symbol(if focused { "▐ " } else { "  " });
    let mut state = ListState::default();
    state.select(Some(visual_sel));
    frame.render_stateful_widget(list, area, &mut state);
}

/// Number of Launchpad row columns (kept in sync with [`lp_cells`]).
const LP_NCOL: usize = 7;
/// The flexible title column in a Launchpad row (the one that marquee-scrolls).
const LP_TITLE_COL: usize = 3;
/// The person column (author / who-ran / assignee); capped, and scrolls when selected.
const LP_PERSON_COL: usize = 5;
/// Max width for the person column before it truncates (and marquees on the focused row).
const LP_PERSON_MAX: usize = 16;
/// Gap between Launchpad columns — tighter than the nav lists since a column is half-width.
const LP_GAP: usize = 2;

/// The aligned cells for one Launchpad row. Every item type fills the *same* seven
/// slots — type · status · #ref · title · detail · person · age — so rows read as
/// siblings and line up vertically, even though PRs, pipelines and work items differ.
/// The "person" is the PR author / who ran the pipeline / the work-item assignee.
fn lp_cells(theme: &Theme, e: &crate::launchpad::Entry, anim: usize) -> Vec<Vec<Span<'static>>> {
    use crate::launchpad::EntryItem;
    let dim = Style::default().fg(theme.dim);
    let fg = Style::default().fg(theme.fg);
    // Kept calm: type badge, ref, person and age are all grey; only the status and the
    // git-diff +/- carry colour.
    let cell = |s: String, st: Style| vec![Span::styled(s, st)];
    let person = |u: Option<&forgetop_core::domain::User>| cell(u.map(|u| u.display_name.clone()).unwrap_or_else(|| "—".into()), dim);
    let age = |t| cell(rel_age(t), dim);
    match &e.item {
        EntryItem::Pr(pr) => {
            // Status is the PR's lifecycle state (Open / Draft / Merged / Closed).
            let (st, stc) = pr_status(theme, pr);
            vec![
                cell("PR".into(), dim),
                cell(st.to_string(), Style::default().fg(stc)),
                cell(pr.number.map(|n| format!("#{n}")).unwrap_or_default(), dim),
                cell(pr.title.clone(), fg),
                vec![
                    Span::styled(format!("+{}", pr.additions), Style::default().fg(theme.green)),
                    Span::raw(" "),
                    Span::styled(format!("-{}", pr.deletions), Style::default().fg(theme.red)),
                ],
                person(Some(&pr.author)),
                age(pr.updated_at),
            ]
        }
        EntryItem::Pipe { run, definition_name } => {
            let num = || run.number.map(|n| format!("#{n}")).unwrap_or_default();
            let (title, reference) = match definition_name {
                Some(def) => (def.clone(), run.name.clone().unwrap_or_else(num)),
                None => (run.name.clone().unwrap_or_else(|| run.definition_id.clone()), num()),
            };
            vec![
                cell("CI".into(), dim),
                cell(format!("{} {:?}", pipeline_glyph(run.status, anim), run.status), Style::default().fg(theme.pipeline_color(run.status))),
                cell(reference, dim),
                cell(title, fg),
                cell(run.branch.clone().unwrap_or_default(), dim),
                person(run.triggered_by.as_ref()),
                age(run.finished_at.or(run.started_at)),
            ]
        }
        EntryItem::Wi(wi) => vec![
            cell("WI".into(), dim),
            cell(format!("● {}", wi.state), Style::default().fg(wi_state_color(theme, &wi.state, wi.state_category))),
            cell(wi.identifier.clone().unwrap_or_default(), dim),
            cell(wi.title.clone(), fg),
            cell(wi.work_item_type.clone().unwrap_or_default(), dim),
            person(wi.assignee.as_ref()),
            age(wi.updated_at),
        ],
    }
}

/// Total display width of a Launchpad cell (its spans).
fn cell_width(cell: &[Span]) -> usize {
    cell.iter().map(|s| s.content.chars().count()).sum()
}

/// Sizes each Launchpad column to its widest cell, clamping the flexible title column
/// so the row still fits `inner_w`.
fn lp_widths(rows: &[Vec<Vec<Span>>], flex: usize, inner_w: usize) -> Vec<usize> {
    let mut w = vec![0usize; LP_NCOL];
    for row in rows {
        for (i, cell) in row.iter().enumerate().take(LP_NCOL) {
            w[i] = w[i].max(cell_width(cell));
        }
    }
    // Cap the person column so a long name doesn't crowd out the title (it scrolls instead).
    w[LP_PERSON_COL] = w[LP_PERSON_COL].min(LP_PERSON_MAX);
    let padding = COL_LEAD + LP_GAP * (LP_NCOL - 1);
    let fixed: usize = (0..LP_NCOL).filter(|&i| i != flex).map(|i| w[i]).sum::<usize>() + padding;
    w[flex] = w[flex].min(inner_w.saturating_sub(fixed)).max(3);
    w
}

/// A horizontally-scrolling window of `text`, `width` columns wide, advancing with
/// `frame`. Holds at the start briefly, then scrolls, wrapping past a gap — so a long
/// selected title can be read in full. Returns `text` unchanged when it already fits.
fn marquee_window(text: &str, width: usize, frame: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if width == 0 || chars.len() <= width {
        return text.to_string();
    }
    const GAP: usize = 4; // blank run between the end and the wrapped-around start
    const HOLD: usize = 2; // frames held at the start before scrolling (~1s at 300ms/frame)
    let gapped: Vec<char> = chars.into_iter().chain(std::iter::repeat_n(' ', GAP)).collect();
    let period = gapped.len();
    let pos = (frame % (period + HOLD)).saturating_sub(HOLD);
    (0..width).map(|i| gapped[(pos + i) % period]).collect()
}

/// Joins Launchpad cells into a line, each padded to its column width (so columns align),
/// then pads the whole line so a selected row highlights edge-to-edge.
fn lp_cells_line(cells: &[Vec<Span<'static>>], widths: &[usize], inner_w: usize) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")]; // COL_LEAD
    for (i, col) in cells.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" ".repeat(LP_GAP)));
        }
        // Render the cell's spans, truncating to the column width, then pad to it.
        let mut used = 0usize;
        for s in col {
            if used >= widths[i] {
                break;
            }
            let take = widths[i] - used;
            let t: String = s.content.chars().take(take).collect();
            used += t.chars().count();
            spans.push(Span::styled(t, s.style));
        }
        if used < widths[i] {
            spans.push(Span::raw(" ".repeat(widths[i] - used)));
        }
    }
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    if used < inner_w {
        spans.push(Span::raw(" ".repeat(inner_w - used)));
    }
    Line::from(spans)
}

/// Circled numerals for the bucket headers.
const CIRCLED: [&str; 8] = ["①", "②", "③", "④", "⑤", "⑥", "⑦", "⑧"];

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
            "number" => Some(2),
            "title" => Some(3),
            "author" => Some(4),
            "checks" => Some(5),
            "updated" => Some(7),
            _ => None,
        },
        1 => match key {
            "state" => Some(0),
            "title" => Some(3),
            "type" => Some(4),
            "assignee" => Some(5),
            "updated" => Some(6),
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

/// The Provider column value: "provider · connection", collapsed to just the provider
/// when the connection is named after it (e.g. a demo "GitHub" connection of type GitHub).
fn provider_tag(provider: ProviderType, connection: &str) -> String {
    if connection.eq_ignore_ascii_case(provider.as_str()) {
        provider.as_str().to_string()
    } else {
        format!("{} · {}", provider.as_str(), connection)
    }
}

/// A "Loading…" empty-state message with the refresh spinner, so a cold fetch looks live.
fn loading_msg(app: &App, base: &str) -> String {
    format!("{} {base}", crate::theme::SPINNER[app.anim % crate::theme::SPINNER.len()])
}

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
        // Green = healthy/done (open, merged); red = closed unmerged (worth a look).
        PullRequestStatus::Open => ("● open", theme.green),
        PullRequestStatus::Merged => ("✦ merged", theme.green),
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
    let base = format!("Pull Requests · {}", crate::app::pr_status_summary(&app.pr_shown_statuses));
    let title = list_title(base, &app.filters[0]);
    if idxs.is_empty() {
        let msg = if !app.filters[0].is_empty() {
            "No matches. Esc clears the filter.".to_string()
        } else if app.health.is_empty() {
            FIRST_RUN_HINT.to_string()
        } else if app.loading {
            loading_msg(app, "Loading pull requests…")
        } else {
            "No pull requests. Press f to change filter, r to refresh.".to_string()
        };
        empty(frame, area, theme, &msg, section_block(theme, &title));
        return;
    }

    let inner_w = area.width.saturating_sub(2) as usize;
    let dim = Style::default().fg(theme.dim).add_modifier(Modifier::BOLD);
    let headers = ["", "Provider", "#", "Title", "Author", "Checks", "±", "Updated"];
    let cells: Vec<Vec<(String, Style)>> = idxs
        .iter()
        .map(|&i| &app.prs[i])
        .map(|row| {
            let pr = &row.pr;
            let (st, stc) = pr_status(theme, pr);
            let (ck, ckc) = pr_checks(theme, pr);
            vec![
                (st.to_string(), Style::default().fg(stc)),
                (provider_tag(row.provider, &row.connection), Style::default().fg(theme.cyan)),
                (pr.number.map(|n| format!("#{n}")).unwrap_or_default(), Style::default().fg(theme.dim)),
                (pr.title.clone(), Style::default().fg(theme.fg)),
                (pr.author.display_name.clone(), Style::default().fg(theme.blue)),
                (ck, Style::default().fg(ckc)),
                (format!("+{} -{}", pr.additions, pr.deletions), Style::default().fg(theme.dim)),
                (rel_age(pr.updated_at), Style::default().fg(theme.dim)),
            ]
        })
        .collect();

    let (header, rows) = columnize(dim, &headers, &cells, 3, inner_w, sort_marker(app, 0));
    render_inline_list(frame, area, app, &title, header, rows);
}

// ---- Work Items ----

/// Green = done, blue = actively in progress, red = blocked (worth a look), grey =
/// waiting/neutral (backlog, todo, triage, canceled). "Blocked" is matched by name since
/// it isn't its own state category.
fn wi_state_color(theme: &Theme, state: &str, cat: WorkItemStateCategory) -> ratatui::style::Color {
    if state.eq_ignore_ascii_case("blocked") {
        return theme.red;
    }
    match cat {
        WorkItemStateCategory::Completed => theme.green,
        WorkItemStateCategory::Started => theme.blue,
        _ => theme.dim,
    }
}

fn render_wis(frame: &mut Frame, area: Rect, app: &mut App) {
    let theme = &app.theme;
    let idxs = app.filtered_wi_indices();
    let hidden_in_view = app.hidden_states_in_view();
    let base = if hidden_in_view == 0 {
        "Work Items · mine".to_string()
    } else {
        format!("Work Items · mine · {hidden_in_view} state(s) hidden")
    };
    let title = list_title(base, &app.filters[1]);
    if idxs.is_empty() {
        let msg = if !app.filters[1].is_empty() {
            "No matches. Esc clears the filter.".to_string()
        } else if hidden_in_view > 0 {
            "All present states are hidden. Press f to choose states.".to_string()
        } else if app.health.is_empty() {
            FIRST_RUN_HINT.to_string()
        } else if app.loading {
            loading_msg(app, "Loading work items…")
        } else {
            "No work items. Press r to refresh.".to_string()
        };
        empty(frame, area, theme, &msg, section_block(theme, &title));
        return;
    }

    let inner_w = area.width.saturating_sub(2) as usize;
    let dim = Style::default().fg(theme.dim).add_modifier(Modifier::BOLD);
    let headers = ["State", "Provider", "ID", "Title", "Type", "Assignee", "Updated"];
    let cells: Vec<Vec<(String, Style)>> = idxs
        .iter()
        .map(|&i| &app.wis[i])
        .map(|row| {
            let wi = &row.wi;
            vec![
                (format!("● {}", wi.state), Style::default().fg(wi_state_color(theme, &wi.state, wi.state_category))),
                (provider_tag(row.provider, &row.connection), Style::default().fg(theme.cyan)),
                (wi.identifier.clone().unwrap_or_default(), Style::default().fg(theme.dim)),
                (wi.title.clone(), Style::default().fg(theme.fg)),
                (wi.work_item_type.clone().unwrap_or_default(), Style::default().fg(theme.dim)),
                (wi.assignee.as_ref().map(|a| a.display_name.clone()).unwrap_or_else(|| "—".into()), Style::default().fg(theme.blue)),
                (rel_age(wi.updated_at), Style::default().fg(theme.dim)),
            ]
        })
        .collect();

    let (header, rows) = columnize(dim, &headers, &cells, 3, inner_w, sort_marker(app, 1));
    render_inline_list(frame, area, app, &title, header, rows);
}

// ---- Pipelines ----

fn render_pipes(frame: &mut Frame, area: Rect, app: &mut App) {
    let theme = &app.theme;
    let idxs = app.filtered_pipe_indices();
    let title = list_title("Pipelines".to_string(), &app.filters[2]);
    if idxs.is_empty() {
        let msg = if !app.filters[2].is_empty() {
            "No matches. Esc clears the filter.".to_string()
        } else if app.health.is_empty() {
            FIRST_RUN_HINT.to_string()
        } else if app.loading {
            loading_msg(app, "Loading pipeline runs…")
        } else {
            "No pipeline runs. Press r to refresh.".to_string()
        };
        empty(frame, area, theme, &msg, section_block(theme, &title));
        return;
    }

    let inner_w = area.width.saturating_sub(2) as usize;
    let dim = Style::default().fg(theme.dim).add_modifier(Modifier::BOLD);
    let headers = ["", "Provider", "Pipeline", "Run", "Branch", "Started", "Approval"];
    let cells: Vec<Vec<(String, Style)>> = idxs
        .iter()
        .map(|&i| &app.pipes[i])
        .map(|p| {
            let color = theme.pipeline_color(p.run.status);
            let approval = if p.awaiting_approval {
                ("approval needed".to_string(), Style::default().fg(theme.red).add_modifier(Modifier::BOLD))
            } else {
                (String::new(), Style::default().fg(theme.dim))
            };
            // Pipeline = definition name ("CI Build"); Run = the run/release ("10.1.100"),
            // or the run number when it has no name.
            let num = || p.run.number.map(|n| format!("#{n}")).unwrap_or_default();
            let pipeline = p.definition_name.clone().or_else(|| p.run.name.clone()).unwrap_or_else(|| p.run.definition_id.clone());
            let run = match &p.definition_name {
                Some(_) => p.run.name.clone().unwrap_or_else(num),
                None => num(),
            };
            vec![
                (format!("{} {:?}", pipeline_glyph(p.run.status, app.anim), p.run.status), Style::default().fg(color)),
                (provider_tag(p.provider, &p.connection), Style::default().fg(theme.cyan)),
                (pipeline, Style::default().fg(theme.fg)),
                (run, Style::default().fg(theme.dim)),
                (p.run.branch.clone().unwrap_or_default(), Style::default().fg(theme.dim)),
                (rel_age(p.run.started_at), Style::default().fg(theme.dim)),
                approval,
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

/// The PR sub-tab bar rendered as a row of pills (active one lit), each with a
/// count where it's meaningful (comment threads / commits / checks / changed files).
fn pr_tabs_line(theme: &Theme, view: &PrView) -> Line<'static> {
    // Conversation, Commits, Checks, Diff.
    let counts = [view.diff.threads.len(), view.commits.len(), view.checks.len(), view.diff.files.len()];
    let mut spans = vec![Span::raw(" ")];
    for (i, name) in PR_TABS.iter().enumerate() {
        let style = if i == view.tab {
            Style::default().fg(theme.bg).bg(theme.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.dim)
        };
        // Show the count only when there's something (avoids "Checks (0)" noise).
        let label = if counts[i] > 0 { format!(" {name} ({}) ", counts[i]) } else { format!(" {name} ") };
        spans.push(Span::styled(label, style));
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
    if let Some(desc) = pr.description.as_ref().filter(|d| !d.trim().is_empty()) {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Description", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))));
        for l in desc.lines() {
            lines.push(Line::from(Span::styled(l.to_string(), Style::default().fg(theme.fg))));
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
    frame.render_widget(Paragraph::new(pr_tabs_line(theme, view)), rows[1]);

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
        Span::styled(format!("● {}", wi.state), Style::default().fg(wi_state_color(theme, &wi.state, wi.state_category)).add_modifier(Modifier::BOLD)),
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
    if matches!(app.screen, Screen::Launchpad) {
        return vec![("↑↓", "move"), ("←→", "columns"), ("↵", "open"), ("Tab", "sections"), ("r", "refresh"), ("?", "help"), ("q", "quit")];
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
        return vec![("PgUp/Dn", "scroll"), ("u", "update state"), ("c", "comment"), ("o", "open"), ("Esc", "back"), ("q", "quit")];
    }
    if let Screen::Pipeline(v) = &app.screen {
        if v.logs.is_some() {
            return vec![("↑↓", "scroll"), ("PgUp/Dn", "jump"), ("Esc", "close logs")];
        }
        let mut keys = vec![("↑↓", "move"), ("↵", "expand"), ("L", "logs")];
        if v.can_respond_approvals && !v.actionable_approvals().is_empty() {
            keys.push(("A", "approve"));
        }
        keys.extend([("T", "trigger"), ("o", "open job"), ("Esc", "back"), ("q", "quit")]);
        return keys;
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
        0 => keys.extend([("↵", "open"), ("f", "status"), ("S", "sort"), ("o", "browser")]),
        1 => keys.extend([("↵", "open"), ("f", "states"), ("S", "sort"), ("o", "browser")]),
        2 => keys.extend([("↵", "drill-in"), ("S", "sort"), ("T", "trigger"), ("o", "open")]),
        _ => {}
    }
    if app.views[app.active].len() > 1 {
        keys.push(("[ ]", "views"));
    }
    keys.push(("V", "save view"));
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

    // Right side: a transient toast, an animated "Refreshing…" while a refresh is in
    // flight, else the standing status line.
    let (right, right_style) = if let Some(t) = &app.toast {
        (format!("{t} "), bar.fg(theme.yellow).add_modifier(Modifier::BOLD))
    } else if app.reloading {
        // Dots appear one at a time; padded to a constant width so nothing jitters.
        let n = (app.anim / 2) % 4;
        (format!("Refreshing{}{} ", ".".repeat(n), " ".repeat(3 - n)), bar.fg(theme.dim))
    } else {
        (format!("{} ", app.status), bar.fg(theme.dim))
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
    // File list on the left; the patch on the right renders comment threads inline,
    // beneath the lines they anchor to (unanchored threads live on the Conversation tab).
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(44), Constraint::Min(20)])
        .split(area);

    render_diff_files(frame, cols[0], theme, diff);
    render_diff_patch(frame, cols[1], theme, diff, pending);
}

/// The directory portion of a path (`""` for a root-level file).
fn dir_of(path: &str) -> &str {
    path.rfind('/').map(|i| &path[..i]).unwrap_or("")
}

/// The filename portion of a path.
fn base_of(path: &str) -> &str {
    path.rfind('/').map(|i| &path[i + 1..]).unwrap_or(path)
}

fn render_diff_files(frame: &mut Frame, area: Rect, theme: &Theme, diff: &DiffView) {
    let scope = match &diff.commit_label {
        Some(l) => format!("commit {l}"),
        None => diff.pr_label.clone(),
    };
    let title = format!("{scope} · files · {}/{} reviewed", diff.viewed_count(), diff.files.len());
    let block = section_block(theme, &title);
    if diff.files.is_empty() {
        empty(frame, area, theme, "No changed files.", block);
        return;
    }

    // Files are pre-sorted by path, so same-directory files are contiguous. Emit a dim
    // directory header whenever the directory changes; track the selected file's row so
    // the (file-indexed) selection lands on the right display row past the headers.
    let mut rows: Vec<Row> = Vec::new();
    let mut sel_row = 0usize;
    let mut last_dir: Option<&str> = None;
    for (i, f) in diff.files.iter().enumerate() {
        let dir = dir_of(&f.path);
        if last_dir != Some(dir) {
            let label = if dir.is_empty() { "(root)".into() } else { format!("{dir}/") };
            rows.push(Row::new(vec![
                Cell::from(""),
                Cell::from(""),
                Cell::from(Span::styled(label, Style::default().fg(theme.dim).add_modifier(Modifier::BOLD))),
                Cell::from(""),
            ]));
            last_dir = Some(dir);
        }
        if i == diff.selected {
            sel_row = rows.len();
        }
        let viewed = diff.is_viewed(&f.path);
        let name_style = if viewed { Style::default().fg(theme.dim) } else { Style::default().fg(theme.fg) };
        rows.push(Row::new(vec![
            Cell::from(Span::styled(if viewed { "[x]" } else { "[ ]" }, Style::default().fg(theme.dim))),
            Cell::from(kind_badge(theme, f.kind)),
            Cell::from(Span::styled(format!("  {}", base_of(&f.path)), name_style)),
            Cell::from(Span::styled(format!("+{} -{}", f.additions, f.deletions), Style::default().fg(theme.dim))),
        ]));
    }

    let widths = [Constraint::Length(3), Constraint::Length(1), Constraint::Min(10), Constraint::Length(10)];
    let table = Table::new(rows, widths)
        .block(block)
        .column_spacing(1)
        .row_highlight_style(highlight(theme))
        .highlight_symbol("▐ ");
    let mut state = TableState::default();
    state.select(Some(sel_row));
    frame.render_stateful_widget(table, area, &mut state);
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
    // Pending (unsaved) comments get a gutter bar.
    let marks = pending_marks(patch, &file.path, pending);
    // Existing threads grouped by the patch line they anchor to — rendered inline below it.
    let mut threads_at: std::collections::HashMap<usize, Vec<&CommentThread>> = std::collections::HashMap::new();
    for t in &diff.threads {
        if t.file_path.as_deref() == Some(file.path.as_str()) {
            if let Some(idx) = t.line.and_then(|l| crate::diff::patch_line_for_source_line(patch, l)) {
                threads_at.entry(idx).or_default().push(t);
            }
        }
    }
    // Pending (unsubmitted) comments, grouped by patch line via the same (line, side) map
    // that drives the gutter marks — so a draft shows inline before you submit it.
    let mut pending_at: std::collections::HashMap<usize, Vec<&LineComment>> = std::collections::HashMap::new();
    if !pending.is_empty() {
        for i in 0..patch.lines().count() {
            if let Some(t) = crate::diff::comment_target(patch, i) {
                for c in pending.iter().filter(|c| c.path == file.path && (c.line, c.side) == t) {
                    pending_at.entry(i).or_default().push(c);
                }
            }
        }
    }

    // One highlighter per file (regexes compile once); None for unhighlighted languages.
    let mut hl = lang_for(&file.path).and_then(LineHighlighter::new);

    // Build the display lines, splicing each thread in beneath the line it anchors to.
    // `cursor_row` tracks where the cursor's patch line landed (comment lines shift it).
    let mut lines: Vec<Line> = Vec::new();
    let mut cursor_row = 0usize;
    for (i, l) in patch.lines().enumerate() {
        let gutter = if marks.contains(&i) {
            Span::styled("▎", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))
        } else {
            Span::raw(" ")
        };
        if patch_focus && i == diff.cursor {
            cursor_row = lines.len();
            // Pad to full width (minus the gutter) so the highlight spans the row.
            let mut text = l.to_string();
            let w = text.chars().count();
            if w + 1 < inner_w {
                text.push_str(&" ".repeat(inner_w - 1 - w));
            }
            lines.push(Line::from(vec![
                gutter,
                Span::styled(text, Style::default().fg(patch_fg(theme, l)).bg(theme.sel_bg).add_modifier(Modifier::BOLD)),
            ]));
        } else {
            let mut line = patch_line_hl(theme, l, hl.as_mut());
            line.spans.insert(0, gutter);
            lines.push(line);
        }
        if let Some(ts) = threads_at.get(&i) {
            for &t in ts {
                let (glyph, state) = if t.is_resolved { ("○", "resolved") } else { ("●", "open") };
                let border = if t.is_resolved { theme.dim } else { theme.accent };
                let bodies: Vec<String> = t.comments.iter().map(|c| format!("{}: {}", c.author.display_name, c.body)).collect();
                lines.extend(inline_box(theme, border, &format!("{glyph} {state}"), &bodies, inner_w));
            }
        }
        if let Some(ps) = pending_at.get(&i) {
            let bodies: Vec<String> = ps.iter().map(|c| format!("you: {}", c.body)).collect();
            lines.extend(inline_box(theme, theme.blue, "● pending", &bodies, inner_w));
        }
    }

    // In the cursor, derive scroll so the cursor line stays roughly centred; the
    // stored scroll is only used for the free-scroll (file-list) mode.
    let scroll = if patch_focus {
        let half = inner_h / 2;
        cursor_row.saturating_sub(half).min(lines.len().saturating_sub(inner_h.max(1))) as u16
    } else {
        diff.scroll
    };

    frame.render_widget(Paragraph::new(lines).block(block).scroll((scroll, 0)), area);
}

/// Wrap `s` to `width` display columns on word boundaries.
fn wrap_words(s: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![s.to_string()];
    }
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        if cur.is_empty() {
            cur = word.to_string();
        } else if cur.chars().count() + 1 + word.chars().count() <= width {
            cur.push(' ');
            cur.push_str(word);
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_string();
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Render a comment — an existing thread or an unsent draft — as a bordered,
/// background-filled box shown inline beneath its diff line, so it's clearly distinct from
/// the code. `header` is the state line; `bodies` are the comment texts (`author: body`).
fn inline_box(theme: &Theme, border: ratatui::style::Color, header: &str, bodies: &[String], width: usize) -> Vec<Line<'static>> {
    let bg = theme.panel;
    let indent = "  ";
    let box_w = width.saturating_sub(3).max(20); // leaves a small right margin
    let content_w = box_w.saturating_sub(4); // inside "│ " … " │"
    let frame = |style: Style| style.fg(border).bg(bg);

    // Inner content rows (each a single styled string): a header, then wrapped comments.
    let mut rows: Vec<Span<'static>> = vec![Span::styled(header.to_string(), frame(Style::default()).add_modifier(Modifier::BOLD))];
    for b in bodies {
        for chunk in wrap_words(b, content_w) {
            rows.push(Span::styled(chunk, Style::default().fg(theme.fg).bg(bg)));
        }
    }

    let dashes = "─".repeat(box_w.saturating_sub(2));
    let mut out = vec![Line::from(vec![
        Span::raw(indent),
        Span::styled(format!("╭{dashes}╮"), frame(Style::default())),
    ])];
    for inner in rows {
        let fill = content_w.saturating_sub(inner.content.chars().count());
        out.push(Line::from(vec![
            Span::raw(indent),
            Span::styled("│ ", frame(Style::default())),
            inner,
            Span::styled(format!("{} │", " ".repeat(fill)), frame(Style::default())),
        ]));
    }
    out.push(Line::from(vec![
        Span::raw(indent),
        Span::styled(format!("╰{dashes}╯"), frame(Style::default())),
    ]));
    out
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

/// Semantic token kind → an indexed theme colour (never truecolor RGB).
fn hl_color(theme: &Theme, kind: HlKind) -> ratatui::style::Color {
    match kind {
        HlKind::Keyword => theme.magenta,
        HlKind::Type => theme.cyan,
        HlKind::Str => theme.green,
        HlKind::Comment => theme.dim,
        HlKind::Number => theme.yellow,
        HlKind::Func => theme.blue,
        HlKind::Punct | HlKind::Plain => theme.fg,
    }
}

/// Like [`patch_line`], but syntax-highlights the source after the diff marker when a
/// language highlighter is available. The `+`/`-`/context marker keeps its add/del colour;
/// headers and unknown languages fall back to the flat [`patch_line`].
fn patch_line_hl(theme: &Theme, line: &str, hl: Option<&mut LineHighlighter>) -> Line<'static> {
    let Some(hl) = hl else { return patch_line(theme, line) };
    if line.starts_with("@@") || line.starts_with("+++") || line.starts_with("---") {
        return patch_line(theme, line);
    }
    // Split the 1-char diff marker (ASCII) from the source it decorates.
    let (marker, source, marker_color) = match line.chars().next() {
        Some('+') => ("+", &line[1..], theme.green),
        Some('-') => ("-", &line[1..], theme.red),
        Some(' ') => (" ", &line[1..], theme.fg),
        _ => return patch_line(theme, line), // e.g. "\ No newline at end of file"
    };
    let mut spans = vec![Span::styled(marker.to_string(), Style::default().fg(marker_color))];
    for (text, kind) in hl.line(source) {
        spans.push(Span::styled(text, Style::default().fg(hl_color(theme, kind))));
    }
    Line::from(spans)
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

fn render_pipeline(frame: &mut Frame, area: Rect, theme: &Theme, view: &PipelineView, anim: usize) {
    // Reserve a banner row for approvals / unsupported note when there's one to show.
    let banner = approval_banner(theme, view);
    let mut constraints = vec![Constraint::Length(3)];
    if banner.is_some() {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Min(3));
    let rows = Layout::default().direction(Direction::Vertical).constraints(constraints).split(area);
    let tree_area = rows[rows.len() - 1];

    // Header: run identity + status + branch/trigger.
    let branch = view.branch.clone().unwrap_or_else(|| "—".into());
    let who = view.run.triggered_by.as_ref().map(|u| u.display_name.clone()).unwrap_or_else(|| "—".into());
    let header = Line::from(vec![
        Span::styled(format!("{} ", pipeline_glyph(view.run.status, anim)), Style::default().fg(theme.pipeline_color(view.run.status))),
        Span::styled(format!("{:?}", view.run.status), Style::default().fg(theme.pipeline_color(view.run.status)).add_modifier(Modifier::BOLD)),
        Span::styled(format!("   branch {branch}   triggered by {who}"), Style::default().fg(theme.dim)),
    ]);
    let header_block = section_block(theme, &view.title);
    frame.render_widget(Paragraph::new(header).block(header_block), rows[0]);

    if let Some(line) = banner {
        frame.render_widget(Paragraph::new(line), rows[1]);
    }

    // A log pane, when open, replaces the tree.
    if let Some(log) = &view.logs {
        let lines: Vec<Line> = log.lines.iter().map(|l| Line::from(Span::styled(l.clone(), Style::default().fg(theme.fg)))).collect();
        let inner_h = tree_area.height.saturating_sub(2);
        let max = (lines.len() as u16).saturating_sub(inner_h);
        frame.render_widget(
            Paragraph::new(lines).block(section_block(theme, &log.title)).scroll((log.scroll.min(max), 0)),
            tree_area,
        );
        return;
    }

    // Tree of stages → jobs → steps.
    let nodes = view.flatten();
    let tree_block = section_block(theme, "Stages · jobs · steps");
    if nodes.is_empty() {
        empty(frame, tree_area, theme, "No stages reported for this run.", tree_block);
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
                Span::styled(format!("{} ", pipeline_glyph(n.status, anim)), Style::default().fg(theme.pipeline_color(n.status))),
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
    frame.render_stateful_widget(list, tree_area, &mut state);
}

/// The approvals banner for the drill-in: pending gates you can act on, a
/// waiting-on-others note, or an explicit "unsupported" note (e.g. Bitbucket).
/// `None` when the provider supports approvals and there's nothing pending.
fn approval_banner<'a>(theme: &Theme, view: &PipelineView) -> Option<Line<'a>> {
    if !view.supports_approvals {
        return Some(Line::from(Span::styled(
            format!("  Approvals not supported on {}", view.provider.as_str()),
            Style::default().fg(theme.dim),
        )));
    }
    if view.approvals.is_empty() {
        return None;
    }
    let actionable = view.actionable_approvals();
    if actionable.is_empty() {
        let names = view.approvals.iter().map(|a| a.name.clone()).collect::<Vec<_>>().join(", ");
        return Some(Line::from(Span::styled(format!("  ⏸ Waiting on others: {names}"), Style::default().fg(theme.dim))));
    }
    let names = actionable.iter().map(|a| a.name.clone()).collect::<Vec<_>>().join(", ");
    // The gate is still surfaced when the provider is view-only (Azure) — just
    // without the "press A" hint, since we can't submit the decision here.
    let hint = if view.can_respond_approvals {
        "   press A to approve / reject"
    } else {
        "   view-only — approve in the provider's UI"
    };
    Some(Line::from(vec![
        Span::styled(format!("  ⏸ Approval needed: {names}"), Style::default().fg(theme.red).add_modifier(Modifier::BOLD)),
        Span::styled(hint, Style::default().fg(theme.dim)),
    ]))
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

    // The palette is a taller search panel (query line + windowed result list).
    if let Overlay::Palette { query, candidates, results, selected } = overlay {
        render_palette(frame, area, theme, query, candidates, results, *selected);
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
        Overlay::Help { .. } | Overlay::Palette { .. } => return, // handled above
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

/// Short type tag shown at the head of each palette row.
fn kind_tag(kind: PaletteKind) -> &'static str {
    match kind {
        PaletteKind::Pr => "PR",
        PaletteKind::Wi => "WI",
        PaletteKind::Pipe => "CI",
    }
}

/// The status dot's colour, following the shared green/blue/yellow/red/grey model.
fn tone_color(theme: &Theme, tone: Tone) -> ratatui::style::Color {
    match tone {
        Tone::Good => theme.green,
        Tone::Active => theme.blue,
        Tone::Warn => theme.yellow,
        Tone::Bad => theme.red,
        Tone::Neutral => theme.dim,
    }
}

/// Truncate to `max` display chars, adding an ellipsis when clipped.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

/// The command palette panel: a query line above a windowed, ranked result list.
fn render_palette(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    query: &str,
    candidates: &[PaletteItem],
    results: &[usize],
    selected: usize,
) {
    const MAX_ROWS: usize = 12;
    // Scroll the window so the selected row stays visible.
    let start = if selected >= MAX_ROWS { selected - MAX_ROWS + 1 } else { 0 };
    let end = (start + MAX_ROWS).min(results.len());

    let mut lines: Vec<Line> = Vec::new();

    let count = results.len();
    lines.push(Line::from(vec![
        Span::styled("> ", Style::default().fg(theme.accent)),
        Span::styled(query.to_string(), Style::default().fg(theme.fg)),
        Span::styled("█", Style::default().fg(theme.accent)),
        Span::styled(
            format!("    {count} match{}", if count == 1 { "" } else { "es" }),
            Style::default().fg(theme.dim),
        ),
    ]));
    lines.push(Line::from(""));

    if results.is_empty() {
        lines.push(Line::from(Span::styled("  No matches", Style::default().fg(theme.dim))));
    } else {
        for pos in start..end {
            let item = &candidates[results[pos]];
            let is_sel = pos == selected;
            let (cursor, title_style) = if is_sel {
                (" ▐ ", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))
            } else {
                ("   ", Style::default().fg(theme.fg))
            };
            let mut spans = vec![
                Span::styled(cursor, Style::default().fg(theme.accent)),
                Span::styled("● ", Style::default().fg(tone_color(theme, item.tone))),
                Span::styled(format!("{} ", kind_tag(item.kind)), Style::default().fg(theme.dim)),
                Span::styled(truncate(&item.title, 40), title_style),
            ];
            if !item.subtitle.is_empty() {
                spans.push(Span::styled(format!("  {}", truncate(&item.subtitle, 24)), Style::default().fg(theme.dim)));
            }
            lines.push(Line::from(spans));
        }
    }

    let hint = [("↑↓", "move"), ("↵", "open"), ("Esc", "cancel")]
        .into_iter()
        .flat_map(|(k, l)| {
            [
                Span::styled(format!(" {k} "), Style::default().fg(theme.bg).bg(theme.accent).add_modifier(Modifier::BOLD)),
                Span::styled(format!(" {l}   "), Style::default().fg(theme.dim)),
            ]
        })
        .collect::<Vec<_>>();
    lines.push(Line::from(""));
    lines.push(Line::from(hint));

    let height = lines.len() as u16 + 2;
    let width = 76.min(area.width.saturating_sub(6));
    let rect = centered_rect(width, height, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.panel))
        .title(Span::styled(" Jump to ", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)));

    frame.render_widget(Clear, rect);
    frame.render_widget(Paragraph::new(lines).block(block), rect);
}

/// Every keybinding, grouped by context — the content of the `?` help panel.
fn help_sections() -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    vec![
        (
            "Global",
            vec![
                ("←/→  h/l  Tab  1–3", "Switch tab"),
                ("↑/↓  k/j", "Move selection"),
                ("Ctrl-P", "Jump to any item (command palette)"),
                ("/", "Quick-filter the list"),
                ("S", "Sort by column (re-pick flips direction)"),
                ("o", "Open selected in browser"),
                ("n", "Add a connection (wizard)"),
                ("v", "Choose which tabs are visible"),
                ("C", "Config / connections"),
                ("r", "Refresh    t  cycle theme"),
                ("N", "Notifications — choose which events ping you"),
                ("?", "This help"),
                ("q  Ctrl-C", "Quit    Esc  back / close"),
            ],
        ),
        (
            "Saved views",
            vec![
                ("[  ]", "Previous / next saved view"),
                ("V", "Save the current filter + sort + states as a view"),
                ("X", "Delete the current view"),
            ],
        ),
        (
            "Pull Requests (list)",
            vec![
                ("Enter", "Open the PR view (all actions live there)"),
                ("f", "Filter by status (Open / Draft / Merged / Closed)"),
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
                ("v (Diff)", "Mark file viewed (updates N/M reviewed)"),
                ("[  ] (Diff)", "Jump to previous / next comment thread"),
                ("s", "Submit buffered line comments as a review"),
                ("o", "Open in browser"),
                ("Esc", "Step back (line → files → close; prompts if comments are unsubmitted)"),
            ],
        ),
        (
            "Work Items (list)",
            vec![
                ("Enter", "Open the item (actions live there)"),
                ("f", "Choose which states to show"),
            ],
        ),
        (
            "Work Item view (after Enter)",
            vec![
                ("u", "Update state (pulled from the provider)"),
                ("c", "Comment"),
                ("o", "Open in browser"),
            ],
        ),
        (
            "Pipelines",
            vec![
                ("Enter", "Drill in (stages → jobs → steps)"),
                ("Enter (in drill-in)", "Expand / collapse a node"),
                ("L", "View the selected job's logs"),
                ("A", "Approve / reject a waiting gate (GitHub, GitLab; Azure is view-only)"),
                ("o", "Open the selected job in the browser"),
                ("T", "Trigger a run"),
            ],
        ),
        (
            "Config / connections",
            vec![
                ("a", "Add a connection"),
                ("p  w", "Bind Pull Requests / Work Items (multi-select)"),
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

    #[test]
    fn marquee_scrolls_only_overflowing_text() {
        // Fits → returned unchanged (no scrolling).
        assert_eq!(marquee_window("short", 10, 3), "short");
        assert_eq!(marquee_window("short", 10, 99), "short");

        let text = "Tidy up the logging middleware";
        // Held at the start for the first few frames, exactly the window width.
        let start = marquee_window(text, 12, 0);
        assert_eq!(start.chars().count(), 12);
        assert_eq!(start, "Tidy up the ");
        assert_eq!(marquee_window(text, 12, 2), start, "held at start (~1s)");
        assert_ne!(marquee_window(text, 12, 3), start, "scrolls after the hold");
        // Later it has advanced (shows text further along).
        let later = marquee_window(text, 12, 12);
        assert_ne!(later, start);
        assert!(later.chars().count() == 12);
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
            connection_id: "c".into(),
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
                viewed: std::collections::HashSet::new(),
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
    fn pr_sub_tabs_show_counts() {
        use crate::app::Screen;
        let checks = vec![
            CheckRun { name: "build".into(), status: CheckStatus::Passed, url: None },
            CheckRun { name: "test".into(), status: CheckStatus::Passed, url: None },
        ];
        let file = |p: &str| FileChange { path: p.into(), kind: FileChangeKind::Added, additions: 1, deletions: 0, patch: None };
        let files = vec![file("a.rs"), file("b.rs"), file("c.rs")];
        let mut app = App::new("slate");
        app.screen = Screen::PrView(Box::new(pr_view(0, checks, files)));
        let out = render_to_string(&mut app, 120, 24);
        assert!(out.contains("Commits (1)"), "commits count");
        assert!(out.contains("Checks (2)"), "checks count");
        assert!(out.contains("Diff (3)"), "changed-file count");
        assert!(!out.contains("Conversation (0)"), "no zero-count noise on empty tabs");
    }

    #[test]
    fn wrap_words_breaks_on_word_boundaries() {
        assert_eq!(wrap_words("hello world foo", 11), vec!["hello world", "foo"]);
        assert_eq!(wrap_words("", 10), vec![String::new()]);
        assert_eq!(wrap_words("onelongword", 4), vec!["onelongword"]); // never splits a word
    }

    #[test]
    fn diff_patch_renders_comment_threads_inline() {
        use crate::app::Screen;
        use forgetop_core::domain::{Comment, CommentThread};
        let file = FileChange {
            path: "a.rs".into(),
            kind: FileChangeKind::Added,
            additions: 2,
            deletions: 0,
            patch: Some("@@ -0,0 +1,2 @@\n+let n = 5;\n+// done".into()),
        };
        let mut view = pr_view(3, vec![], vec![file]);
        view.diff.threads = vec![CommentThread {
            id: "t1".into(),
            comments: vec![Comment {
                id: "c1".into(),
                author: User { id: "u".into(), display_name: "Priya".into(), handle: None, avatar_url: None },
                body: "cap the backoff here".into(),
                created_at: None,
            }],
            file_path: Some("a.rs".into()),
            line: Some(1),
            is_resolved: false,
        }];
        let mut app = App::new("slate");
        app.screen = Screen::PrView(Box::new(view));

        let out = render_to_string(&mut app, 150, 24);
        assert!(out.contains("let n = 5"), "the code line still renders");
        assert!(out.contains("Priya"), "the comment author renders inline in the patch");
        assert!(out.contains("cap the backoff"), "the comment body renders inline in the patch");
        assert!(out.contains("open"), "the thread state marker renders");
        assert!(out.contains("╭") && out.contains("╰"), "the comment is drawn in its own box");
    }

    #[test]
    fn diff_patch_renders_pending_draft_inline() {
        use crate::app::{DiffFocus, Screen};
        use forgetop_core::domain::{DiffSide, LineComment};
        let file = FileChange {
            path: "a.rs".into(),
            kind: FileChangeKind::Added,
            additions: 2,
            deletions: 0,
            patch: Some("@@ -0,0 +1,2 @@\n+let n = 5;\n+// done".into()),
        };
        let mut view = pr_view(3, vec![], vec![file]);
        view.diff.focus = DiffFocus::Patch;
        view.pending = vec![LineComment { path: "a.rs".into(), line: 1, side: DiffSide::New, body: "hold off on this".into() }];
        let mut app = App::new("slate");
        app.screen = Screen::PrView(Box::new(view));

        let out = render_to_string(&mut app, 150, 24);
        assert!(out.contains("pending"), "an unsubmitted comment shows a pending box");
        assert!(out.contains("hold off on this"), "the draft body renders inline");
        assert!(out.contains("╭"), "the draft is boxed like a real comment");
    }

    #[test]
    fn diff_file_list_groups_by_dir_and_shows_viewed_progress() {
        use crate::app::Screen;
        let file = |p: &str| FileChange { path: p.into(), kind: FileChangeKind::Modified, additions: 1, deletions: 0, patch: None };
        // Pre-sorted (as the app does at open); two under src/, one at root.
        let files = vec![file("README.md"), file("src/a.rs"), file("src/b.rs")];
        let mut view = pr_view(3, vec![], files);
        view.diff.viewed.insert("src/a.rs".into());
        let mut app = App::new("slate");
        app.screen = Screen::PrView(Box::new(view));

        let out = render_to_string(&mut app, 150, 24);
        assert!(out.contains("1/3 reviewed"), "progress in the title");
        assert!(out.contains("src/"), "directory header");
        assert!(out.contains("[x]"), "a viewed file's checkbox is ticked");
        assert!(out.contains("[ ]"), "an unviewed file's checkbox is empty");
    }

    #[test]
    fn conversation_shows_the_pr_description() {
        use crate::app::Screen;
        let mut app = App::new("slate");
        app.screen = Screen::PrView(Box::new(pr_view(0, vec![], vec![]))); // sample_pr has a description
        let out = render_to_string(&mut app, 120, 24);
        assert!(out.contains("Description"), "the description has a heading");
        assert!(out.contains("does the thing"), "the description body renders");
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
    fn pr_view_bar_shows_saved_views() {
        let mut app = App::new("slate");
        app.screen = Screen::List;
        app.apply_views(vec![], vec![], vec![]); // seed the default PR views
        app.prs.push(crate::app::PrRow { connection_id: "c".into(), connection: "GH".into(), provider: ProviderType::GitHub, pr: sample_pr() });
        app.pr_state.select(Some(0));

        let out = render_to_string(&mut app, 100, 24);
        assert!(out.contains("All") && out.contains("Mine") && out.contains("Review"), "the view bar lists the default PR views");
        assert!(out.contains("views"), "footer/bar advertises view switching");
    }

    #[test]
    fn merge_picker_overlay_renders_over_the_list() {
        use crate::overlay::{Overlay, PickerKind};
        let mut app = App::new("slate");
        app.prs.push(crate::app::PrRow { connection_id: "c".into(), connection: "GH".into(), provider: ProviderType::GitHub, pr: sample_pr() });
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
    fn provider_tag_collapses_when_connection_is_the_provider() {
        // A connection named after its provider shows just the provider (no "GitHub · GitHub").
        assert_eq!(provider_tag(ProviderType::GitHub, "GitHub"), "GitHub");
        assert_eq!(provider_tag(ProviderType::GitLab, "gitlab"), "GitLab");
        // Otherwise it disambiguates with the connection name.
        assert_eq!(provider_tag(ProviderType::GitHub, "acme-corp"), "GitHub · acme-corp");
    }

    #[test]
    fn pr_list_shows_the_provider_column_for_aggregation() {
        let mut app = App::new("slate");
        app.screen = Screen::List;
        app.prs.push(crate::app::PrRow {
            connection_id: "c".into(),
            connection: "MyHub".into(),
            provider: ProviderType::GitHub,
            pr: sample_pr(),
        });
        app.pr_state.select(Some(0));
        let out = render_to_string(&mut app, 140, 24);
        assert!(out.contains("Provider"), "provider header present");
        assert!(out.contains("GitHub") && out.contains("MyHub"), "row is tagged with its provider · connection");
    }

    #[test]
    fn pr_write_actions_live_in_the_view_not_the_list() {
        // The PR list footer offers only browse/open — no write actions.
        let mut app = App::new("slate");
        app.screen = Screen::List;
        app.prs.push(crate::app::PrRow { connection_id: "c".into(), connection: "GH".into(), provider: ProviderType::GitHub, pr: sample_pr() });
        app.pr_state.select(Some(0));
        let list = render_to_string(&mut app, 140, 24);
        assert!(list.contains("open") && list.contains("browser"), "list keeps browse/open");
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
        let view = PipelineView::new("CI #101".into(), sample_run(), "demo".into(), ProviderType::GitHub, "ci".into(), Some("main".into()));
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
            ProviderType::GitHub,
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
    fn drill_in_banner_flags_approval_needed_and_unsupported() {
        use crate::app::Screen;

        // A GitHub run with a gate I can action → red "Approval needed" banner + A key.
        let mut app = App::new("slate");
        let mut view = PipelineView::new("CI #101".into(), sample_run(), "demo".into(), ProviderType::GitHub, "ci".into(), None);
        view.supports_approvals = true;
        view.can_respond_approvals = true;
        view.approvals = vec![PipelineApproval { id: "prod".into(), name: "production".into(), can_respond: true }];
        app.screen = Screen::Pipeline(Box::new(view));
        let out = render_to_string(&mut app, 120, 30);
        assert!(out.contains("Approval needed") && out.contains("production"), "actionable gate banner");
        assert!(out.contains("press A") && out.contains("approve"), "actionable footer + press-A hint");

        // A Bitbucket run → explicit unsupported note.
        let mut app = App::new("slate");
        let view = PipelineView::new("Deploy".into(), sample_run(), "bb".into(), ProviderType::Bitbucket, "ci".into(), None);
        app.screen = Screen::Pipeline(Box::new(view));
        let out = render_to_string(&mut app, 120, 30);
        assert!(out.contains("not supported on Bitbucket"), "bitbucket approvals unsupported note");
    }

    #[test]
    fn view_only_provider_shows_gate_without_approve_action() {
        // Azure: surfaces the pending gate but no `A` action (respond isn't possible).
        let mut app = App::new("slate");
        let mut view = PipelineView::new("Deploy".into(), sample_run(), "az".into(), ProviderType::AzureDevOps, "ci".into(), None);
        view.supports_approvals = true;
        view.can_respond_approvals = false;
        view.approvals = vec![PipelineApproval { id: "prod".into(), name: "production".into(), can_respond: true }];
        app.screen = Screen::Pipeline(Box::new(view));
        let out = render_to_string(&mut app, 120, 30);
        assert!(out.contains("Approval needed") && out.contains("production"), "gate still surfaced");
        assert!(out.contains("view-only"), "banner marks it view-only");
        assert!(!out.contains("press A"), "no press-A hint when view-only");
    }

    #[test]
    fn pipelines_list_flags_approval_needed_column() {
        let mut app = App::new("slate");
        app.screen = Screen::List;
        app.active = 2;
        app.pipes.push(crate::app::PipeRow {
            connection_id: "c".into(),
            connection: "GH".into(),
            provider: ProviderType::GitHub,
            definition_name: Some("CI Build".into()),
            awaiting_approval: true,
            run: sample_run(),
        });
        app.pipe_state.select(Some(0));
        let out = render_to_string(&mut app, 150, 24);
        assert!(out.contains("Approval"), "approval column header present");
        assert!(out.contains("approval needed"), "the awaiting row is flagged");
    }

    #[test]
    fn pipelines_footer_lists_drillin_and_trigger() {
        let mut app = App::new("slate");
        app.screen = Screen::List;
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
    fn launchpad_renders_two_columns_with_typed_rows() {
        use crate::launchpad::{Bucket, Entry, EntryItem};
        let entry = |bucket, item| Entry {
            bucket,
            connection_id: "c".into(),
            connection: "GH".into(),
            provider: ProviderType::GitHub,
            item,
        };
        let pr = {
            let mut p = sample_pr();
            p.title = "Add retry policy".into();
            p
        };
        let run = {
            let mut r = sample_run();
            r.name = Some("nightly".into());
            r
        };
        let wi = WorkItem {
            id: "w".into(),
            identifier: Some("FOR-1".into()),
            title: "Investigate flake".into(),
            description: None,
            state: "Todo".into(),
            state_category: WorkItemStateCategory::Unstarted,
            work_item_type: Some("Bug".into()),
            assignee: None,
            created_at: None,
            updated_at: None,
            url: None,
        };
        let mut app = App::new("slate"); // defaults to the Launchpad screen
        app.lp = vec![
            entry(Bucket::NeedsReview, EntryItem::Pr(pr)),
            entry(Bucket::NeedsFixing, EntryItem::Pipe { run, definition_name: Some("CI Build".into()) }),
            entry(Bucket::YourWork, EntryItem::Wi(wi)),
        ];
        let out = render_to_string(&mut app, 140, 24);
        // Two named columns.
        assert!(out.contains("Needs you") && out.contains("Your work"), "two columns");
        // Buckets land in the right columns.
        assert!(out.contains("Needs your review") && out.contains("Needs fixing") && out.contains("Assigned to you"), "bucket headers");
        // Every row carries a type badge …
        assert!(out.contains("PR") && out.contains("CI") && out.contains("WI"), "type badges for each kind");
        // … and nav-style detail: PR number + change stats, WI id/state/type, pipeline branch.
        assert!(out.contains("#42") && out.contains("+10 -2"), "PR row shows number and change stats");
        assert!(out.contains("FOR-1") && out.contains("Todo") && out.contains("Bug"), "work-item row shows id, state, type");
        assert!(out.contains("CI Build") && out.contains("main"), "pipeline row shows pipeline name + branch");
        // The person column shows the PR author (not the provider).
        assert!(out.contains("Alice Ng"), "PR row shows the author");
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
    fn work_items_list_is_browse_only_actions_in_the_view() {
        // The WI list offers browse + the states filter — no state-change / comment.
        let mut app = App::new("slate");
        app.screen = Screen::List;
        app.active = 1;
        let list = render_to_string(&mut app, 140, 24);
        assert!(list.contains("open") && list.contains("states"), "list keeps open + states filter");
        assert!(!list.contains("comment") && !list.contains("update state"), "state/comment are not on the list");

        // Opening the item surfaces update-state + comment in its footer.
        app.screen = Screen::WiView(Box::new(crate::app::WiView {
            connection_id: "c".into(),
            wi: WorkItem {
                id: "w1".into(),
                identifier: Some("FOR-1".into()),
                title: "A task".into(),
                description: None,
                state: "Todo".into(),
                state_category: WorkItemStateCategory::Unstarted,
                work_item_type: None,
                assignee: None,
                created_at: None,
                updated_at: None,
                url: None,
            },
            threads: vec![],
            scroll: 0,
        }));
        let view = render_to_string(&mut app, 140, 24);
        assert!(view.contains("update state") && view.contains("comment"), "view has update-state + comment");
    }

    #[test]
    fn toast_renders_in_footer() {
        let mut app = App::new("slate");
        app.toast = Some("Filter: mine (1 PRs)".into());
        let out = render_to_string(&mut app, 100, 24);
        assert!(out.contains("Filter: mine"), "toast should appear in the footer");
    }

    #[test]
    fn refreshing_shows_in_the_footer_not_the_header() {
        let mut app = App::new("slate");
        app.status = "9 PRs · 10 work items · 8 runs".into();
        app.reloading = true;
        let out = render_to_string(&mut app, 100, 24);
        assert!(out.contains("Refreshing"), "a refresh shows 'Refreshing…' in the footer");
        // The header keeps just the theme + clock — no refresh glyph up there.
        assert!(!out.contains("⟳"), "no spinner in the top-right");
    }

    #[test]
    fn patch_line_highlights_code_after_the_diff_marker() {
        use crate::highlight::{Lang, LineHighlighter};
        let theme = Theme::by_name("slate");
        let mut hl = LineHighlighter::new(Lang::Rust).unwrap();
        let line = patch_line_hl(&theme, "+let n = 5;", Some(&mut hl));

        // The add marker keeps its green, separate from the code.
        assert_eq!(line.spans[0].content, "+");
        assert_eq!(line.spans[0].style.fg, Some(theme.green));
        // `let` is a keyword (magenta); `5` is a number (yellow).
        let kw = line.spans.iter().find(|s| s.content.contains("let")).expect("a `let` span");
        assert_eq!(kw.style.fg, Some(theme.magenta), "keyword is magenta");
        assert!(
            line.spans.iter().any(|s| s.content.contains('5') && s.style.fg == Some(theme.yellow)),
            "number is yellow"
        );
    }

    #[test]
    fn patch_line_headers_and_unknown_langs_stay_flat() {
        use crate::highlight::{Lang, LineHighlighter};
        let theme = Theme::by_name("slate");
        let mut hl = LineHighlighter::new(Lang::Rust).unwrap();
        // A hunk header stays accent even with a highlighter available.
        let hdr = patch_line_hl(&theme, "@@ -1 +1 @@", Some(&mut hl));
        assert_eq!(hdr.spans[0].style.fg, Some(theme.accent));
        // No highlighter (unknown language) → one flat context span.
        let plain = patch_line_hl(&theme, " untouched", None);
        assert_eq!(plain.spans.len(), 1);
        assert_eq!(plain.spans[0].style.fg, Some(theme.fg));
    }

    #[test]
    fn truncate_clips_long_text_with_an_ellipsis() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("exactly-ten", 11), "exactly-ten");
        assert_eq!(truncate("this is far too long", 8), "this is…");
    }

    #[test]
    fn palette_renders_query_results_and_status_dots() {
        use crate::overlay::Overlay;
        use crate::palette::{self, PaletteItem, PaletteKind, Tone};

        let items = vec![
            PaletteItem {
                kind: PaletteKind::Pr,
                id: "1".into(),
                connection_id: "c".into(),
                title: "Add the widget".into(),
                subtitle: "alice · GitHub".into(),
                tone: Tone::Good,
                sort_ts: None,
            },
            PaletteItem {
                kind: PaletteKind::Pipe,
                id: "2".into(),
                connection_id: "c".into(),
                title: "CI Build".into(),
                subtitle: "main".into(),
                tone: Tone::Bad,
                sort_ts: None,
            },
        ];
        let results = palette::rank("", &items);
        let mut app = App::new("slate");
        app.overlay = Some(Overlay::Palette { query: "a".into(), candidates: items, results, selected: 0 });

        let out = render_to_string(&mut app, 100, 30);
        assert!(out.contains("Jump to"), "panel title");
        assert!(out.contains("Add the widget"), "a result title is shown");
        assert!(out.contains("CI Build"), "results from every type are shown");
        assert!(out.contains("match"), "the match count is shown");
        assert!(out.contains("● "), "status dots are rendered");
        assert!(out.contains("PR") && out.contains("CI"), "type badges are shown");
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
