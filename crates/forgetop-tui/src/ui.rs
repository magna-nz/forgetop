//! All rendering. Immediate-mode: we redraw the whole frame from `App` each tick.

use chrono::{DateTime, Utc};
use forgetop_core::domain::*;
use forgetop_core::launchpad::{pr_approved_by, pr_changes_requested_by, pr_state, PrBlocker, PrState};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState, Tabs, Wrap,
};
use ratatui::Frame;

use crate::app::{
    dashboard_target, is_error_line, match_ranges, pipe_definition_name, App, ConfigView, DiffFocus, DiffView, Hit, LogView,
    LpSlot, PipeGroup, PipeHead, PipeLine, PipeRow, PipelineView, PrView, Screen, WiView, LOG_SPLIT_MIN_WIDTH, LOG_TREE_WIDTH,
    PR_TABS, TABS,
};
use crate::diff::{cursor_line_label, pending_marks};
use crate::highlight::{lang_for, HlKind, LineHighlighter};
use crate::overlay::Overlay;
use crate::palette::{parse_query, PaletteItem, Tone};
use crate::theme::{check_icon, pipeline_glyph, Theme};
use crate::wizard::{Prompt, PromptKind};

/// Shown in empty sections when nothing is configured yet.
const FIRST_RUN_HINT: &str = "No connections yet — press n to add one, or C for config.";

/// Shown while setup has been handed to the browser and no connection has landed yet.
const AWAITING_SETUP_TITLE: &str = " Setting up in your browser ";

thread_local! {
    /// The frame's clickable regions, collected as it draws and handed to `App::hits` at the end.
    static HITS: std::cell::RefCell<Vec<(Rect, Hit)>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Set while drawing something that only looks like a screen — the unfocused preview — so
    /// its rows and tabs don't answer clicks meant for the list that has the keys.
    static HITS_MUTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Records that `rect` (clipped to a non-empty area) shows `target` in this frame.
fn hit(rect: Rect, target: Hit) {
    if rect.width > 0 && rect.height > 0 && !HITS_MUTED.with(|m| m.get()) {
        HITS.with(|h| h.borrow_mut().push((rect, target)));
    }
}

/// Records one-row hits for the visible rows of a scrolled list: row `r` of `targets` is drawn
/// at `body.y + r - offset`. `None` rows (headings, spacers) take no clicks.
fn hit_rows(body: Rect, offset: usize, targets: impl IntoIterator<Item = Option<Hit>>) {
    for (r, target) in targets.into_iter().enumerate().skip(offset) {
        let dy = r - offset;
        if dy >= body.height as usize {
            break;
        }
        if let Some(t) = target {
            hit(Rect { y: body.y + dy as u16, height: 1, ..body }, t);
        }
    }
}

pub fn render(frame: &mut Frame, app: &mut App) {
    HITS.with(|h| h.borrow_mut().clear());
    render_frame(frame, app);
    app.hits = HITS.with(|h| std::mem::take(&mut *h.borrow_mut()));
}

fn render_frame(frame: &mut Frame, app: &mut App) {
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
    // A saved-views bar sits above the list when the section has more than one view. A focused
    // preview still has the list beside it, so the bar stays and the layout doesn't jump.
    if (matches!(app.screen, Screen::List) || app.preview_focus) && app.views[app.active].len() > 1 {
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
    } else if app.awaiting_browser_setup {
        render_awaiting_setup(frame, area, app);
    }
}

/// The "waiting for setup in the browser" card. Ranks below the wizard and any overlay:
/// if the user has opened either, that is what they are doing now.
fn render_awaiting_setup(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let accent = theme.accent;

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        "Add your provider access tokens in the dashboard.",
        Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    match app.dashboard_url.as_deref() {
        Some(url) => lines.push(Line::from(vec![
            Span::styled("It should be open at ", Style::default().fg(theme.dim)),
            Span::styled(dashboard_target(url, "#settings"), Style::default().fg(accent)),
        ])),
        None => lines.push(Line::from(Span::styled(
            "Start it with `forgetop --dashboard`.",
            Style::default().fg(theme.dim),
        ))),
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "This screen updates on its own as soon as a connection is saved.",
        Style::default().fg(theme.dim),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(" n ", Style::default().fg(theme.bg).bg(accent).add_modifier(Modifier::BOLD)),
        Span::styled("  set up here instead   ", Style::default().fg(theme.dim)),
        Span::styled(" B ", Style::default().fg(theme.bg).bg(accent).add_modifier(Modifier::BOLD)),
        Span::styled("  reopen the dashboard", Style::default().fg(theme.dim)),
    ]));

    let height = lines.len() as u16 + 3;
    let width = 72.min(area.width.saturating_sub(6));
    let rect = centered_rect(width, height, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent))
        .style(Style::default().bg(theme.panel))
        .title(Span::styled(AWAITING_SETUP_TITLE, Style::default().fg(accent).add_modifier(Modifier::BOLD)));

    frame.render_widget(Clear, rect);
    frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), rect);
}

fn render_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let clock = app.last_refresh.format("%H:%M:%S");
    let right = format!("{} · {} ", theme.name, clock);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.bg))
        .title(Span::styled(concat!(" ▟ forgetop v", env!("CARGO_PKG_VERSION"), " "), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)))
        .title_top(Line::from(Span::styled(right, Style::default().fg(theme.dim))).right_aligned());

    let vis = app.visible_indices();
    let titles: Vec<Line> = tab_titles(app).into_iter().map(Line::from).collect();
    let selected = if matches!(app.screen, Screen::Inbox) {
        titles.len() // Inbox isn't a section tab — highlight none of them
    } else if matches!(app.screen, Screen::Launchpad) {
        0
    } else {
        // An open item lights its own section, not whatever list is behind it: a PR opened
        // from the Command Center belongs to Pull Requests, which is also where Tab leaves from.
        1 + vis.iter().position(|&i| i == app.screen_section()).unwrap_or(0)
    };

    let tabs = Tabs::new(titles)
        .select(selected)
        .divider(Span::styled("  ", Style::default().fg(theme.dim)))
        .padding("", "")
        .style(Style::default().fg(theme.dim).bg(theme.bg))
        .highlight_style(Style::default().fg(theme.bg).bg(theme.accent).add_modifier(Modifier::BOLD))
        .block(block);

    frame.render_widget(tabs, area);
    // Tabs lays titles out from the inner left edge, each followed by the two-column divider.
    let mut x = area.x + 1;
    for (pos, title) in tab_titles(app).iter().enumerate() {
        let w = (title.chars().count() as u16).min(area.right().saturating_sub(1).saturating_sub(x));
        hit(Rect { x, y: area.y + 1, width: w, height: 1 }, Hit::Tab(pos));
        x = x.saturating_add(w + 2);
    }

    // How to move along the strip, just past its last tab — dim, and only where it can't run
    // into the right-hand chrome (Refreshing…, Notifications).
    let strip_w: usize = tab_titles(app).iter().map(|t| t.chars().count()).sum::<usize>() + 2 * vis.len();
    // Drawn as a key chip, like the footer's, so it reads as "press this" rather than a label.
    let hint = Line::from(vec![
        Span::styled("   ", Style::default().bg(theme.bg)),
        Span::styled(" Tab ", Style::default().fg(theme.bg).bg(theme.accent).add_modifier(Modifier::BOLD)),
        Span::styled(" next section", Style::default().fg(theme.dim).bg(theme.bg)),
    ]);
    let hint_w = hint.width();
    let right_w = notifications_label(app).chars().count()
        + refreshing_in_tab_row(app, area.width).map_or(0, |t| t.chars().count());
    let inner = area.width.saturating_sub(2) as usize;
    if strip_w + hint_w + 2 + right_w <= inner {
        let rect = Rect { x: area.x + 1 + strip_w as u16, y: area.y + 1, width: hint_w as u16, height: 1 };
        frame.render_widget(Paragraph::new(hint), rect);
    }

    // Notifications is a nav item pinned to the far right of the tab row — highlighted when
    // its screen is open, but *not* part of the Tab cycle. Dim grey at (0), bold yellow
    // when there's something, accent when active.
    let unread = app.unread_count();
    let label = notifications_label(app);
    let style = if matches!(app.screen, Screen::Inbox) {
        Style::default().fg(theme.bg).bg(theme.accent).add_modifier(Modifier::BOLD)
    } else if unread == 0 {
        Style::default().fg(theme.dim).bg(theme.bg)
    } else {
        Style::default().fg(theme.yellow).bg(theme.bg).add_modifier(Modifier::BOLD)
    };
    let w = label.chars().count() as u16;
    let inner_w = area.width.saturating_sub(2);
    if inner_w > w + 1 {
        let rect = Rect { x: area.x + 1 + inner_w - w, y: area.y + 1, width: w, height: 1 };
        frame.render_widget(Paragraph::new(Line::from(Span::styled(label, style))), rect);

        // "Refreshing…" rides immediately to the left of Notifications, so the fetch state
        // sits with the rest of the top-right chrome. The Notifications label carries its
        // own leading space, which is the gap between the two.
        if let Some(text) = refreshing_in_tab_row(app, area.width) {
            let rw = text.chars().count() as u16;
            let rect = Rect { x: area.x + 1 + inner_w - w - rw, y: area.y + 1, width: rw, height: 1 };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(text, Style::default().fg(theme.dim).bg(theme.bg)))),
                rect,
            );
        }
    }
}

/// The tab strip's labels, in render order. Shared with the width arithmetic that decides
/// whether the right-hand chrome fits beside them.
fn tab_titles(app: &App) -> Vec<String> {
    // Launchpad is tab 0; its badge is the count of items that actually need you.
    // "Command Center" is the user-facing name for the launchpad (code keeps `launchpad`/`lp`).
    let lp_count = app.lp.iter().filter(|e| !e.bucket.muted()).count();
    let mut titles = vec![format!(" Command Center ({lp_count}) ")];
    titles.extend(app.visible_indices().iter().map(|&i| {
        let count = match i {
            0 => app.prs.len(),
            1 => app.wis.len(),
            _ => app.pipes.len(),
        };
        format!(" {} ({count}) ", TABS[i])
    }));
    titles
}

fn notifications_label(app: &App) -> String {
    format!(" Notifications ({}) [i] ", app.unread_count())
}

/// The animated "Refreshing…" text, or `None` when no refresh is in flight.
fn refreshing_label(app: &App) -> Option<String> {
    if !app.reloading {
        return None;
    }
    // Dots appear one at a time; padded to a constant width so nothing jitters.
    let n = (app.anim / 2) % 4;
    // While a refresh runs over cache-seeded rows, say how old they are. Without this the
    // list looks live when it is in fact the last run's data, and a user acts on a merged
    // PR or a finished pipeline believing it current. `data_age` is cleared the moment
    // live data lands, so this disappears on its own.
    // `rel_age` says "now" under a minute, which reads wrong in a "… old" sentence, so the
    // sub-minute case gets its own wording rather than "showing now old".
    let age = match app.data_age.map(|ts| rel_age(Some(ts))) {
        Some(age) if age == "now" => " · showing cached".to_string(),
        Some(age) => format!(" · showing {age} old"),
        None => String::new(),
    };
    Some(format!("Refreshing{}{}", ".".repeat(n), " ".repeat(3 - n)) + &age)
}

/// "Refreshing…" belongs beside Notifications in the tab row — but only when the row has
/// room for it. In a narrow terminal it would overprint the tabs, so there it falls back
/// to the footer instead; `render_footer` asks the same question.
fn refreshing_in_tab_row(app: &App, width: u16) -> Option<String> {
    let text = refreshing_label(app)?;
    let titles = tab_titles(app);
    let tabs_w: usize = titles.iter().map(|t| t.chars().count()).sum::<usize>()
        + 2 * titles.len().saturating_sub(1); // the "  " divider between tabs
    let needed = tabs_w + notifications_label(app).chars().count() + text.chars().count();
    (usize::from(width.saturating_sub(2)) >= needed).then_some(text)
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
    // Recorded before anything decides on the split, so the key handler and the anim tick
    // agree with this frame about whether the preview fits.
    app.content_w = area.width;
    let focused = app.preview_focus
        && area.width >= crate::app::PREVIEW_MIN_WIDTH
        && matches!(app.screen, Screen::PrView(_) | Screen::WiView(_) | Screen::Pipeline(_));
    // Unfocused, the split waits for a built preview (the anim tick builds it within a frame):
    // an empty list, or one whose row can't be previewed, keeps the full width.
    if focused || (matches!(app.screen, Screen::List) && app.preview_shown() && app.preview.is_some()) {
        render_split(frame, area, app, focused);
        return;
    }
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
        Screen::Inbox => {
            render_inbox(frame, area, app);
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

/// The section list beside its preview. Unfocused, the right half is the preview built for the
/// selected row; focused, it is the live view that is the screen, with every key it has. Which
/// half has focus has to read at a glance: it gets heavy borders, the other goes dim.
fn render_split(frame: &mut Frame, area: Rect, app: &mut App, focused: bool) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);
    render_table(frame, cols[0], app);
    if focused {
        app.detail_scroll_max = render_item_view(frame, cols[1], &app.theme, &app.screen, app.anim);
    } else if let Some(p) = app.preview.as_ref() {
        HITS_MUTED.with(|m| m.set(true));
        render_item_view(frame, cols[1], &app.theme, &p.view, app.anim);
        HITS_MUTED.with(|m| m.set(false));
    }
    let (active, idle) = if focused { (cols[1], cols[0]) } else { (cols[0], cols[1]) };
    mark_focus(frame, active, &app.theme);
    mark_idle(frame, idle, &app.theme);
}

/// Draws a PR, work-item or pipeline view into `area`, returning how far it can scroll (the
/// key handler clamps against it). Anything else draws nothing.
fn render_item_view(frame: &mut Frame, area: Rect, theme: &Theme, view: &Screen, anim: usize) -> u16 {
    match view {
        Screen::PrView(v) => render_pr_view(frame, area, theme, v),
        Screen::WiView(v) => render_wi_view(frame, area, theme, v),
        Screen::Pipeline(v) => {
            render_pipeline(frame, area, theme, v, anim);
            0
        }
        _ => 0,
    }
}

/// The accent-coloured frame glyphs and their heavy counterparts.
const FRAME_LIGHT: [&str; 6] = ["╭", "╮", "╰", "╯", "─", "│"];
const FRAME_HEAVY: [&str; 6] = ["┏", "┓", "┗", "┛", "━", "┃"];

/// The focused half: its accent frames are redrawn heavy, and its title on the top edge becomes
/// a solid accent chip, like the active tab. Only accent-coloured frame glyphs change, so inline
/// comment boxes and other deliberately coloured frames keep their shape.
fn mark_focus(frame: &mut Frame, area: Rect, theme: &Theme) {
    let buf = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            if cell.fg != theme.accent {
                continue;
            }
            if let Some(i) = FRAME_LIGHT.iter().position(|g| *g == cell.symbol()) {
                cell.set_symbol(FRAME_HEAVY[i]);
            } else if y == area.top() && cell.modifier.contains(Modifier::BOLD) {
                cell.set_fg(theme.bg).set_bg(theme.accent);
            }
        }
    }
}

/// The half without focus, on a background a step away from the focused one: its chrome — frames, bold accent titles and headings, the active
/// sub-tab chip — drops to the dim colour. Body text keeps its colour (author names included:
/// most themes draw them in the accent hue, but never bold), so the preview still reads; it
/// just stops competing with the side that takes the keys.
fn mark_idle(frame: &mut Frame, area: Rect, theme: &Theme) {
    let buf = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            let chrome = FRAME_LIGHT.contains(&cell.symbol()) || cell.modifier.contains(Modifier::BOLD);
            if cell.fg == theme.accent && chrome {
                cell.set_fg(theme.dim);
            }
            if cell.bg == theme.accent {
                cell.set_bg(theme.dim);
            } else if cell.bg == theme.bg {
                cell.set_bg(theme.idle_bg);
            }
        }
    }
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
        empty(frame, area, theme, msg, section_block(theme, "Command Center · what needs you"));
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

    // Recorded before the empty case returns, so a click still moves focus to an empty column.
    hit(area, Hit::LpColumn(side));
    let col = app.lp_column(side);
    let slots = app.lp_slots(side);
    if slots.is_empty() {
        empty(frame, area, theme, "— nothing here —", block);
        return;
    }

    let content_w = (area.width.saturating_sub(2) as usize).saturating_sub(2); // borders + highlight symbol
    // One shared set of column widths for the whole side (real entries only), so rows line up.
    let entry_cells: Vec<Vec<Vec<Span>>> = slots
        .iter()
        .filter_map(|s| match s {
            LpSlot::Entry(i) => Some(lp_cells(theme, &app.lp[*i], app.anim, side == 0, app.review_sla_hours)),
            LpSlot::More(_) => None,
        })
        .collect();
    let widths = lp_widths(&entry_cells, 3, content_w);

    let mut items: Vec<ListItem> = Vec::new();
    // The slot each list item shows, for clicks; `None` for headings and spacers.
    let mut item_slot: Vec<Option<usize>> = Vec::new();
    let mut visual_sel = 0usize;
    let mut last: Option<Bucket> = None;
    let mut cell_i = 0usize;
    for (pos, slot) in slots.iter().enumerate() {
        let selected = pos == app.lp_sel[side];
        match *slot {
            LpSlot::Entry(i) => {
                let e = &app.lp[i];
                if last != Some(e.bucket) {
                    let count = col.iter().filter(|&&j| app.lp[j].bucket == e.bucket).count();
                    let rank = Bucket::ORDER.iter().position(|b| *b == e.bucket).unwrap_or(0);
                    // All headings share one calm grey so they read uniformly as section labels.
                    let style = Style::default().fg(theme.dim).add_modifier(Modifier::BOLD);
                    if !items.is_empty() {
                        items.push(ListItem::new(Line::from("")));
                        item_slot.push(None);
                    }
                    items.push(ListItem::new(Line::from(Span::styled(
                        format!("{}  {} ({count})", CIRCLED[rank.min(7)], e.bucket.title()),
                        style,
                    ))));
                    item_slot.push(None);
                    last = Some(e.bucket);
                }
                if selected {
                    visual_sel = items.len();
                }
                let cells = &entry_cells[cell_i];
                cell_i += 1;
                // On the focused row, any overflowing column (where, title, person) scrolls so
                // it's readable.
                let line = if selected && focused {
                    let mut c = cells.clone();
                    for col in [LP_WHERE_COL, LP_TITLE_COL, LP_PERSON_COL] {
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
                item_slot.push(Some(pos));
            }
            LpSlot::More(_) => {
                if selected {
                    visual_sel = items.len();
                }
                let style = Style::default().fg(theme.accent).add_modifier(Modifier::BOLD);
                items.push(ListItem::new(Line::from(Span::styled("more…".to_string(), style))));
                item_slot.push(Some(pos));
            }
        }
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(if focused { highlight(theme) } else { Style::default() })
        .highlight_symbol(if focused { "▐ " } else { "  " });
    let mut state = ListState::default();
    state.select(Some(visual_sel));
    frame.render_stateful_widget(list, area, &mut state);
    let body = area.inner(ratatui::layout::Margin::new(1, 1));
    hit_rows(body, state.offset(), item_slot.into_iter().map(|s| s.map(|pos| Hit::LpRow { side, pos })));
}

/// Number of Launchpad row columns (kept in sync with [`lp_cells`]).
const LP_NCOL: usize = 7;
/// The flexible title column in a Launchpad row (the one that marquee-scrolls).
const LP_TITLE_COL: usize = 3;
/// The "where" column (repo + PR number / workflow / work-item key); capped, and scrolls when
/// selected, so a long `repo · workflow` can't crowd out the title.
const LP_WHERE_COL: usize = 2;
/// Max width for the "where" column before it truncates (and marquees on the focused row).
const LP_WHERE_MAX: usize = 24;
/// The person column (author / who-ran / assignee); capped, and scrolls when selected.
const LP_PERSON_COL: usize = 5;
/// Max width for the person column before it truncates (and marquees on the focused row).
const LP_PERSON_MAX: usize = 16;
/// Gap between Launchpad columns — tighter than the nav lists since a column is half-width.
const LP_GAP: usize = 2;
/// Blank columns kept at the end of a row. The title column is elastic and would otherwise eat
/// every spare column, leaving the age hard up against the pane border.
const LP_TRAIL: usize = 3;

/// Joins a repository with the qualifier that identifies an item inside it — a PR number, a
/// workflow name, a work-item key. Either half may be missing (Jira and Linear aren't
/// repo-addressed; some runs have no definition), and a qualifier that merely repeats the row's
/// title is dropped rather than shown twice.
fn lp_where(repo: Option<&str>, qualifier: Option<&str>, sep: &str, title: &str) -> String {
    let repo = short_repo(repo);
    match qualifier.filter(|q| !q.is_empty() && *q != title) {
        Some(q) if repo.is_empty() => q.to_string(),
        Some(q) => format!("{repo}{sep}{q}"),
        None => repo,
    }
}

/// A pipeline run's outcome, lowercased to sit alongside the PR statuses ("open", "merged").
fn pipe_status_label(status: PipelineRunStatus) -> &'static str {
    match status {
        PipelineRunStatus::Queued => "queued",
        PipelineRunStatus::Running => "running",
        PipelineRunStatus::Succeeded => "passed",
        PipelineRunStatus::PartiallySucceeded => "partial",
        PipelineRunStatus::Failed => "failed",
        PipelineRunStatus::Canceled => "canceled",
    }
}

/// The aligned cells for one Launchpad row. Every item type fills the *same* seven
/// slots — type · status · where · title · signal · person · age — so rows read as
/// siblings and line up vertically, even though PRs, pipelines and work items differ.
///
/// The three middle slots are what make a row legible, and each is type-appropriate:
/// **where** locates the item (`forgetop #161`, `forgetop · CI`), **title** says what it is,
/// and **signal** answers *why it is in front of you* — the blocker or check roll-up for a PR,
/// the branch for a run. The "person" is the PR author / who ran the pipeline / the work-item
/// assignee; `show_person` is false in the right-hand column, where every row is yours anyway
/// and the name would just repeat down the pane (the empty cell collapses to zero width).
///
/// A review request's age is how long it has waited (`created_at`, not the last activity a bot
/// push or a comment would reset), coloured against `sla_hours` so stale asks stand out.
fn lp_cells(theme: &Theme, e: &crate::launchpad::Entry, anim: usize, show_person: bool, sla_hours: u32) -> Vec<Vec<Span<'static>>> {
    use crate::launchpad::EntryItem;
    let dim = Style::default().fg(theme.dim);
    let fg = Style::default().fg(theme.fg);
    // Kept calm: type badge, where, person and age are all grey (bar a late review request);
    // only the status, the blocker
    // and the git-diff +/- carry colour.
    let cell = |s: String, st: Style| vec![Span::styled(s, st)];
    let person = |u: Option<&forgetop_core::domain::User>| {
        if show_person {
            cell(u.map(|u| u.display_name.clone()).unwrap_or_else(|| "—".into()), dim)
        } else {
            Vec::new()
        }
    };
    let age = |t| cell(rel_age(t), dim);
    let diffstat = |add, del| {
        vec![
            Span::styled(format!("+{add}"), Style::default().fg(theme.green)),
            Span::raw(" "),
            Span::styled(format!("-{del}"), Style::default().fg(theme.red)),
        ]
    };
    match &e.item {
        EntryItem::Pr(pr) => {
            // Status is the PR's lifecycle state (Open / Draft / Merged / Closed).
            let (st, stc) = pr_status(theme, pr);
            let number = pr.number.map(|n| format!("#{n}"));
            // Why this row is here. A conflict or a changes-requested review is named in words —
            // neither is visible anywhere else on the row — while failing checks are already
            // legible as the red roll-up, so that case keeps the roll-up and the diffstat.
            let signal = match pr.status {
                // Nothing is blocking a finished PR; its size is the only thing still worth saying.
                PullRequestStatus::Merged | PullRequestStatus::Closed => diffstat(pr.additions, pr.deletions),
                _ => {
                    let (sig, sigc) = pr_signal(theme, pr);
                    let mut s = cell(sig, Style::default().fg(sigc));
                    // A worded blocker fills the cell on its own; a check roll-up is short enough
                    // to leave room for the size beside it.
                    let worded = matches!(pr_state(pr), PrState::Blocked(PrBlocker::Conflicting | PrBlocker::ChangesRequested));
                    if !worded {
                        s.push(Span::raw("  "));
                        s.extend(diffstat(pr.additions, pr.deletions));
                    }
                    s
                }
            };
            let age_cell = if e.bucket == crate::launchpad::Bucket::NeedsReview {
                let waited = pr.created_at.or(pr.updated_at);
                cell(rel_age(waited), Style::default().fg(sla_color(theme, waited, sla_hours, Utc::now())))
            } else {
                age(pr.updated_at)
            };
            vec![
                cell("PR".into(), dim),
                cell(st.to_string(), Style::default().fg(stc)),
                cell(lp_where(pr.repository.as_deref(), number.as_deref(), " ", &pr.title), dim),
                cell(pr.title.clone(), fg),
                signal,
                person(Some(&pr.author)),
                age_cell,
            ]
        }
        EntryItem::Pipe { run, definition_name } => {
            // The title is what the run was *building* (commit subject / triggering PR), so the
            // workflow it ran under belongs beside the repository instead.
            let title = forgetop_core::launchpad::pipe_title(run, definition_name.as_deref());
            let workflow = forgetop_core::launchpad::pipe_workflow(run, definition_name.as_deref());
            let branch = run.branch.clone().map(|b| format!("⑂ {b}")).unwrap_or_default();
            vec![
                cell("CI".into(), dim),
                cell(
                    format!("{} {}", pipeline_glyph(run.status, anim), pipe_status_label(run.status)),
                    Style::default().fg(theme.pipeline_color(run.status)),
                ),
                cell(lp_where(run.repository.as_deref(), Some(workflow), " · ", title), dim),
                cell(title.to_string(), fg),
                cell(branch, dim),
                person(run.triggered_by.as_ref()),
                age(run.finished_at.or(run.started_at)),
            ]
        }
        EntryItem::Wi(wi) => vec![
            cell("WI".into(), dim),
            cell(format!("● {}", wi.state), Style::default().fg(wi_state_color(theme, &wi.state, wi.state_category))),
            cell(lp_where(wi.repository.as_deref(), wi.identifier.as_deref(), " ", &wi.title), dim),
            cell(wi.title.clone(), fg),
            cell(wi.work_item_type.clone().unwrap_or_default(), dim),
            person(wi.assignee.as_ref()),
            age(wi.updated_at),
        ],
    }
}

/// The colour of a review request's age: grey inside `sla_hours`, yellow past it, red past three
/// times it. An unknown timestamp stays grey — there is nothing to be late against.
fn sla_color(theme: &Theme, since: Option<DateTime<Utc>>, sla_hours: u32, now: DateTime<Utc>) -> ratatui::style::Color {
    let Some(since) = since else { return theme.dim };
    let waited = (now - since).num_minutes().max(0);
    let sla = i64::from(sla_hours.max(1)) * 60;
    if waited >= sla * 3 {
        theme.red
    } else if waited >= sla {
        theme.yellow
    } else {
        theme.dim
    }
}

/// Total display width of a Launchpad cell (its spans).
fn cell_width(cell: &[Span]) -> usize {
    cell.iter().map(|s| s.content.chars().count()).sum()
}

/// Sizes each Launchpad column to its widest cell, clamping the flexible title column so the row
/// fits `inner_w` with [`LP_TRAIL`] columns to spare.
fn lp_widths(rows: &[Vec<Vec<Span>>], flex: usize, inner_w: usize) -> Vec<usize> {
    let mut w = vec![0usize; LP_NCOL];
    for row in rows {
        for (i, cell) in row.iter().enumerate().take(LP_NCOL) {
            w[i] = w[i].max(cell_width(cell));
        }
    }
    // Cap the person and "where" columns so a long name or `repo · workflow` doesn't crowd out
    // the title (they scroll instead).
    w[LP_PERSON_COL] = w[LP_PERSON_COL].min(LP_PERSON_MAX);
    w[LP_WHERE_COL] = w[LP_WHERE_COL].min(LP_WHERE_MAX);
    let padding = COL_LEAD + LP_GAP * (LP_NCOL - 1) + LP_TRAIL;
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

/// The selected row of a list that has handed focus to its preview: a dim `▌` in the lead
/// column, no bar.
fn mark_selected_idle(line: &mut Line, theme: &Theme) {
    if let Some(lead) = line.spans.first_mut() {
        *lead = Span::styled("▌", Style::default().fg(theme.dim));
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
                // While the preview beside it has focus, the list keeps only a quiet marker: the
                // bright bar belongs in the pane that takes the keys.
                if app.preview_focus {
                    mark_selected_idle(&mut row, &app.theme);
                } else {
                    mark_selected(&mut row, &app.theme);
                }
            }
            row
        })
        .collect();
    let count = lines.len();
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), parts[1]);
    // While the preview has the keys the list only shows where you are; it takes no clicks.
    if !app.preview_focus {
        hit_rows(parts[1], scroll as usize, (0..count).map(|i| Some(Hit::ListRow(i))));
    }
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

/// The Title column of the Pull Requests list — the one that flexes.
const PR_TITLE_COL: usize = 4;

/// A title narrower than this reads as noise, so columns are shed to keep at least this much.
const MIN_TITLE_W: usize = 28;

/// Drops columns, in `drop_order`, until the `flex` column can show [`MIN_TITLE_W`] characters
/// (or all of its widest value, if shorter). A wide list is returned untouched; a narrow one —
/// beside the preview pane — keeps its identifying columns and loses the ones the preview
/// repeats. The sort arrow follows its column, and disappears with it if that one is shed.
#[allow(clippy::type_complexity)]
fn shed_columns(
    headers: &[&'static str],
    mut cells: Vec<Vec<(String, Style)>>,
    flex: usize,
    sort: Option<(usize, bool)>,
    inner_w: usize,
    drop_order: &[usize],
) -> (Vec<&'static str>, Vec<Vec<(String, Style)>>, Option<(usize, bool)>) {
    let natural = |i: usize| -> usize {
        let body = cells.iter().filter_map(|r| r.get(i)).map(|(t, _)| t.chars().count()).max().unwrap_or(0);
        body.max(headers[i].chars().count() + 2) // room for a sort arrow
    };
    let widths: Vec<usize> = (0..headers.len()).map(natural).collect();
    let want = widths[flex].min(MIN_TITLE_W);
    let mut keep: Vec<bool> = vec![true; headers.len()];
    let room = |keep: &[bool]| -> usize {
        let kept: Vec<usize> = (0..headers.len()).filter(|&i| keep[i]).collect();
        let fixed: usize = kept.iter().filter(|&&i| i != flex).map(|&i| widths[i]).sum();
        let padding = COL_LEAD + COL_GAP * kept.len().saturating_sub(1);
        inner_w.saturating_sub(fixed + padding)
    };
    for &col in drop_order {
        if room(&keep) >= want {
            break;
        }
        keep[col] = false;
    }
    if keep.iter().all(|k| *k) {
        return (headers.to_vec(), cells, sort);
    }
    let new_index = |old: usize| -> Option<usize> { keep[old].then(|| keep[..old].iter().filter(|k| **k).count()) };
    let headers = headers.iter().enumerate().filter(|(i, _)| keep[*i]).map(|(_, h)| *h).collect();
    for row in &mut cells {
        let mut i = 0;
        row.retain(|_| {
            i += 1;
            keep[i - 1]
        });
    }
    let sort = sort.and_then(|(col, desc)| new_index(col).map(|c| (c, desc)));
    (headers, cells, sort)
}

/// Maps a section's active sort to the header column index that gets the arrow.
fn sort_header_col(section: usize, key: &str) -> Option<usize> {
    match section {
        // Pull Requests: "", Provider, Repository, #, Title, Author, State, ±, Updated.
        0 => match key {
            "status" => Some(0),
            "number" => Some(3),
            "title" => Some(4),
            "author" => Some(5),
            "checks" => Some(6),
            "updated" => Some(8),
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

/// Shown when a section's connections have explicitly chosen no repositories. Deliberately not
/// "nothing to show": nothing was fetched because nothing was asked for, and the fix is one key.
const NO_REPOS_HINT: &str = "No repositories selected. Press g to choose which ones to fetch from.";

/// Appends "· Repos · 5 of 37" to a section title, once discovery has a real count to show.
fn with_scope(base: String, app: &App, section: usize) -> String {
    match &app.repo_scope[section] {
        Some(s) => format!("{base} · {}", s.label()),
        None => base,
    }
}

fn scope_is_empty(app: &App, section: usize) -> bool {
    app.repo_scope[section].as_ref().is_some_and(|s| s.none_selected)
}

/// The repository column: just the trailing name, since the owner/workspace repeats on every row.
fn short_repo(repo: Option<&str>) -> String {
    repo.map(|r| r.rsplit('/').next().unwrap_or(r).to_string()).unwrap_or_default()
}

fn pr_status(theme: &Theme, pr: &PullRequest) -> (&'static str, ratatui::style::Color) {
    if pr.is_draft {
        return ("◌ draft", theme.dim);
    }
    match pr.status {
        // Green = open (healthy), magenta = merged (shipped/done), red = closed unmerged.
        PullRequestStatus::Open => ("● open", theme.green),
        PullRequestStatus::Merged => ("✦ merged", theme.magenta),
        PullRequestStatus::Closed => ("✗ closed", theme.red),
        PullRequestStatus::Draft => ("◌ draft", theme.dim),
    }
}

/// A `2/8`-style check roll-up behind `glyph`, or `glyph —` when the provider gave no per-check
/// summary. The glyph is passed in because the *state* decides it, not `pr.checks`: a run that is
/// half finished rounds to Passed or Failed there while the summary still shows work in flight.
fn check_rollup(glyph: &str, pr: &PullRequest) -> String {
    match &pr.check_summary {
        Some(s) => format!("{glyph} {}/{}", s.successful, s.total()),
        None => format!("{glyph} —"),
    }
}

fn pr_checks(theme: &Theme, pr: &PullRequest) -> (String, ratatui::style::Color) {
    (check_rollup(check_icon(pr.checks), pr), theme.check_color(pr.checks))
}

/// The compact "what is stopping this" cell: the blocker when there is one, otherwise the check
/// roll-up. Shared by the PR list's State column and the Command Center rows so the same pull
/// request never reads as healthy in one place and blocked in another.
fn pr_signal(theme: &Theme, pr: &PullRequest) -> (String, ratatui::style::Color) {
    match pr_state(pr) {
        PrState::Blocked(PrBlocker::Conflicting) => ("⚠ conflicts".into(), theme.yellow),
        PrState::Blocked(PrBlocker::ChangesRequested) => ("⚠ changes".into(), theme.yellow),
        PrState::Blocked(PrBlocker::ChecksFailing) => (check_rollup("✗", pr), theme.red),
        PrState::ChecksRunning => (check_rollup("◐", pr), theme.blue),
        _ => pr_checks(theme, pr),
    }
}

/// How a pull request's merge state reads: a glyph and a word, coloured, rather than the raw
/// enum name — only one of the four is good news, and a flat string doesn't say which.
fn mergeable_meta(theme: &Theme, state: MergeableState) -> (&'static str, ratatui::style::Color) {
    match state {
        MergeableState::Mergeable => ("✓ clean", theme.green),
        MergeableState::Conflicting => ("✗ conflicts", theme.red),
        MergeableState::Blocked => ("⚠ blocked", theme.yellow),
        // Not bad news — the forge just hasn't computed the merge yet. Colouring it would cry wolf.
        MergeableState::Unknown => ("· unknown", theme.dim),
    }
}

/// "5 of 8 checks passed", or `None` when the pull request has no CI to speak of.
fn checks_clause(pr: &PullRequest) -> Option<String> {
    match &pr.check_summary {
        Some(s) if s.total() > 0 => Some(format!("{} of {} checks passed", s.successful, s.total())),
        // No per-check numbers: fall back to the roll-up's word, and say nothing at all when
        // there are no checks configured.
        _ => match pr.checks {
            CheckStatus::None => None,
            CheckStatus::Passed => Some("checks passed".into()),
            CheckStatus::Failed => Some("checks failing".into()),
            CheckStatus::Pending => Some("checks running".into()),
        },
    }
}

/// The one-line verdict shown under the PR header: where this pull request stands, and why.
/// Rendered from [`pr_state`], so it agrees with the list's State column and the Command Center.
fn pr_state_line(theme: &Theme, pr: &PullRequest) -> Line<'static> {
    let target = || pr.target_ref.clone().unwrap_or_else(|| "the target branch".into());
    let who = |u: Option<&forgetop_core::domain::User>| u.map(|u| format!(" by {}", u.display_name)).unwrap_or_default();
    let (glyph, text, color) = match pr_state(pr) {
        PrState::Merged => {
            let age = rel_age(pr.updated_at);
            let when = if age == "—" { String::new() } else { format!(" {age} ago") };
            ("✦", format!("Merged into {}{when}", target()), theme.magenta)
        }
        PrState::Closed => ("✗", "Closed without merging".into(), theme.red),
        PrState::Draft => ("◌", "Draft — not open for review yet".into(), theme.dim),
        PrState::Blocked(PrBlocker::Conflicting) => ("⚠", format!("Blocked — conflicts with {}", target()), theme.yellow),
        PrState::Blocked(PrBlocker::ChangesRequested) => (
            "⚠",
            format!("Blocked — changes requested{}", who(pr_changes_requested_by(pr))),
            theme.yellow,
        ),
        PrState::Blocked(PrBlocker::ChecksFailing) => {
            // The count of failures, never total minus passed: with checks still in flight that
            // subtraction reports work-in-progress as failed.
            let detail = match &pr.check_summary {
                Some(s) if s.failed > 0 => format!("{} of {} checks failed", s.failed, s.total()),
                _ => "checks failing".into(),
            };
            ("✗", format!("Blocked — {detail}"), theme.red)
        }
        PrState::ChecksRunning => {
            let detail = match &pr.check_summary {
                Some(s) => format!("{} of {} done", s.total() - s.in_progress, s.total()),
                None => "still running".into(),
            };
            ("◐", format!("Checks running — {detail}"), theme.blue)
        }
        PrState::ReadyToMerge => {
            let mut parts: Vec<String> = checks_clause(pr).into_iter().collect();
            parts.push(format!("approved{}", who(pr_approved_by(pr))));
            ("✓", format!("Ready to merge — {}", parts.join(", ")), theme.green)
        }
        PrState::NothingBlocking => {
            let mut parts: Vec<String> = checks_clause(pr).into_iter().collect();
            if pr.reviewers.is_empty() {
                parts.push("no reviews yet".into());
            }
            let detail = if parts.is_empty() { String::new() } else { format!(" — {}", parts.join(", ")) };
            ("✓", format!("Nothing blocking{detail}"), theme.green)
        }
    };
    let style = Style::default().fg(color).add_modifier(Modifier::BOLD);
    Line::from(vec![Span::raw("  "), Span::styled(glyph.to_string(), style), Span::styled(format!("  {text}"), style)])
}

fn render_prs(frame: &mut Frame, area: Rect, app: &mut App) {
    let theme = &app.theme;
    let idxs = app.filtered_pr_indices();
    let base = with_scope(format!("Pull Requests · {}", crate::app::pr_status_summary(&app.pr_shown_statuses)), app, 0);
    let title = list_title(base, &app.filters[0]);
    if idxs.is_empty() {
        let msg = if !app.filters[0].is_empty() {
            "No matches. Esc clears the filter.".to_string()
        } else if scope_is_empty(app, 0) {
            NO_REPOS_HINT.to_string()
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
    // With one connection spanning an account, the provider/connection column no longer tells
    // rows apart — the repository is what does.
    // "State", not "Checks": a conflict or a changes-requested review stops a pull request just
    // as dead as red CI, and a list that shows only checks reports both of those as healthy.
    let headers = ["", "Provider", "Repository", "#", "Title", "Author", "State", "±", "Updated"];
    let cells: Vec<Vec<(String, Style)>> = idxs
        .iter()
        .map(|&i| &app.prs[i])
        .map(|row| {
            let pr = &row.pr;
            let (st, stc) = pr_status(theme, pr);
            let (ck, ckc) = pr_signal(theme, pr);
            vec![
                (st.to_string(), Style::default().fg(stc)),
                (provider_tag(row.provider, &row.connection), Style::default().fg(theme.cyan)),
                (short_repo(pr.repository.as_deref()), Style::default().fg(theme.dim)),
                (pr.number.map(|n| format!("#{n}")).unwrap_or_default(), Style::default().fg(theme.dim)),
                (pr.title.clone(), Style::default().fg(theme.fg)),
                (pr.author.display_name.clone(), Style::default().fg(theme.blue)),
                (ck, Style::default().fg(ckc)),
                (format!("+{} -{}", pr.additions, pr.deletions), Style::default().fg(theme.dim)),
                (rel_age(pr.updated_at), Style::default().fg(theme.dim)),
            ]
        })
        .collect();

    // Beside the preview the list is narrow: shed the columns the preview repeats, least
    // useful first — Provider, ±, Author, Updated — before the title gets squeezed.
    let (headers, cells, sort) = shed_columns(&headers, cells, PR_TITLE_COL, sort_marker(app, 0), inner_w, &[1, 7, 5, 8]);
    let flex = headers.iter().position(|h| *h == "Title").unwrap_or(0);
    let (header, rows) = columnize(dim, &headers, &cells, flex, inner_w, sort);
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
    let title = list_title(with_scope(base, app, 1), &app.filters[1]);
    if idxs.is_empty() {
        let msg = if !app.filters[1].is_empty() {
            "No matches. Esc clears the filter.".to_string()
        } else if scope_is_empty(app, 1) {
            NO_REPOS_HINT.to_string()
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

    // Provider, Type, Assignee, Updated go first when the list sits beside the preview.
    let (headers, cells, sort) = shed_columns(&headers, cells, 3, sort_marker(app, 1), inner_w, &[1, 4, 5, 6]);
    let flex = headers.iter().position(|h| *h == "Title").unwrap_or(0);
    let (header, rows) = columnize(dim, &headers, &cells, flex, inner_w, sort);
    render_inline_list(frame, area, app, &title, header, rows);
}

// ---- Pipelines ----

/// One column of the Pipelines table.
///
/// The set is not fixed: a column is shown when the lines on screen can actually fill it.
/// That single rule covers everything below — a Provider column saying "GitHub" on every row
/// is nine characters of nothing, and a Branch column is meaningless above a group that spans
/// four of them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PipeCol {
    /// Expand arrow on a header, tree glyph on a child. Nothing when ungrouped.
    Tree,
    Status,
    Provider,
    Repository,
    /// The group's name on a header; on a run, whatever varies inside its group.
    Subject,
    /// "5 runs · 3 failed" on a header, the run's own number on a run.
    Runs,
    /// Only when ungrouped. Every grouped mode carries the branch in Subject instead —
    /// on the header when it is the key, on the child when it is what varies.
    Branch,
    Commit,
    Started,
    Approval,
}

impl PipeCol {
    fn heading(self, app: &App, any_open: bool) -> String {
        match self {
            PipeCol::Tree | PipeCol::Status => String::new(),
            PipeCol::Provider => "Provider".into(),
            PipeCol::Repository => "Repository".into(),
            PipeCol::Subject => app.pipe_subject_heading(any_open).into(),
            // Ungrouped, the cell is one run's number, not a count of them.
            PipeCol::Runs => if app.pipe_group == PipeGroup::Off { "Run" } else { "Runs" }.into(),
            PipeCol::Branch => "Branch".into(),
            PipeCol::Commit => "Commit".into(),
            PipeCol::Started => "Started".into(),
            PipeCol::Approval => "Approval".into(),
        }
    }
}

/// Picks the columns for what is currently on screen.
fn pipe_columns(app: &App, lines: &[PipeLine], multi_provider: bool) -> Vec<PipeCol> {
    let grouped = app.pipe_group != PipeGroup::Off;
    let approvals = lines.iter().any(|l| match l {
        PipeLine::Head(h) => h.approval,
        PipeLine::Run(i) => app.pipes[*i].awaiting_approval,
    });

    let mut cols = Vec::new();
    if grouped {
        cols.push(PipeCol::Tree);
    }
    cols.push(PipeCol::Status);
    if multi_provider {
        cols.push(PipeCol::Provider);
    }
    // The repository leads: it is what a run is read against, and trailing it behind the
    // timings left the one fact that says *where* this is happening at the far edge.
    cols.push(PipeCol::Repository);
    cols.push(PipeCol::Subject);
    cols.push(PipeCol::Runs);
    // Ungrouped there is no header to carry the branch and no child to put it in Subject,
    // so it needs its own column — this mode must stay the list the tab had before grouping.
    if !grouped {
        cols.push(PipeCol::Branch);
    }
    if app.pipe_group == PipeGroup::Trigger {
        cols.push(PipeCol::Commit);
    }
    cols.push(PipeCol::Started);
    if approvals {
        cols.push(PipeCol::Approval);
    }
    cols
}

/// Which live column the active sort marks.
///
/// The Pipelines columns are chosen per render, so a fixed key-to-index table (the way the
/// other two sections do it) points at whatever column happens to sit at that index — the
/// arrow ends up over Repository while the list is sorted by start time. Resolving through
/// the column itself also means a sort on a column that is not currently shown draws no
/// arrow at all, rather than a wrong one.
fn pipe_sort_marker(app: &App, cols: &[PipeCol]) -> Option<(usize, bool)> {
    let sort = app.sort_for(2)?;
    let col = match sort.key.as_str() {
        "status" => PipeCol::Status,
        "provider" => PipeCol::Provider,
        "repository" => PipeCol::Repository,
        "started" => PipeCol::Started,
        // Grouped, both of these are rendered in Subject — the header carries one and the
        // child the other. Ungrouped they are separate columns again.
        "pipeline" => PipeCol::Subject,
        "branch" => {
            if app.pipe_group == PipeGroup::Off {
                PipeCol::Branch
            } else {
                PipeCol::Subject
            }
        }
        _ => return None,
    };
    Some((cols.iter().position(|c| *c == col)?, sort.desc))
}

/// Drops the least important columns until the table fits the pane.
///
/// `columnize` clamps only its flexible column and never drops one, so an unusually wide set
/// — two providers, a commit and a waiting gate all at once — pushes the rightmost columns
/// off the edge. Approval is the one column that is only ever present *because* something
/// needs attention, so it must not be the thing that falls off; Repository, Commit and
/// Provider are context and give way first.
fn fit_columns(cols: &mut Vec<PipeCol>, cells: &mut [Vec<(String, Style)>], headings: &[String], inner_w: usize) {
    let natural = |cols: &[PipeCol], cells: &[Vec<(String, Style)>]| -> usize {
        let mut w: Vec<usize> = headings.iter().map(|h| h.chars().count()).collect();
        for row in cells.iter() {
            for (i, (text, _)) in row.iter().enumerate() {
                w[i] = w[i].max(text.chars().count());
            }
        }
        w.iter().sum::<usize>() + COL_LEAD + COL_GAP * cols.len().saturating_sub(1)
    };

    // The repository goes last: it is the column the list leads with, and beside the preview
    // pane (the default on a wide terminal) this narrow case is the everyday one.
    for droppable in [PipeCol::Provider, PipeCol::Commit, PipeCol::Repository] {
        if natural(cols, cells) <= inner_w {
            return;
        }
        if let Some(at) = cols.iter().position(|c| *c == droppable) {
            cols.remove(at);
            for row in cells.iter_mut() {
                row.remove(at);
            }
        }
    }
}

/// The owner prefix every repository on screen shares, if they all share one.
///
/// With a single owner the prefix is pure repetition down the column, so it is elided and
/// `magna-nz/forgetop` reads as `forgetop`. Two owners and it stays, because then it matters.
fn shared_owner(app: &App, idxs: &[usize]) -> Option<String> {
    let mut owner: Option<String> = None;
    for &i in idxs {
        let repo = app.pipes[i].run.repository.clone().unwrap_or_default();
        let (o, rest) = repo.split_once('/')?;
        if rest.is_empty() {
            return None;
        }
        match &owner {
            Some(prev) if prev != o => return None,
            Some(_) => {}
            None => owner = Some(o.to_string()),
        }
    }
    owner
}

fn strip_owner(repo: &str, owner: Option<&String>) -> String {
    match owner {
        Some(o) => repo.strip_prefix(&format!("{o}/")).unwrap_or(repo).to_string(),
        None => repo.to_string(),
    }
}

/// A short, capitalised outcome for the status column — `PartiallySucceeded` spelled in full
/// is eighteen characters of column for a state nobody scans for.
fn pipe_status_word(status: PipelineRunStatus) -> &'static str {
    match status {
        PipelineRunStatus::Queued => "Queued",
        PipelineRunStatus::Running => "Running",
        PipelineRunStatus::PartiallySucceeded => "Partial",
        PipelineRunStatus::Canceled => "Canceled",
        PipelineRunStatus::Succeeded => "Succeeded",
        PipelineRunStatus::Failed => "Failed",
    }
}

/// A run's outcome as a cell: the glyph and the word, on every row.
///
/// Pass and fail used to be the bare tick and cross, on the grounds that the word repeats.
/// But a column that spells out four of its six states and draws the other two leaves you
/// decoding a symbol in the one place the answer matters most — and a tick alone is no help
/// at all where colour is lost (a monochrome terminal, a pasted screenshot, a colour-blind
/// reader). The glyph stays because it is what makes the column scannable at a glance.
fn pipe_status_cell(status: PipelineRunStatus, anim: usize) -> String {
    format!("{} {}", pipeline_glyph(status, anim), pipe_status_word(status))
}

fn render_pipes(frame: &mut Frame, area: Rect, app: &mut App) {
    let theme = &app.theme;
    let idxs = app.filtered_pipe_indices();
    let mut base = with_scope("Pipelines".to_string(), app, 2);
    if app.pipe_group != PipeGroup::Off && !idxs.is_empty() {
        // The column arrow means "runs sort by this", within each group. Group order is a
        // separate fact, so it is stated — but only while it is the whole story: with an
        // explicit sort the runs inside a group no longer follow it.
        base.push_str(&format!(" · by {}", app.pipe_group.as_str()));
        if app.pipe_sort.is_none() {
            base.push_str(" (newest first)");
        }
    }
    let title = list_title(base, &app.filters[2]);
    if idxs.is_empty() {
        let msg = if !app.filters[2].is_empty() {
            "No matches. Esc clears the filter.".to_string()
        } else if scope_is_empty(app, 2) {
            NO_REPOS_HINT.to_string()
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
    let lines = app.pipe_lines();
    let any_open = lines.iter().any(|l| matches!(l, PipeLine::Head(h) if h.expanded));

    let mut providers: Vec<String> = idxs.iter().map(|&i| provider_tag(app.pipes[i].provider, &app.pipes[i].connection)).collect();
    providers.sort();
    providers.dedup();
    let cols = pipe_columns(app, &lines, providers.len() > 1);
    let owner = shared_owner(app, &idxs);

    // Headers and runs are the same shape now, so they go through `columnize` together and
    // every column is measured across both. Laying headers out separately was what let the
    // roll-up drift to the right edge, under no heading at all.
    let cells: Vec<Vec<(String, Style)>> = lines
        .iter()
        .enumerate()
        .map(|(n, line)| match line {
            PipeLine::Head(h) => cols.iter().map(|c| head_cell(*c, h, theme, app.anim, owner.as_ref())).collect(),
            PipeLine::Run(i) => {
                let p = &app.pipes[*i];
                let child = app.pipe_group != PipeGroup::Off;
                let subject = if child { format!("─ {}", app.pipe_child_subject(p)) } else { pipe_definition_name(p) };
                // The next line tells us whether this run closes its group, so `└` costs a
                // peek rather than a scan back through the list for every row.
                let last = !matches!(lines.get(n + 1), Some(PipeLine::Run(_)));
                cols.iter().map(|c| run_cell(*c, p, &subject, last, child, theme, app.anim, owner.as_ref())).collect()
            }
        })
        .collect();

    let mut cols = cols;
    let mut cells = cells;
    let headings: Vec<String> = cols.iter().map(|c| c.heading(app, any_open)).collect();
    fit_columns(&mut cols, &mut cells, &headings, inner_w);

    let headings: Vec<String> = cols.iter().map(|c| c.heading(app, any_open)).collect();
    let heading_refs: Vec<&str> = headings.iter().map(String::as_str).collect();
    let flex = cols.iter().position(|c| *c == PipeCol::Subject).unwrap_or(0);
    let (header, rows) = columnize(dim, &heading_refs, &cells, flex, inner_w, pipe_sort_marker(app, &cols));
    render_inline_list(frame, area, app, &title, header, rows);
}

fn head_cell(col: PipeCol, h: &PipeHead, theme: &Theme, anim: usize, owner: Option<&String>) -> (String, Style) {
    let dim = Style::default().fg(theme.dim);
    match col {
        PipeCol::Tree => (if h.expanded { "▾" } else { "▸" }.to_string(), Style::default().fg(theme.dim)),
        PipeCol::Status => (
            pipe_status_cell(h.status, anim),
            Style::default().fg(theme.pipeline_color(h.status)),
        ),
        PipeCol::Provider => (provider_tag(h.provider, &h.connection), Style::default().fg(theme.cyan)),
        // Plain foreground, like the Title column on Pull Requests and Work Items — accent is
        // this theme's chrome colour (borders, pane titles, the live tab), and spending it on
        // row content made the Pipelines table read as a different application. Bold is what
        // keeps a roll-up apart from the runs underneath it; the colour was never doing that.
        PipeCol::Subject => (h.subject.clone(), Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)),
        PipeCol::Runs => {
            let text = format!(
                "{} {}{}",
                h.runs,
                if h.runs == 1 { "run" } else { "runs" },
                if h.failed > 0 { format!(" · {} failed", h.failed) } else { String::new() }
            );
            let style = if h.failed > 0 { Style::default().fg(theme.red) } else { dim };
            (text, style)
        }
        // Only ever present when ungrouped, where there are no headers — kept total so the
        // cell count can never disagree with the heading count.
        PipeCol::Branch => (String::new(), dim),
        PipeCol::Commit => (h.commit.clone(), dim),
        PipeCol::Started => (rel_age(h.started), dim),
        PipeCol::Repository => (strip_owner(&h.repo, owner), dim),
        PipeCol::Approval => {
            if h.approval {
                ("approval needed".to_string(), Style::default().fg(theme.red).add_modifier(Modifier::BOLD))
            } else {
                (String::new(), dim)
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_cell(
    col: PipeCol,
    p: &PipeRow,
    subject: &str,
    last: bool,
    child: bool,
    theme: &Theme,
    anim: usize,
    owner: Option<&String>,
) -> (String, Style) {
    let dim = Style::default().fg(theme.dim);
    match col {
        // Tree is only in the set when grouped, and every run is then a child.
        PipeCol::Tree => ((if last { "└" } else { "├" }).to_string(), Style::default().fg(theme.dim)),
        // Inside a group a run is only its glyph: the header already says in words where the
        // pipeline stands, and ✓ / ✗ down the children is what the eye scans for. Ungrouped,
        // with no header above, each run keeps its word.
        PipeCol::Status => (
            if child { pipeline_glyph(p.run.status, anim).to_string() } else { pipe_status_cell(p.run.status, anim) },
            Style::default().fg(theme.pipeline_color(p.run.status)),
        ),
        PipeCol::Provider => (provider_tag(p.provider, &p.connection), Style::default().fg(theme.cyan)),
        PipeCol::Subject => (subject.to_string(), Style::default().fg(theme.fg)),
        PipeCol::Runs => {
            // Run = the run/release name ("10.1.100"), or the run number when it has no name.
            let num = || p.run.number.map(|n| format!("#{n}")).unwrap_or_default();
            let text = match &p.definition_name {
                Some(_) => p.run.name.clone().unwrap_or_else(num),
                None => num(),
            };
            (text, dim)
        }
        PipeCol::Branch => (p.run.branch.clone().unwrap_or_default(), dim),
        PipeCol::Commit => (p.run.commit_sha.as_deref().unwrap_or_default().chars().take(7).collect(), dim),
        PipeCol::Started => (rel_age(p.run.started_at), dim),
        // A child's repository is its group's, already on the header; repeating it is the
        // noise grouping exists to remove.
        PipeCol::Repository => (
            if child { String::new() } else { strip_owner(&p.run.repository.clone().unwrap_or_default(), owner) },
            dim,
        ),
        PipeCol::Approval => {
            if p.awaiting_approval {
                ("approval needed".to_string(), Style::default().fg(theme.red).add_modifier(Modifier::BOLD))
            } else {
                (String::new(), dim)
            }
        }
    }
}

// ---- full-screen PR / work-item views ----

fn field(theme: &Theme, label: &str, value: String) -> Line<'static> {
    field_styled(theme, label, value, theme.fg)
}

/// A field whose value carries meaning in its colour — the merge gates, where a flat string
/// leaves you unable to tell good news from bad.
fn field_styled(theme: &Theme, label: &str, value: String, color: ratatui::style::Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<12}"), Style::default().fg(theme.dim)),
        Span::styled(value, Style::default().fg(color)),
    ])
}

fn comment_lines(theme: &Theme, threads: &[CommentThread]) -> Vec<Line<'static>> {
    let total: usize = threads.iter().map(|t| t.comments.len()).sum();
    let mut heading = vec![Span::styled(format!("Comments ({total})"), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))];
    if total == 0 {
        heading.push(Span::styled("   No comments.", Style::default().fg(theme.dim)));
    }
    let mut lines = vec![Line::from(""), Line::from(heading)];
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

/// The widest the actor column may grow before a name is truncated, so one long name can't
/// push every event's detail off the right edge.
const ACTIVITY_ACTOR_MAX: usize = 20;

/// Splits a timeline event into a short verb and, where the event names one, the thing it moved
/// to — `state` / `→ In Progress`, `assigned` / `→ Sam Rivera` — for the Activity columns.
///
/// Every provider's mapper writes the summary the same way (`changed status to X`,
/// `assigned this to X`, `added the X label`), so the target is read back out of that sentence.
/// A summary that doesn't follow the pattern is shown whole rather than guessed at.
pub fn activity_parts(kind: TimelineEventKind, summary: &str) -> (String, Option<String>) {
    use TimelineEventKind as K;
    let after_to = |s: &str| s.split_once(" to ").map(|(_, t)| t.trim().to_string()).filter(|t| !t.is_empty());
    let verb = |v: &str| (v.to_string(), None);
    match kind {
        // An empty target (Jira reports some transitions without a name) is still a state change.
        K::StateChanged => ("state".into(), after_to(summary)),
        K::Assigned if summary.starts_with("unassigned") => verb("unassigned"),
        K::Assigned => ("assigned".into(), after_to(summary)),
        K::Labeled => {
            let label = summary.strip_prefix("added the ").and_then(|l| l.strip_suffix(" label")).map(str::to_string);
            ("labeled".into(), label)
        }
        K::Approved => verb("approved"),
        K::Reviewed => verb(summary.trim()),
        K::ChangesRequested => verb("changes requested"),
        K::Commented => verb("commented"),
        K::Committed => verb("committed"),
        // The forge's own word, not a generic one: Azure "completed" / "abandoned", Bitbucket
        // "declined", and the rest ("created this", "opened this pull request", …).
        K::Merged | K::Closed | K::Reopened | K::Other => {
            let s = summary.trim();
            // Jira's "set resolution to Done" reads as `resolution → Done`.
            if let Some((field, to)) = s.strip_prefix("set ").and_then(|r| r.split_once(" to ")) {
                return (field.to_string(), Some(to.trim().to_string()));
            }
            let s = s.strip_suffix(" this pull request").or_else(|| s.strip_suffix(" this")).unwrap_or(s);
            verb(s)
        }
    }
}

/// The Activity section — one aligned row per timeline event (age · actor · verb · → target),
/// shared by the work-item view and the PR Conversation tab. Empty when the provider reports no
/// timeline, so a forge without one shows no heading rather than an empty section.
fn activity_lines(theme: &Theme, events: &[TimelineEvent]) -> Vec<Line<'static>> {
    if events.is_empty() {
        return Vec::new();
    }
    let actor = |e: &TimelineEvent| truncate(e.actor.as_ref().map(|a| a.display_name.as_str()).unwrap_or("—"), ACTIVITY_ACTOR_MAX);
    let rows: Vec<(String, String, String, Option<String>)> = events
        .iter()
        .map(|e| {
            let (verb, target) = activity_parts(e.kind, &e.summary);
            (rel_age(e.at), actor(e), verb, target)
        })
        .collect();
    let age_w = rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(0);
    let actor_w = rows.iter().map(|r| r.1.chars().count()).max().unwrap_or(0);
    // Only the verbs that carry a target need to line up; a bare verb ends the row.
    let verb_w = rows.iter().filter(|r| r.3.is_some()).map(|r| r.2.chars().count()).max().unwrap_or(0);
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled("Activity", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))),
    ];
    for (age, who, verb, target) in rows {
        let mut spans = vec![
            Span::styled(format!("  {age:>age_w$}  "), Style::default().fg(theme.dim)),
            Span::styled(format!("{who:<actor_w$}  "), Style::default().fg(theme.blue)),
        ];
        match target {
            Some(t) => {
                spans.push(Span::styled(format!("{verb:<verb_w$} "), Style::default().fg(theme.fg)));
                spans.push(Span::styled("→ ", Style::default().fg(theme.dim)));
                spans.push(Span::styled(t, Style::default().fg(theme.fg)));
            }
            None => spans.push(Span::styled(verb, Style::default().fg(theme.fg))),
        }
        lines.push(Line::from(spans));
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

/// A reviewer's vote as a coloured glyph: green ✓ approved, red ✗ changes requested,
/// yellow … waiting, dim · not yet voted.
fn review_glyph(theme: &Theme, vote: ReviewVote) -> (&'static str, ratatui::style::Color) {
    match vote {
        ReviewVote::Approved | ReviewVote::ApprovedWithSuggestions => ("✓", theme.green),
        ReviewVote::Rejected => ("✗", theme.red),
        ReviewVote::WaitingForAuthor => ("…", theme.yellow),
        ReviewVote::NoVote => ("·", theme.dim),
    }
}

fn pr_conversation_lines(theme: &Theme, pr: &PullRequest, threads: &[CommentThread], timeline: &[TimelineEvent]) -> Vec<Line<'static>> {
    let mut lines = vec![
        field(theme, "Author", pr.author.display_name.clone()),
        field(theme, "Branch", format!("{} → {}", pr.source_ref.clone().unwrap_or_default(), pr.target_ref.clone().unwrap_or_default())),
        {
            let (ck, ckc) = pr_signal(theme, pr);
            field_styled(theme, "Checks", ck, ckc)
        },
        {
            let (mg, mgc) = mergeable_meta(theme, pr.mergeable);
            field_styled(theme, "Mergeable", mg.to_string(), mgc)
        },
        field(theme, "Changes", format!("{} files  +{} -{}", pr.changed_files, pr.additions, pr.deletions)),
    ];
    if !pr.reviewers.is_empty() {
        let mut spans = vec![Span::styled(format!("{:<12}", "Reviewers"), Style::default().fg(theme.dim))];
        for (i, r) in pr.reviewers.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(", ", Style::default().fg(theme.dim)));
            }
            let (glyph, color) = review_glyph(theme, r.vote);
            spans.push(Span::styled(format!("{} ({:?}) ", r.user.display_name, r.vote), Style::default().fg(theme.fg)));
            spans.push(Span::styled(glyph, Style::default().fg(color)));
        }
        lines.push(Line::from(spans));
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
    lines.extend(activity_lines(theme, timeline));
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
            // Explicit gaps: `cell` pads *to* its width, so a value that exactly fills its
            // column (a 9-character sha, say) would otherwise run straight into the next one.
            let gap = || Span::raw(" ".repeat(COL_GAP));
            let mut line = Line::from(vec![
                Span::styled(cell(&c.sha, 9), Style::default().fg(theme.yellow)),
                gap(),
                Span::styled(cell(&c.message, 56), Style::default().fg(theme.fg)),
                gap(),
                Span::styled(cell(&c.author, 16), Style::default().fg(theme.blue)),
                gap(),
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
    // Header, the one-line verdict, the sub-tab bar, then the content. The verdict gets two rows
    // and paints the first, so the blank one separates it from the tab bar below.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(2), Constraint::Length(1), Constraint::Min(3)])
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

    // Where this pull request stands — visible from every tab, not just the Checks one.
    frame.render_widget(Paragraph::new(pr_state_line(theme, &view.pr)), rows[1]);

    // Sub-tab bar.
    let tabs = pr_tabs_line(theme, view);
    // Spans alternate: a leading space, then each tab label followed by a one-column gap.
    let mut x = rows[2].x + 1;
    for (i, label) in tabs.spans.iter().skip(1).step_by(2).take(PR_TABS.len()).enumerate() {
        let w = label.content.chars().count() as u16;
        hit(Rect { x, y: rows[2].y, width: w, height: 1 }.intersection(rows[2]), Hit::PrTab(i));
        x = x.saturating_add(w + 1);
    }
    frame.render_widget(Paragraph::new(tabs), rows[2]);

    // Content.
    if view.tab == 3 {
        render_diff(frame, rows[3], theme, &view.diff, &view.pending);
        return 0; // the Diff tab manages its own scrolling
    }
    // Commits: a row cursor (Enter drills into that commit's diff), scroll follows it.
    if view.tab == 1 {
        let lines = pr_commit_lines(theme, &view.commits, view.commit_sel);
        let inner_h = rows[3].height.saturating_sub(2) as usize;
        let total = view.commits.len();
        let scroll = view.commit_sel.saturating_sub(inner_h / 2).min(total.saturating_sub(inner_h.max(1))) as u16;
        frame.render_widget(Paragraph::new(lines).block(section_block(theme, "Commits")).scroll((scroll, 0)), rows[3]);
        let body = rows[3].inner(ratatui::layout::Margin::new(1, 1));
        hit_rows(body, scroll as usize, (0..total).map(|i| Some(Hit::CommitRow(i))));
        return 0;
    }
    let (title, lines) = match view.tab {
        0 => ("Conversation", pr_conversation_lines(theme, &view.pr, &view.diff.threads, &view.timeline)),
        _ => ("Checks", pr_checks_lines(theme, &view.checks)),
    };
    let para = Paragraph::new(lines).block(section_block(theme, title)).wrap(Wrap { trim: false });
    let max = wrapped_scroll_max(&para, rows[3]);
    frame.render_widget(para.scroll((view.scroll.min(max), 0)), rows[3]);
    max
}

/// How far a wrapped, bordered pane can scroll: the rows its lines take once wrapped to the
/// pane's width, less the rows it has. Counting logical lines instead stops the scroll short
/// of the end whenever a long line wraps, hiding whatever sits at the bottom (the Activity).
fn wrapped_scroll_max(para: &Paragraph, area: Rect) -> u16 {
    // `line_count` adds the block's top and bottom rows but wraps at the width it is given, not
    // inside the block's side borders — so it is handed the inner width of the bordered pane.
    let rows = u16::try_from(para.line_count(area.width.saturating_sub(2))).unwrap_or(u16::MAX);
    rows.saturating_sub(area.height)
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
    lines.extend(activity_lines(theme, &view.timeline));
    let para = Paragraph::new(lines).block(section_block(theme, "Work Item")).wrap(Wrap { trim: false });
    let max = wrapped_scroll_max(&para, rows[1]);
    frame.render_widget(para.scroll((view.scroll.min(max), 0)), rows[1]);
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

/// The footer's palette key — drawn in yellow on every screen, since it finds everything the
/// footer leaves out.
const SEARCH_KEY: &str = "Ctrl-K";

/// Context-aware key glossary for the active tab (azdo-style bar along the bottom). Leads with
/// the Ctrl-K palette ("search anywhere") and the dashboard shortcut (when its server runs),
/// except while the wizard, an overlay, or the quick-filter is capturing input.
fn footer_keys(app: &App) -> Vec<(&'static str, &'static str)> {
    let mut keys = base_footer_keys(app);
    // A focused preview is the item view itself, so its own keys lead; `p` is how back.
    if app.preview_focus && app.wizard.is_none() && app.overlay.is_none() {
        keys.insert(0, ("p", "back to list"));
    }
    // Prepend (not append) so they survive the footer being clipped on narrow terminals: the
    // palette is where everything the footer leaves out (feedback, views, find, …) is found,
    // and the dashboard shortcut shows whenever its server is running.
    if app.wizard.is_none() && app.overlay.is_none() && !app.filtering {
        if app.dashboard_url.is_some() {
            keys.insert(0, ("B", "dashboard"));
        }
        keys.insert(0, (SEARCH_KEY, "search anywhere"));
    }
    keys
}

/// The per-screen key glossary before the global dashboard hint is appended.
fn base_footer_keys(app: &App) -> Vec<(&'static str, &'static str)> {
    if let Some(wizard) = &app.wizard {
        return match wizard.current() {
            Some(Prompt { kind: PromptKind::Pick { .. }, .. }) => vec![("↑↓", "choose"), ("↵", "next"), ("Esc", "cancel")],
            Some(Prompt { kind: PromptKind::Multi { .. }, .. }) => {
                vec![("↑↓", "move"), ("space", "toggle"), ("↵", "next"), ("Esc", "cancel")]
            }
            Some(_) => vec![("type", "value"), ("↵", "next"), ("Esc", "cancel")],
            None => vec![("Esc", "cancel")],
        };
    }
    if let Some(overlay) = &app.overlay {
        return overlay.hint();
    }
    if matches!(app.screen, Screen::Launchpad) {
        return vec![("←→", "columns"), ("↵", "open"), ("D", "dismiss"), ("r", "refresh"), ("?", "help"), ("Ctrl-C", "quit")];
    }
    if let Screen::PrView(v) = &app.screen {
        // A merged PR only offers Revert; an open one offers approve / (reject) / merge.
        let merged = v.pr.status == PullRequestStatus::Merged;
        let acts: Vec<(&'static str, &'static str)> = if merged { vec![("R", "revert")] } else { vec![("a", "approve"), ("m", "merge")] };
        let acts_full: Vec<(&'static str, &'static str)> =
            if merged { vec![("R", "revert")] } else { vec![("a", "approve"), ("x", "reject"), ("m", "merge")] };
        return if v.tab == 3 {
            if v.diff.focus == DiffFocus::Patch {
                let mut keys = vec![("↑↓", "line"), ("]/[", "threads"), ("c", "comment"), ("r", "reply")];
                if !v.pending.is_empty() {
                    keys.push(("s", "submit review"));
                }
                keys.extend([("PgUp/Dn", "jump"), ("Esc", "files"), ("o", "open")]);
                keys
            } else {
                let mut keys = vec![("←→", "tabs"), ("↑↓", "file"), ("↵", "open file"), ("PgUp/Dn", "scroll")];
                keys.extend(acts);
                keys.extend([("o", "open"), ("Esc", "back")]);
                keys
            }
        } else if v.tab == 1 {
            let mut keys = vec![("←→", "tabs"), ("↑↓", "commit"), ("↵", "commit diff")];
            keys.extend(acts);
            keys.extend([("o", "open"), ("Esc", "back")]);
            keys
        } else {
            let mut keys = vec![("←→", "tabs"), ("PgUp/Dn", "scroll")];
            keys.extend(acts_full);
            keys.extend([("c", "comment"), ("r", "reply"), ("o", "open"), ("Esc", "back")]);
            keys
        };
    }
    if matches!(app.screen, Screen::WiView(_)) {
        // Scrolling is left to `?` help: the view's own actions need the room.
        return vec![
            ("u", "update state"),
            ("@", "assign"),
            ("e", "edit"),
            ("c", "comment"),
            ("o", "open"),
            ("Esc/q", "back"),
        ];
    }
    if matches!(app.screen, Screen::Inbox) {
        return vec![("↵", "open item"), ("o", "browser"), ("x", "mark read"), ("A", "all read"), ("Esc", "back")];
    }
    if let Screen::Pipeline(v) = &app.screen {
        if let Some(log) = &v.logs {
            if log.search_input.is_some() {
                return vec![("type", "search"), ("↵", "find"), ("Esc", "cancel")];
            }
            if v.logs_have_keys() {
                let mut keys = vec![("↑↓", "scroll"), ("g/G", "top/end"), ("f", "follow"), ("E", "first error"), ("/", "search")];
                if log.query.is_some() {
                    keys.push(("n/N", "next/prev"));
                }
                if v.log_split.get() {
                    keys.push(("w", "tree"));
                }
                keys.push(("Esc", "close logs"));
                return keys;
            }
            return vec![("↵", "expand"), ("w", "logs"), ("Esc/L", "close logs"), ("q", "back")];
        }
        let mut keys = vec![("↵", "expand"), ("L", "logs")];
        if v.can_respond_approvals && !v.actionable_approvals().is_empty() {
            keys.push(("A", "approve"));
        }
        if matches!(v.run.status, PipelineRunStatus::Queued | PipelineRunStatus::Running) {
            keys.push(("X", "cancel"));
        }
        keys.extend([("T", "trigger"), ("o", "open job"), ("Esc/q", "back")]);
        return keys;
    }
    if matches!(app.screen, Screen::Config(_)) {
        return vec![
            ("a", "add"),
            ("p", "bind-PR"),
            ("w", "bind-WI"),
            ("s", "pipelines"),
            ("x", "remove"),
            ("Esc/q", "back"),
        ];
    }
    // Moving, tab walking, focusing the preview, saved views, repos and find are left to `?`
    // help and the Ctrl-K palette, so the footer keeps the section's own actions.
    let mut keys = Vec::new();
    // Enter's hint only where it opens the row: focusing the preview and expanding or drilling
    // into a pipeline are left out like the others.
    let preview = app.preview_shown() && app.preview.is_some();
    if app.active != 2 && !preview {
        keys.push(("↵", "open"));
    }
    match app.active {
        0 => keys.extend([("f", "status"), ("S", "sort"), ("o", "browser")]),
        1 => keys.extend([("f", "states"), ("S", "sort"), ("o", "browser")]),
        2 => keys.extend([("G", "group"), ("S", "sort"), ("T", "trigger"), ("o", "open")]),
        _ => {}
    }
    keys.extend([
        ("C", "connections"),
        ("r", "refresh"),
        ("t", "theme"),
        ("?", "help"),
        ("Esc/q", "back"),
    ]);
    keys
}

/// A short glyph + label + colour for each notification kind.
fn notif_kind(theme: &Theme, kind: NotificationKind) -> (&'static str, &'static str, ratatui::style::Color) {
    match kind {
        NotificationKind::ReviewRequested => ("◆", "review", theme.accent),
        NotificationKind::Assigned => ("◎", "assigned", theme.accent),
        NotificationKind::Mention => ("@", "mention", theme.magenta),
        NotificationKind::CiFailed => ("✗", "ci failed", theme.red),
        NotificationKind::Comment => ("▪", "comment", theme.dim),
        NotificationKind::StateChange => ("↻", "update", theme.dim),
        NotificationKind::Other => ("•", "activity", theme.dim),
    }
}

fn render_inbox(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let title = format!("Inbox — {} unread of {}", app.unread_count(), app.inbox.len());
    let block = section_block(theme, &title);
    if app.inbox.is_empty() {
        empty(frame, area, theme, "No notifications. The inbox aggregates GitHub, GitLab, and Linear.", block);
        return;
    }

    let rows: Vec<Row> = app
        .inbox
        .iter()
        .map(|r| {
            let n = &r.notification;
            let (glyph, label, color) = notif_kind(theme, n.kind);
            let dot = if n.unread {
                Span::styled("●", Style::default().fg(theme.yellow))
            } else {
                Span::raw(" ")
            };
            let title_style = if n.unread {
                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.dim)
            };
            Row::new(vec![
                Cell::from(dot),
                Cell::from(Span::styled(format!("{glyph} {label}"), Style::default().fg(color))),
                Cell::from(Span::styled(n.title.clone(), title_style)),
                Cell::from(Span::styled(n.context.clone(), Style::default().fg(theme.dim))),
                Cell::from(Span::styled(r.connection.clone(), Style::default().fg(theme.dim))),
                Cell::from(Span::styled(rel_age(n.updated_at), Style::default().fg(theme.dim))),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(1),
        Constraint::Length(12),
        Constraint::Min(20),
        Constraint::Length(22),
        Constraint::Length(12),
        Constraint::Length(5),
    ];
    let table = Table::new(rows, widths)
        .block(block)
        .column_spacing(1)
        .row_highlight_style(highlight(theme))
        .highlight_symbol("▐ ");
    let mut state = TableState::default();
    state.select(Some(app.inbox_sel.min(app.inbox.len().saturating_sub(1))));
    frame.render_stateful_widget(table, area, &mut state);
    let body = area.inner(ratatui::layout::Margin::new(1, 1));
    hit_rows(body, state.offset(), (0..app.inbox.len()).map(|i| Some(Hit::InboxRow(i))));
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
    // Inside an open item, the keys that change it (approve, merge, comment…) get yellow chips,
    // apart from the blue ones that only move around.
    let item_open = matches!(app.screen, Screen::PrView(_) | Screen::WiView(_) | Screen::Pipeline(_))
        && app.overlay.is_none()
        && app.wizard.is_none();
    for (key, label) in footer_keys(app) {
        let search = key == SEARCH_KEY;
        let chip = if search || (item_open && is_write_action(label)) { theme.yellow } else { theme.accent };
        spans.push(Span::styled(format!(" {key} "), bar.fg(theme.bg).bg(chip).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(format!(" {label}  "), bar.fg(if search { theme.yellow } else { theme.fg })));
    }

    // Right side: a transient toast, else the standing status line. "Refreshing…" normally
    // lives up in the tab row beside Notifications; it only lands here when that row is too
    // narrow to hold it.
    let narrow_refresh = match refreshing_in_tab_row(app, area.width) {
        Some(_) => None,
        None => refreshing_label(app),
    };
    let (right, right_style) = if let Some(t) = &app.toast {
        (format!("{t} "), bar.fg(theme.yellow).add_modifier(Modifier::BOLD))
    } else if let Some(text) = &narrow_refresh {
        (format!("{text} "), bar.fg(theme.dim))
    } else {
        (format!("{} ", app.status), bar.fg(theme.dim))
    };
    let mut right_w = right.chars().count().min(70) as u16 + 1;
    // Inside an open item the standing counts give way to its keys when both don't fit — the
    // counts describe the lists behind it. A toast (what just happened) and "Refreshing…"
    // keep their place.
    if item_open && app.toast.is_none() && narrow_refresh.is_none() {
        let keys_w: u16 = spans.iter().map(|s| s.content.chars().count() as u16).sum();
        if keys_w.saturating_add(right_w) > area.width {
            right_w = 0;
        }
    }

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

/// Footer labels for the keys that write to the provider, as the item views name them.
fn is_write_action(label: &str) -> bool {
    matches!(
        label,
        "approve" | "reject" | "merge" | "revert" | "comment" | "reply" | "submit review" | "update state" | "assign" | "edit"
            | "trigger" | "cancel"
    )
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
    // The file each table row shows, for clicks; `None` for directory headings.
    let mut row_file: Vec<Option<usize>> = Vec::new();
    let mut sel_row = 0usize;
    let mut last_dir: Option<&str> = None;
    for (i, f) in diff.files.iter().enumerate() {
        let dir = dir_of(&f.path);
        if last_dir != Some(dir) {
            row_file.push(None);
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
        row_file.push(Some(i));
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
    let body = area.inner(ratatui::layout::Margin::new(1, 1));
    hit_rows(body, state.offset(), row_file.into_iter().map(|f| f.map(Hit::DiffFile)));
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
    // The patch line each display row shows, for clicks; `None` for inline comment boxes.
    let mut row_line: Vec<Option<usize>> = Vec::new();
    let mut cursor_row = 0usize;
    for (i, l) in patch.lines().enumerate() {
        // Every row pushed before the patch line's own is a comment box from the previous line.
        row_line.resize(lines.len(), None);
        row_line.push(Some(i));
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

    row_line.resize(lines.len(), None);
    frame.render_widget(Paragraph::new(lines).block(block).scroll((scroll, 0)), area);
    hit(area, Hit::DiffPatch);
    let body = area.inner(ratatui::layout::Margin::new(1, 1));
    hit_rows(body, scroll as usize, row_line.into_iter().map(|l| l.map(Hit::DiffLine)));
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

/// Semantic token kind → an indexed theme colour (never truecolor RGB), plus the text
/// modifiers the markup kinds need (a heading is bold, a link underlined).
fn hl_style(theme: &Theme, kind: HlKind) -> Style {
    let base = Style::default();
    match kind {
        HlKind::Keyword => base.fg(theme.magenta),
        HlKind::Type => base.fg(theme.cyan),
        HlKind::Str => base.fg(theme.green),
        HlKind::Comment => base.fg(theme.dim),
        HlKind::Number => base.fg(theme.yellow),
        HlKind::Func => base.fg(theme.blue),
        HlKind::Heading => base.fg(theme.magenta).add_modifier(Modifier::BOLD),
        HlKind::Link => base.fg(theme.blue).add_modifier(Modifier::UNDERLINED),
        HlKind::Emph => base.fg(theme.fg).add_modifier(Modifier::BOLD),
        HlKind::Punct | HlKind::Plain => base.fg(theme.fg),
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
        spans.push(Span::styled(text, hl_style(theme, kind)));
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
    // A cache-seeded run's status is whatever it was when the view was last open — it may have
    // gone red since. Say so until the refetch confirms it, rather than painting remembered
    // status identically to live status: the user acts on this (waits on a run, approves a
    // gate), so an unmarked stale "Running" is worse than a slower honest one.
    let mut header = vec![
        Span::styled(format!("{} ", pipeline_glyph(view.run.status, anim)), Style::default().fg(theme.pipeline_color(view.run.status))),
        Span::styled(format!("{:?}", view.run.status), Style::default().fg(theme.pipeline_color(view.run.status)).add_modifier(Modifier::BOLD)),
    ];
    if view.stale {
        header.push(Span::styled(" (unconfirmed)", Style::default().fg(theme.yellow)));
    }
    header.push(Span::styled(format!("   branch {branch}   triggered by {who}"), Style::default().fg(theme.dim)));
    let header = Line::from(header);
    let header_block = section_block(theme, &view.title);
    frame.render_widget(Paragraph::new(header).block(header_block), rows[0]);

    if let Some(line) = banner {
        frame.render_widget(Paragraph::new(line), rows[1]);
    }

    // An open log pane sits beside the tree — or, when there isn't room for both, fills the pane.
    if let Some(log) = &view.logs {
        let split = tree_area.width >= LOG_SPLIT_MIN_WIDTH;
        view.log_split.set(split);
        if !split {
            render_log_pane(frame, tree_area, theme, log);
            return;
        }
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(LOG_TREE_WIDTH), Constraint::Min(20)])
            .split(tree_area);
        render_pipeline_tree(frame, cols[0], theme, view, anim);
        render_log_pane(frame, cols[1], theme, log);
        let (active, idle) = if view.log_focus { (cols[1], cols[0]) } else { (cols[0], cols[1]) };
        mark_focus(frame, active, theme);
        mark_idle(frame, idle, theme);
        return;
    }
    render_pipeline_tree(frame, tree_area, theme, view, anim);
}

/// The stages → jobs → steps tree.
fn render_pipeline_tree(frame: &mut Frame, tree_area: Rect, theme: &Theme, view: &PipelineView, anim: usize) {
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
    let body = tree_area.inner(ratatui::layout::Margin::new(1, 1));
    hit_rows(body, state.offset(), (0..nodes.len()).map(|i| Some(Hit::PipeNode(i))));
}

/// The live state appended to the log pane's title.
fn log_title(log: &LogView) -> String {
    let mut title = log.title.clone();
    if !log.loaded {
        title.push_str("   loading…");
    } else if log.live {
        title.push_str(if log.follow { "   ● live · following" } else { "   ● live · paused" });
    }
    if log.fetch_failed {
        title.push_str("   (update failed — showing last lines)");
    }
    title
}

/// The log pane's bottom line: the open `/` prompt, or the committed search's position, plus
/// any note (`first error at line N`). `None` when there's nothing to say.
fn log_status_line<'a>(theme: &Theme, log: &LogView) -> Option<Line<'a>> {
    let mut spans = Vec::new();
    if let Some(input) = &log.search_input {
        spans.push(Span::styled(format!("/{input}"), Style::default().fg(theme.fg)));
        spans.push(Span::styled("█", Style::default().fg(theme.accent)));
    } else if let Some(q) = &log.query {
        spans.push(Span::styled(format!("/{q}"), Style::default().fg(theme.yellow)));
        let pos = match (log.match_idx, log.matches.len()) {
            (_, 0) => "no matches".to_string(),
            (Some(i), n) => format!("{} of {n}", i + 1),
            (None, n) => format!("{n} matches"),
        };
        spans.push(Span::styled(format!("   {pos}"), Style::default().fg(theme.dim)));
    }
    if let Some(note) = &log.note {
        let lead = if spans.is_empty() { "" } else { "   " };
        spans.push(Span::styled(format!("{lead}{note}"), Style::default().fg(theme.red)));
    }
    (!spans.is_empty()).then(|| Line::from(spans))
}

/// One log line: red when it reports an error, search hits on a yellow background, and the
/// current match's line on the selection background.
fn log_line<'a>(theme: &Theme, text: &str, query: Option<&str>, current: bool) -> Line<'a> {
    let mut base = Style::default().fg(if is_error_line(text) { theme.red } else { theme.fg });
    if current {
        base = base.bg(theme.sel_bg);
    }
    let ranges = query.map(|q| match_ranges(text, q)).unwrap_or_default();
    if ranges.is_empty() {
        return Line::from(Span::styled(text.to_owned(), base));
    }
    let hit = Style::default().fg(theme.bg).bg(theme.yellow).add_modifier(Modifier::BOLD);
    let mut spans = Vec::new();
    let mut at = 0;
    for (start, end) in ranges {
        if start > at {
            spans.push(Span::styled(text[at..start].to_owned(), base));
        }
        spans.push(Span::styled(text[start..end].to_owned(), hit));
        at = end;
    }
    if at < text.len() {
        spans.push(Span::styled(text[at..].to_owned(), base));
    }
    Line::from(spans)
}

/// The scrollable log pane. Only the visible window is built, so a 10k-line log costs no more
/// per frame than a short one.
fn render_log_pane(frame: &mut Frame, area: Rect, theme: &Theme, log: &LogView) {
    hit(area, Hit::LogPane);
    let title = log_title(log);
    let block = section_block(theme, &title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let status = log_status_line(theme, log);
    let body_h = inner.height.saturating_sub(u16::from(status.is_some() && inner.height > 1));
    log.viewport.set(body_h);
    let top = log.effective_scroll() as usize;
    let current = log.match_idx.and_then(|i| log.matches.get(i).copied());
    let lines: Vec<Line> = log
        .lines
        .iter()
        .enumerate()
        .skip(top)
        .take(body_h as usize)
        .map(|(i, l)| log_line(theme, l, log.query.as_deref(), current == Some(i)))
        .collect();
    frame.render_widget(Paragraph::new(lines), Rect { height: body_h, ..inner });
    if let Some(status) = status {
        if inner.height > body_h {
            let row = Rect { y: inner.y + body_h, height: 1, ..inner };
            frame.render_widget(Paragraph::new(status), row);
        }
    }
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
        render_palette(frame, area, theme, overlay, query, candidates, results, *selected);
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
        Overlay::Toggle { items, selected, filter, .. } => (
            crate::overlay::visible_toggle_indices(items, filter.as_deref())
                .into_iter()
                .enumerate()
                .map(|(i, idx)| {
                    let item = &items[idx];
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
                .collect::<Vec<Line>>()
                .into_iter()
                .fold(
                    match filter {
                        // A searchable checklist shows what's being typed, or it looks broken
                        // when the list narrows to nothing.
                        Some(q) => vec![Line::from(Span::styled(
                            format!("search: {q}▏"),
                            Style::default().fg(theme.dim).add_modifier(Modifier::ITALIC),
                        ))],
                        None => Vec::new(),
                    },
                    |mut acc, line| {
                        acc.push(line);
                        acc
                    },
                ),
            theme.green,
        ),
        Overlay::Search { query, items, selected, .. } => {
            let visible = crate::overlay::visible_search_indices(items, query);
            let mut body = vec![
                Line::from(vec![
                    Span::styled(query.clone(), Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)),
                    Span::styled("▏", Style::default().fg(theme.accent)),
                ]),
                Line::from(""),
            ];
            if visible.is_empty() {
                body.push(Line::from(Span::styled("   No one matches.", Style::default().fg(theme.dim))));
            }
            for (i, idx) in visible.into_iter().enumerate() {
                let item = &items[idx];
                // The "nobody" row (Unassigned) reads as an option, not as a person.
                let plain = if item.id.is_none() { theme.dim } else { theme.fg };
                body.push(if i == *selected {
                    Line::from(vec![
                        Span::styled(" ▐ ", Style::default().fg(theme.accent)),
                        Span::styled(item.label.clone(), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
                    ])
                } else {
                    Line::from(Span::styled(format!("   {}", item.label), Style::default().fg(plain)))
                });
            }
            (body, theme.accent)
        }
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

/// The status dot's colour, following the shared green/blue/yellow/red/grey model.
fn tone_color(theme: &Theme, tone: Tone) -> ratatui::style::Color {
    match tone {
        Tone::Good => theme.green,
        Tone::Active => theme.blue,
        Tone::Warn => theme.yellow,
        Tone::Bad => theme.red,
        Tone::Merged => theme.magenta,
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

/// One display line of the palette's result list: a group header, or a result (by its
/// position in `results`).
enum PaletteLine {
    Header(&'static str),
    Row(usize),
}

/// The palette's result lines: each result, with a header above the first of every group
/// (results arrive contiguous per group — see `palette::rank`).
fn palette_lines(candidates: &[PaletteItem], results: &[usize]) -> Vec<PaletteLine> {
    let mut lines = Vec::new();
    let mut group = None;
    for (pos, &i) in results.iter().enumerate() {
        let g = candidates[i].group;
        if group != Some(g) {
            lines.push(PaletteLine::Header(g.label()));
            group = Some(g);
        }
        lines.push(PaletteLine::Row(pos));
    }
    lines
}

/// Prefix legend shown under the results.
const PALETTE_PREFIXES: &str = "> actions  : commands  @ people  # ids  ? keys";

/// The command palette panel: a query line above a windowed, grouped, ranked result list.
#[allow(clippy::too_many_arguments)]
fn render_palette(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    overlay: &Overlay,
    query: &str,
    candidates: &[PaletteItem],
    results: &[usize],
    selected: usize,
) {
    // Up to 16 result lines (headers included), fewer on a short terminal: the rest of the
    // panel (borders, query, spacing, legend, hints) takes 7, plus a line of margin each side.
    let max_rows = (area.height.saturating_sub(9) as usize).clamp(1, 16);
    let width = 76.min(area.width.saturating_sub(6));
    let inner = width.saturating_sub(2) as usize;

    let mut lines: Vec<Line> = Vec::new();

    let count = results.len();
    let mut query_line = vec![
        Span::styled("❯ ", Style::default().fg(theme.accent)),
        Span::styled(query.to_string(), Style::default().fg(theme.fg)),
        Span::styled("█", Style::default().fg(theme.accent)),
    ];
    if !query.is_empty() {
        query_line.push(Span::styled(
            format!("    {count} match{}", if count == 1 { "" } else { "es" }),
            Style::default().fg(theme.dim),
        ));
    }
    lines.push(Line::from(query_line));
    lines.push(Line::from(""));

    if results.is_empty() {
        let msg = match parse_query(query) {
            (Some(g), _) => format!("  No {} match", g.label().to_lowercase()),
            (None, _) => "  No matches".to_string(),
        };
        lines.push(Line::from(Span::styled(msg, Style::default().fg(theme.dim))));
    } else {
        let all = palette_lines(candidates, results);
        // Scroll the window so the selected row stays visible.
        let sel_line = all.iter().position(|l| matches!(l, PaletteLine::Row(p) if *p == selected)).unwrap_or(0);
        let start = if sel_line >= max_rows { sel_line + 1 - max_rows } else { 0 };
        let end = (start + max_rows).min(all.len());
        for line in &all[start..end] {
            let pos = match line {
                PaletteLine::Header(label) => {
                    lines.push(Line::from(Span::styled(
                        format!("   {}", label.to_uppercase()),
                        Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
                    )));
                    continue;
                }
                PaletteLine::Row(pos) => *pos,
            };
            let item = &candidates[results[pos]];
            let is_sel = pos == selected;
            let (cursor, title_style) = if is_sel {
                (" ▐ ", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))
            } else {
                ("   ", Style::default().fg(theme.fg))
            };
            let mut spans = vec![Span::styled(cursor, Style::default().fg(theme.accent))];
            // Items lead with their status dot; everything else with a blank of the same width.
            match item.tone {
                Some(tone) => spans.push(Span::styled("● ", Style::default().fg(tone_color(theme, tone)))),
                None => spans.push(Span::raw("  ")),
            }
            spans.push(Span::styled(format!("{:<5}", item.tag()), Style::default().fg(theme.dim)));
            // Right-aligned key badge: " k " plus a space before it.
            let badge = item.key_hint.as_deref().map(|k| format!(" {k} "));
            let badge_w = badge.as_ref().map_or(0, |b| b.chars().count() + 1);
            let avail = inner.saturating_sub(3 + 2 + 5 + badge_w);
            let title_max = if item.subtitle.is_empty() { avail } else { (avail * 3 / 5).max(12).min(avail) };
            let title = truncate(&item.title, title_max);
            let mut used = title.chars().count();
            spans.push(Span::styled(title, title_style));
            let sub_room = avail.saturating_sub(used + 2);
            if !item.subtitle.is_empty() && sub_room >= 4 {
                let sub = truncate(&item.subtitle, sub_room);
                used += 2 + sub.chars().count();
                spans.push(Span::styled(format!("  {sub}"), Style::default().fg(theme.dim)));
            }
            if let Some(badge) = badge {
                spans.push(Span::raw(" ".repeat(avail.saturating_sub(used) + 1)));
                spans.push(Span::styled(badge, Style::default().fg(theme.yellow).bg(theme.bg)));
            }
            lines.push(Line::from(spans));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(format!(" {PALETTE_PREFIXES}"), Style::default().fg(theme.dim))));
    let hint = overlay
        .hint()
        .into_iter()
        .flat_map(|(k, l)| {
            [
                Span::styled(format!(" {k} "), Style::default().fg(theme.bg).bg(theme.accent).add_modifier(Modifier::BOLD)),
                Span::styled(format!(" {l}  "), Style::default().fg(theme.dim)),
            ]
        })
        .collect::<Vec<_>>();
    lines.push(Line::from(hint));

    let height = lines.len() as u16 + 2;
    let rect = centered_rect(width, height, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.panel))
        .title(Span::styled(format!(" {} ", overlay.title()), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)));

    frame.render_widget(Clear, rect);
    frame.render_widget(Paragraph::new(lines).block(block), rect);
}

/// Every keybinding, grouped by context — the content of the `?` help panel, and the
/// command palette's Keys group.
pub(crate) fn help_sections() -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    vec![
        (
            "Global",
            vec![
                ("1–4", "Jump to a tab"),
                ("Tab  Shift-Tab", "Next / previous tab — from anywhere, an open item included"),
                ("↑/↓  k/j", "Move selection"),
                ("Ctrl-K  Ctrl-P", "Command palette: search items, actions, views, settings, keys"),
                ("i", "Notification inbox (mentions, reviews, CI, assignments)"),
                ("B", "Open the web dashboard in your browser"),
                ("F", "Give feedback through the GitHub issue form"),
                ("↵  p", "Focus the preview pane (140+ cols); its keys then work, p returns"),
                ("Click", "Select a tab or row; click it again to open (a diff line: comment)"),
                ("Wheel", "Scroll the pane under the pointer · Shift-drag selects text"),
                ("P", "Preview pane off / on for this section"),
                ("/", "Quick-filter the list"),
                ("S", "Sort by column (re-pick flips direction)"),
                ("g", "Repositories — which ones this section fetches from"),
                ("o", "Open selected in browser"),
                ("v", "Choose which tabs are visible"),
                ("C", "Connections (opens the dashboard)"),
                ("r", "Refresh    t  cycle theme"),
                ("N", "Notifications — choose which events ping you"),
                ("?", "This help"),
                ("Esc  q", "Back / close — never quits"),
                ("Ctrl-C", "Quit"),
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
                ("@", "Assign (type to search; @ again assigns you)"),
                ("e", "Edit the title, or the description in $EDITOR"),
                ("c", "Comment"),
                ("o", "Open in browser"),
            ],
        ),
        (
            "Pipelines",
            vec![
                ("G", "Group by pipeline / trigger / branch / off"),
                ("Enter or Space (on a group)", "Expand / collapse"),
                ("z  Z", "Collapse / expand every group"),
                ("Enter", "Drill in (stages → jobs → steps)"),
                ("Enter (in drill-in)", "Expand / collapse a node"),
                ("L", "View the selected job's logs (beside the tree; a live job's log updates itself)"),
                ("w (logs open)", "Move the keys between the tree and the log pane"),
                ("f  g  G", "Logs: toggle follow / top / bottom (and follow)"),
                ("E", "Logs: jump to the first error line"),
                ("/  n  N", "Logs: search, next / previous match"),
                ("A", "Approve / reject a waiting gate (GitHub, GitLab; Azure is view-only)"),
                ("o", "Open the selected job in the browser"),
                ("T", "Trigger a run"),
                ("X (in drill-in)", "Cancel a queued or running run"),
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
    // Build each section as a group of lines, then balance the groups across two columns so
    // the whole reference fits on one screen without scrolling.
    let group = |name: &'static str, keys: Vec<(&'static str, &'static str)>| -> Vec<Line<'static>> {
        let mut g = vec![Line::from(Span::styled(name, Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)))];
        for (k, d) in keys {
            g.push(Line::from(vec![
                Span::styled(format!("  {k:<15}"), Style::default().fg(theme.yellow)),
                Span::styled(d, Style::default().fg(theme.fg)),
            ]));
        }
        g.push(Line::from(""));
        g
    };
    let (mut left, mut right): (Vec<Line>, Vec<Line>) = (Vec::new(), Vec::new());
    let (mut lh, mut rh) = (0usize, 0usize);
    for (name, keys) in help_sections() {
        let g = group(name, keys);
        if lh <= rh {
            lh += g.len();
            left.extend(g);
        } else {
            rh += g.len();
            right.extend(g);
        }
    }

    let width = 104.min(area.width.saturating_sub(4));
    let height = area.height.saturating_sub(2).max(12);
    let rect = centered_rect(width, height, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.panel))
        .title(Span::styled(" Keybindings ", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)))
        .title_bottom(Line::from(Span::styled(" ↑↓ scroll · Esc close ", Style::default().fg(theme.dim))).right_aligned());
    let inner = block.inner(rect);

    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .spacing(2)
        .split(inner);
    let scroll = (scroll, 0);
    frame.render_widget(Paragraph::new(left).scroll(scroll).wrap(Wrap { trim: false }), cols[0]);
    frame.render_widget(Paragraph::new(right).scroll(scroll).wrap(Wrap { trim: false }), cols[1]);
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
        PromptKind::Multi { items, on, selected } => {
            for (i, item) in items.iter().enumerate() {
                let tick = if on.get(i).copied().unwrap_or(false) { "[x]" } else { "[ ]" };
                if i == *selected {
                    lines.push(Line::from(vec![
                        Span::styled(" ▐ ", Style::default().fg(accent)),
                        Span::styled(format!("{tick} {item}"), Style::default().fg(accent).add_modifier(Modifier::BOLD)),
                    ]));
                } else {
                    lines.push(Line::from(Span::styled(format!("   {tick} {item}"), Style::default().fg(theme.fg))));
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

    /// A Launchpad entry in `bucket`, from a stub GitHub connection.
    fn lp_entry(bucket: crate::launchpad::Bucket, item: crate::launchpad::EntryItem) -> crate::launchpad::Entry {
        crate::launchpad::Entry { bucket, connection_id: "c".into(), connection: "GH".into(), provider: ProviderType::GitHub, item }
    }

    #[test]
    fn command_center_row_hits_skip_the_bucket_headings() {
        use crate::launchpad::{Bucket, EntryItem};
        let mut app = App::new("slate");
        app.screen = Screen::Launchpad;
        let titled = |t: &str| {
            let mut pr = sample_pr();
            pr.title = t.into();
            pr
        };
        app.lp = vec![
            lp_entry(Bucket::NeedsReview, EntryItem::Pr(titled("Review me first"))),
            lp_entry(Bucket::NeedsFixing, EntryItem::Pr(titled("Then fix me"))),
        ];
        let width = 200;
        let out = render_to_string(&mut app, width, 24);
        let rows: Vec<String> = out.chars().collect::<Vec<_>>().chunks(width as usize).map(|c| c.iter().collect()).collect();
        let row_of = |pos| app.hits.iter().find(|(_, h)| *h == Hit::LpRow { side: 0, pos }).expect("row hit").0.y as usize;
        assert!(rows[row_of(0)].contains("Review me first"));
        assert!(rows[row_of(1)].contains("Then fix me"), "the second bucket's heading and spacer take no slot");
    }

    #[test]
    fn sla_color_bands_grey_yellow_red() {
        let theme = Theme::by_name("slate");
        let now = Utc::now();
        let ago = |h: i64| Some(now - chrono::Duration::hours(h));
        assert_eq!(sla_color(&theme, ago(23), 24, now), theme.dim, "inside the SLA stays grey");
        assert_eq!(sla_color(&theme, ago(24), 24, now), theme.yellow, "at the SLA turns yellow");
        assert_eq!(sla_color(&theme, ago(71), 24, now), theme.yellow);
        assert_eq!(sla_color(&theme, ago(72), 24, now), theme.red, "at three times it turns red");
        assert_eq!(sla_color(&theme, None, 24, now), theme.dim, "no timestamp, nothing to be late against");
        assert_eq!(sla_color(&theme, ago(3), 0, now), theme.red, "a zero SLA is clamped to an hour, not a divide-by-zero");
    }

    #[test]
    fn review_request_age_is_time_waited_and_coloured() {
        use crate::launchpad::{Bucket, EntryItem};
        let theme = Theme::by_name("slate");
        let mut pr = sample_pr();
        pr.created_at = Some(Utc::now() - chrono::Duration::hours(30));
        pr.updated_at = Some(Utc::now() - chrono::Duration::minutes(5));
        let age = |bucket| lp_cells(&theme, &lp_entry(bucket, EntryItem::Pr(pr.clone())), 0, true, 24)[6][0].clone();
        let review = age(Bucket::NeedsReview);
        assert_eq!((review.content.as_ref(), review.style.fg), ("1d", Some(theme.yellow)), "waited since opened, past the SLA");
        let mine = age(Bucket::YourOpenPrs);
        assert_eq!((mine.content.as_ref(), mine.style.fg), ("5m", Some(theme.dim)), "other buckets keep last-activity grey");
    }

    fn reviewer(name: &str, vote: ReviewVote) -> Reviewer {
        Reviewer { user: User { id: name.into(), display_name: name.into(), handle: None, avatar_url: None }, vote, is_required: false }
    }

    fn sample_pr() -> PullRequest {
        PullRequest {
            repository: None,
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

    /// The frame as rows, for assertions about *where* on screen something sits.
    fn render_to_rows(app: &mut App, w: u16, h: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| render(f, app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect()).collect()
    }

    fn pr_view(tab: usize, checks: Vec<CheckRun>, files: Vec<FileChange>) -> crate::app::PrView {
        use crate::app::DiffView;
        crate::app::PrView {
            timeline: Vec::new(),
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
            reply_target: None,
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

    /// The PR screen has to answer "where does this stand" without you reading a field list or
    /// opening the Checks tab. One line, from `pr_state`, above the sub-tabs.
    #[test]
    fn pr_screen_states_where_the_pull_request_stands() {
        use crate::app::Screen;
        let line = |f: &dyn Fn(&mut PullRequest)| {
            let mut app = App::new("slate");
            let mut v = pr_view(0, vec![], vec![]);
            v.pr.check_summary = Some(CheckSummary { successful: 2, in_progress: 0, failed: 6, neutral: 0 });
            v.pr.checks = CheckStatus::Failed;
            f(&mut v.pr);
            app.screen = Screen::PrView(Box::new(v));
            render_to_string(&mut app, 92, 14)
        };
        let green = |p: &mut PullRequest| {
            p.checks = CheckStatus::Passed;
            p.check_summary = Some(CheckSummary { successful: 8, in_progress: 0, failed: 0, neutral: 0 });
        };

        // The count is the failures, not total-minus-passed.
        assert!(line(&|_p| {}).contains("Blocked — 6 of 8 checks failed"));
        assert!(line(&|p| {
            green(p);
            p.mergeable = MergeableState::Conflicting;
        })
        .contains("Blocked — conflicts with main"));
        assert!(line(&|p| {
            green(p);
            p.reviewers = vec![reviewer("sam", ReviewVote::Rejected)];
        })
        .contains("Blocked — changes requested by sam"));
        assert!(line(&|p| {
            p.checks = CheckStatus::Pending;
            p.check_summary = Some(CheckSummary { successful: 5, in_progress: 3, failed: 0, neutral: 0 });
        })
        .contains("Checks running — 5 of 8 done"));
        // Green means nothing is in your way — approval or not.
        assert!(line(&green).contains("Nothing blocking — 8 of 8 checks passed, no reviews yet"));
        assert!(line(&|p| {
            green(p);
            p.reviewers = vec![reviewer("alice", ReviewVote::Approved)];
        })
        .contains("Ready to merge — 8 of 8 checks passed, approved by alice"));
        // Lifecycle short-circuits: a merged PR keeps stale votes and merge state, and must not
        // be described by them.
        assert!(line(&|p| {
            p.status = PullRequestStatus::Merged;
            p.mergeable = MergeableState::Conflicting;
        })
        .contains("Merged into main"));
        assert!(line(&|p| p.is_draft = true).contains("Draft — not open for review yet"));
    }

    /// The verdict *sentence* is built twice — here and in the web dashboard's `format.ts` —
    /// because the two frontends share no runtime. AGENTS.md forbids a logic fork between them,
    /// so both suites read `testdata/pr_state_cases.json` and must produce the same string.
    /// Change the wording on one side and the other side's tests fail.
    #[test]
    fn pr_state_sentences_match_the_shared_fixture() {
        let cases: serde_json::Value = serde_json::from_str(include_str!("../../../testdata/pr_state_cases.json")).expect("fixture parses");
        let theme = Theme::by_name("slate");
        for case in cases.as_array().expect("fixture is a list") {
            let g = |k: &str| case[k].as_str().unwrap_or_default().to_string();
            let mut pr = sample_pr();
            pr.target_ref = Some("main".into());
            pr.updated_at = None; // keeps "Merged into main" free of a drifting age
            pr.status = serde_json::from_value(case["status"].clone()).expect("status");
            pr.is_draft = case["is_draft"].as_bool().expect("is_draft");
            pr.checks = serde_json::from_value(case["checks"].clone()).expect("checks");
            pr.mergeable = serde_json::from_value(case["mergeable"].clone()).expect("mergeable");
            pr.check_summary = serde_json::from_value(case["summary"].clone()).expect("summary");
            pr.reviewers = case["votes"]
                .as_array()
                .expect("votes")
                .iter()
                .map(|v| {
                    let name = v[0].as_str().expect("reviewer name");
                    reviewer(name, serde_json::from_value(v[1].clone()).expect("vote"))
                })
                .collect();

            // Compare wording, not spacing: the terminal pads the glyph with two columns while
            // the dashboard uses a CSS gap. Both are right for their medium.
            let line = pr_state_line(&theme, &pr);
            let raw: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            let rendered = raw.split_whitespace().collect::<Vec<_>>().join(" ");
            assert_eq!(rendered, g("expect"), "case: {}", g("name"));
        }
    }

    /// The two gates on a merge are now both in the field list, and both say which way they fell.
    #[test]
    fn pr_screen_lists_both_merge_gates() {
        use crate::app::Screen;
        let mut app = App::new("slate");
        let mut v = pr_view(0, vec![], vec![]);
        v.pr.check_summary = Some(CheckSummary { successful: 8, in_progress: 0, failed: 0, neutral: 0 });
        v.pr.mergeable = MergeableState::Unknown;
        app.screen = Screen::PrView(Box::new(v));
        let out = render_to_string(&mut app, 92, 20);
        assert!(out.contains("Checks") && out.contains("✓ 8/8"), "checks field with its roll-up");
        assert!(out.contains("Mergeable") && out.contains("· unknown"), "merge state as a word, not the raw enum");
        assert!(!out.contains("Unknown"), "the raw enum name is gone");
    }

    /// `cell` pads *to* a width, so a value that exactly fills its column touches the next one —
    /// a 9-character short sha ran straight into the commit subject.
    #[test]
    fn commit_rows_keep_a_gap_between_every_column() {
        use crate::app::Screen;
        let mut app = App::new("slate");
        let mut v = pr_view(1, vec![], vec![]);
        v.commits = vec![Commit {
            sha: "497854a95".into(), // exactly the column width
            message: "testing more and more".into(),
            author: "magna-nz".into(),
            date: None,
            url: None,
        }];
        app.screen = Screen::PrView(Box::new(v));
        let out = render_to_string(&mut app, 120, 20);
        assert!(!out.contains("497854a95testing"), "sha must not run into the subject");
        assert!(out.contains("497854a95   testing more and more"), "one gap between them");
    }

    /// The verdict needs air between it and the tab bar, so it reads as its own statement rather
    /// than a label on the tabs.
    #[test]
    fn a_blank_row_separates_the_verdict_from_the_tab_bar() {
        use crate::app::Screen;
        let mut app = App::new("slate");
        let mut v = pr_view(0, vec![], vec![]);
        v.pr.checks = CheckStatus::Failed;
        v.pr.check_summary = Some(CheckSummary { successful: 7, in_progress: 0, failed: 1, neutral: 0 });
        app.screen = Screen::PrView(Box::new(v));
        let width = 74usize;
        let out = render_to_string(&mut app, width as u16, 15);
        let chars: Vec<char> = out.chars().collect();
        let rows: Vec<String> = chars.chunks(width).map(|r| r.iter().collect()).collect();
        let verdict = rows.iter().position(|r| r.contains("Blocked — 1 of 8 checks failed")).expect("verdict row");
        assert!(rows[verdict + 1].trim().is_empty(), "blank row under the verdict");
        assert!(rows[verdict + 2].contains("Conversation"), "tab bar follows the blank row");
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
    fn merged_pr_is_magenta_distinct_from_open_green() {
        let theme = Theme::by_name("slate");
        let mut pr = sample_pr();
        pr.is_draft = false;
        pr.status = PullRequestStatus::Merged;
        assert_eq!(pr_status(&theme, &pr).1, theme.magenta, "merged is magenta");
        pr.status = PullRequestStatus::Open;
        assert_eq!(pr_status(&theme, &pr).1, theme.green, "open stays green");
    }

    #[test]
    fn reviewers_show_a_green_tick_or_red_cross_by_vote() {
        use forgetop_core::domain::{Reviewer, ReviewVote};
        let theme = Theme::by_name("slate");
        let who = |name: &str| User { id: name.into(), display_name: name.into(), handle: None, avatar_url: None };
        let mut pr = sample_pr();
        pr.reviewers = vec![
            Reviewer { user: who("Priya Nair"), vote: ReviewVote::Approved, is_required: true },
            Reviewer { user: who("Marcus Lee"), vote: ReviewVote::Rejected, is_required: false },
        ];
        let lines = pr_conversation_lines(&theme, &pr, &[], &[]);

        let green_tick = lines.iter().any(|l| l.spans.iter().any(|s| s.content.as_ref() == "✓" && s.style.fg == Some(theme.green)));
        let red_cross = lines.iter().any(|l| l.spans.iter().any(|s| s.content.as_ref() == "✗" && s.style.fg == Some(theme.red)));
        assert!(green_tick, "an approved reviewer gets a green tick");
        assert!(red_cross, "a changes-requested reviewer gets a red cross");
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

    fn scope(selected: usize, available: Option<usize>, truncated: bool, none_selected: bool) -> crate::app::ScopeSummary {
        crate::app::ScopeSummary {
            connections: vec!["c".into()],
            selected,
            available,
            truncated,
            none_selected,
        }
    }

    #[test]
    fn pr_list_shows_the_repository_a_row_belongs_to() {
        // One connection now spans an account, so the provider/connection tag no longer tells
        // rows apart — the repository does.
        let mut app = App::new("slate");
        app.screen = Screen::List;
        let mut pr = sample_pr();
        pr.repository = Some("acme/payments".into());
        app.prs.push(crate::app::PrRow { connection_id: "c".into(), connection: "GH".into(), provider: ProviderType::GitHub, pr });
        app.pr_state.select(Some(0));
        let out = render_to_string(&mut app, 160, 24);
        assert!(out.contains("Repository"), "repository header present");
        assert!(out.contains("payments"), "row names the repository it lives in");
    }

    /// The old Checks column reported a conflicted or changes-requested pull request as ✓ 8/8 —
    /// identical to a genuinely clean one. The State column names whatever is actually stopping it.
    #[test]
    fn pr_list_state_column_names_the_blocker_not_just_the_checks() {
        let row = |repo: &str, f: &dyn Fn(&mut PullRequest)| {
            let mut pr = sample_pr();
            pr.repository = Some(repo.into());
            pr.checks = CheckStatus::Passed;
            pr.check_summary = Some(CheckSummary { successful: 8, in_progress: 0, failed: 0, neutral: 0 });
            f(&mut pr);
            crate::app::PrRow { connection_id: "c".into(), connection: "GH".into(), provider: ProviderType::GitHub, pr }
        };
        let mut app = App::new("slate");
        app.screen = Screen::List;
        app.prs = vec![
            row("acme/clean", &|_p| {}),
            row("acme/conflicted", &|p| p.mergeable = MergeableState::Conflicting),
            row("acme/rejected", &|p| p.reviewers = vec![reviewer("sam", ReviewVote::Rejected)]),
            row("acme/failing", &|p| {
                p.checks = CheckStatus::Failed;
                p.check_summary = Some(CheckSummary { successful: 2, in_progress: 0, failed: 6, neutral: 0 });
            }),
            row("acme/running", &|p| {
                p.checks = CheckStatus::Pending;
                p.check_summary = Some(CheckSummary { successful: 5, in_progress: 3, failed: 0, neutral: 0 });
            }),
        ];
        app.pr_state.select(Some(0));
        let out = render_to_string(&mut app, 180, 24);

        assert!(out.contains("State") && !out.contains("Checks"), "the column is State now");
        assert!(out.contains("⚠ conflicts"), "a conflict is named, not hidden behind green checks");
        assert!(out.contains("⚠ changes"), "a changes-requested review is named");
        assert!(out.contains("✗ 2/8"), "failing checks keep their roll-up");
        assert!(out.contains("◐ 5/8"), "in-flight checks read as running, not passed");
        assert!(out.contains("✓ 8/8"), "the genuinely clean row still says so");
    }

    #[test]
    fn section_header_reports_how_much_of_the_account_is_in_scope() {
        let mut app = App::new("slate");
        app.screen = Screen::List;
        app.prs.push(crate::app::PrRow { connection_id: "c".into(), connection: "GH".into(), provider: ProviderType::GitHub, pr: sample_pr() });
        app.pr_state.select(Some(0));

        app.repo_scope[0] = Some(scope(5, Some(37), false, false));
        assert!(render_to_string(&mut app, 160, 24).contains("Repos · 5 of 37"));

        // Truncation is marked rather than presented as a total.
        app.repo_scope[0] = Some(scope(5, Some(500), true, false));
        assert!(render_to_string(&mut app, 160, 24).contains("Repos · 5 of 500+"));

        // Before discovery has answered there is no denominator to invent.
        app.repo_scope[0] = Some(scope(5, None, false, false));
        let out = render_to_string(&mut app, 160, 24);
        assert!(out.contains("Repos · 5") && !out.contains("Repos · 5 of"));
    }

    #[test]
    fn an_empty_scope_reads_as_a_choice_not_as_an_empty_section() {
        // Nothing was fetched because nothing was asked for. "No pull requests" would say the
        // opposite of what happened, and wouldn't point at the fix.
        let mut app = App::new("slate");
        app.screen = Screen::List;
        app.health.push(forgetop_core::service::ConnectionHealth {
            connection: forgetop_core::provider::Connection {
                id: "c".into(),
                provider_type: ProviderType::GitHub,
                display_name: "GH".into(),
                base_url: None,
                organization: None,
                project: None,
                repository: None,
                username: None,
                credential_ref: None,
                repo_scope: Some(vec![]),
            },
            healthy: true,
        });
        app.repo_scope[0] = Some(scope(0, Some(37), false, true));
        let out = render_to_string(&mut app, 160, 24);
        assert!(out.contains("No repositories selected"), "says why the list is empty");
        assert!(!out.contains("No pull requests"), "and doesn't claim there are none");
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
            event: None,
            attempt: None,
            pull_request: None,
            repository: None,
            id: "r1".into(),
            definition_id: "ci".into(),
            number: Some(101),
            name: Some("CI".into()),
            title: None,
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
    fn the_log_pane_sits_beside_the_tree_and_fills_the_pane_when_narrow() {
        use crate::app::{LogView, Screen};
        let mut view = PipelineView::new("CI #101".into(), sample_run(), "demo".into(), ProviderType::GitHub, "ci".into(), None);
        let mut log = LogView::with_lines("Logs · compile", "j1", vec![]);
        log.set_text("step one\nerror: boom\nFAILED here\ndone");
        log.live = true;
        log.follow = true;
        log.query = Some("here".into());
        log.search_input = None;
        view.logs = Some(log);
        let mut app = App::new("slate");
        app.screen = Screen::Pipeline(Box::new(view));

        let wide = render_to_string(&mut app, 160, 30);
        assert!(wide.contains("Stages · jobs · steps"), "the tree stays beside the logs");
        assert!(wide.contains("Logs · compile") && wide.contains("● live · following"));
        assert!(wide.contains("error: boom"));
        assert!(wide.contains("/here"), "the committed search shows on the status line");
        let Screen::Pipeline(v) = &app.screen else { panic!() };
        assert!(v.log_split.get());

        let narrow = render_to_string(&mut app, 70, 30);
        assert!(!narrow.contains("Stages · jobs · steps"), "narrow: logs alone");
        let Screen::Pipeline(v) = &app.screen else { panic!() };
        assert!(!v.log_split.get() && v.logs_have_keys(), "and the logs take the keys");

        // Tiny terminals and empty logs must not panic.
        if let Screen::Pipeline(v) = &mut app.screen {
            v.logs = Some(LogView::with_lines("Logs", "j1", vec![]));
        }
        render_to_string(&mut app, 20, 6);
        render_to_string(&mut app, 120, 8);
    }

    /// A cache-seeded run must not present remembered status as live — the user waits on runs
    /// and approves gates off this header.
    #[test]
    fn a_cache_seeded_run_marks_its_status_unconfirmed() {
        use crate::app::Screen;
        let mut app = App::new("slate");
        let mut view = PipelineView::new(
            "CI #101".into(),
            sample_run(),
            "demo".into(),
            ProviderType::GitHub,
            "ci".into(),
            Some("main".into()),
        );
        view.stale = true;
        app.screen = Screen::Pipeline(Box::new(view));
        let out = render_to_string(&mut app, 120, 30);
        assert!(out.contains("(unconfirmed)"), "a cache-seeded run says its status is unconfirmed");
    }

    #[test]
    fn a_confirmed_run_carries_no_unconfirmed_marker() {
        use crate::app::Screen;
        let mut app = App::new("slate");
        let view = PipelineView::new(
            "CI #101".into(),
            sample_run(),
            "demo".into(),
            ProviderType::GitHub,
            "ci".into(),
            Some("main".into()),
        );
        app.screen = Screen::Pipeline(Box::new(view));
        let out = render_to_string(&mut app, 120, 30);
        assert!(!out.contains("unconfirmed"), "a live run is not marked");
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

        // Collapsed, the group header carries the flag — otherwise a gate waiting inside a
        // closed group would be invisible.
        // The tree glyph `└` closes a group, so it appears on a child row and nowhere else —
        // the precise test for whether the run itself is on screen.
        let out = render_to_string(&mut app, 150, 24);
        assert!(out.contains("Approval"), "approval column header present");
        assert!(out.contains("approval needed"), "the collapsed group announces the gate");
        assert!(!out.contains('└'), "and the run row is put away");

        // Expanded, the run's own Approval cell carries it. This is the assertion the test
        // was making before grouping existed; grouping moved it behind an expand.
        let key = match &app.pipe_lines()[0] {
            crate::app::PipeLine::Head(h) => h.key.clone(),
            crate::app::PipeLine::Run(_) => panic!("header"),
        };
        app.pipe_expanded.insert(key);
        let out = render_to_string(&mut app, 150, 24);
        assert!(out.contains('└'), "the run row itself is rendered, hanging off the group");
        assert!(out.contains("approval needed"), "and the row's Approval cell is flagged");

        // Ungrouped, the row stands alone and must still be flagged.
        app.pipe_group = crate::app::PipeGroup::Off;
        let out = render_to_string(&mut app, 150, 24);
        assert!(!out.contains('└') && !out.contains('▸'), "no tree glyphs when grouping is off");
        assert!(out.contains("approval needed"), "and the run is still flagged");
    }

    /// The default landing for the tab: one line per pipeline, carrying the repository the
    /// flat table has no column for, and the runs put away until asked for.
    #[test]
    fn pipelines_list_groups_by_pipeline_and_lands_collapsed() {
        use crate::app::{PipeGroup, PipeRow};
        let row = |def: &str, sha: &str, status: PipelineRunStatus| {
            let mut run = sample_run();
            run.id = format!("{def}-{sha}");
            run.definition_id = def.to_lowercase();
            run.repository = Some("nz/app".into());
            run.commit_sha = Some(sha.into());
            run.status = status;
            PipeRow {
                connection_id: "c".into(),
                connection: "GH".into(),
                provider: ProviderType::GitHub,
                definition_name: Some(def.into()),
                awaiting_approval: false,
                run,
            }
        };
        let mut app = App::new("slate");
        app.screen = Screen::List;
        app.active = 2;
        app.pipes = vec![
            row("CI", "aaa", PipelineRunStatus::Succeeded),
            row("CI", "bbb", PipelineRunStatus::Failed),
            row("Integration", "aaa", PipelineRunStatus::Succeeded),
        ];
        app.pipe_state.select(Some(0));

        let out = render_to_string(&mut app, 150, 24);
        assert!(out.contains("by pipeline"), "the title says how the list is arranged");
        assert!(out.contains("2 runs"), "the CI header counts what is underneath it");
        assert!(out.contains("1 failed"), "a failure inside a collapsed group is still announced");
        // The owner prefix is shared by every row, so it is elided: `nz/app` reads as `app`.
        // Asserting on "app" alone would pass either way — it is a substring of "nz/app".
        assert!(out.contains("Repository"), "the repository has a column of its own");
        assert!(!out.contains("nz/app"), "shared owner prefix elided");
        assert!(out.contains("app"), "but the repository itself is still named");
        assert!(out.contains("▸"), "groups are collapsed");

        // Expanding the first group brings its runs onto the screen.
        let key = match &app.pipe_lines()[0] {
            crate::app::PipeLine::Head(h) => h.key.clone(),
            crate::app::PipeLine::Run(_) => panic!("header"),
        };
        app.pipe_expanded.insert(key);
        let out = render_to_string(&mut app, 150, 24);
        assert!(out.contains("▾"), "the opened group is marked open");

        // Grouping off is the list the tab had before grouping existed.
        app.pipe_group = PipeGroup::Off;
        let out = render_to_string(&mut app, 150, 24);
        assert!(!out.contains("▸") && !out.contains("▾"), "no headers when grouping is off");
        assert!(!out.contains("by pipeline"), "and the title drops the arrangement note");
    }

    /// Builds a pipelines list from (definition, repo, branch, sha, minutes-ago, status).
    fn pipe_list(rows: &[(&str, &str, &str, &str, i64, PipelineRunStatus)]) -> App {
        use crate::app::PipeRow;
        let mut app = App::new("slate");
        app.screen = Screen::List;
        app.active = 2;
        app.pipes = rows
            .iter()
            .enumerate()
            .map(|(n, (def, repo, branch, sha, mins, st))| {
                let mut run = sample_run();
                run.id = format!("{def}-{sha}");
                run.definition_id = def.to_lowercase();
                run.repository = Some((*repo).into());
                run.branch = Some((*branch).into());
                run.commit_sha = Some((*sha).into());
                run.number = Some(n as i64 + 1);
                run.name = None;
                run.started_at = Some(Utc::now() - chrono::Duration::minutes(*mins));
                run.status = *st;
                PipeRow {
                    connection_id: "c".into(),
                    connection: "GH".into(),
                    provider: ProviderType::GitHub,
                    definition_name: Some((*def).into()),
                    awaiting_approval: false,
                    run,
                }
            })
            .collect();
        app.pipe_state.select(Some(0));
        app
    }

    /// A column every row fills identically is nine characters of nothing. Provider only
    /// earns its place once there is more than one provider to tell apart.
    #[test]
    fn the_provider_column_appears_only_when_providers_differ() {
        use PipelineRunStatus::Succeeded as S;
        let mut app = pipe_list(&[
            ("CI", "nz/app", "main", "aaa", 10, S),
            ("Integration", "nz/app", "main", "aaa", 12, S),
        ]);
        let out = render_to_string(&mut app, 150, 16);
        assert!(!out.contains("Provider"), "one provider, so no Provider column");

        app.pipes[1].provider = ProviderType::GitLab;
        app.pipes[1].connection = "GL".into();
        let out = render_to_string(&mut app, 150, 16);
        assert!(out.contains("Provider"), "two providers, so the column is worth its width");
    }

    /// The owner prefix repeats down the whole column when there is only one owner.
    #[test]
    fn a_shared_repository_owner_is_elided() {
        use PipelineRunStatus::Succeeded as S;
        let mut app = pipe_list(&[
            ("CI", "magna-nz/forgetop", "main", "aaa", 10, S),
            ("Release", "magna-nz/test-pg", "main", "bbb", 12, S),
        ]);
        let out = render_to_string(&mut app, 150, 16);
        assert!(out.contains("forgetop") && out.contains("test-pg"), "both repositories named");
        assert!(!out.contains("magna-nz/"), "the owner every row shares is dropped");

        // A second owner makes the prefix meaningful again.
        app.pipes[1].run.repository = Some("other-org/test-pg".into());
        let out = render_to_string(&mut app, 150, 16);
        assert!(out.contains("magna-nz/") && out.contains("other-org/"), "two owners, both kept");
    }

    /// Expanded, a run puts its own branch where the group's name sits — so every row has a
    /// value right after the tree gutter instead of a run of empty columns.
    #[test]
    fn a_child_run_takes_the_subject_slot_with_what_varies() {
        use PipelineRunStatus::Succeeded as S;
        let mut app = pipe_list(&[
            ("CI", "nz/app", "main", "aaa", 10, S),
            ("CI", "nz/app", "v2.0", "bbb", 20, S),
        ]);
        // "Pipelines" is the section title, so `contains("Pipeline")` proves nothing on its
        // own — the heading is checked exactly, and Branch by its absence.
        let out = render_to_string(&mut app, 150, 16);
        assert!(!out.contains("Branch"), "collapsed: no Branch column");
        assert!(!out.contains("main") && !out.contains("v2.0"), "the group spans branches, so names none");

        let key = match &app.pipe_lines()[0] {
            crate::app::PipeLine::Head(h) => h.key.clone(),
            crate::app::PipeLine::Run(_) => panic!("header"),
        };
        app.pipe_expanded.insert(key);
        let out = render_to_string(&mut app, 150, 16);
        assert!(out.contains("Pipeline / Branch"), "the heading names both kinds of value");
        assert!(out.contains("─ main") && out.contains("─ v2.0"), "each run names its own branch");
        assert!(out.contains('├') && out.contains('└'), "and hangs off the tree gutter");
    }

    /// Grouped by branch the relationship inverts: the header names the branch, and the runs
    /// under it name their pipelines.
    #[test]
    fn grouping_by_branch_inverts_what_the_subject_column_holds() {
        use PipelineRunStatus::Succeeded as S;
        let mut app = pipe_list(&[
            ("CI", "nz/app", "main", "aaa", 10, S),
            ("Integration", "nz/app", "main", "aaa", 12, S),
        ]);
        app.pipe_group = crate::app::PipeGroup::Branch;
        let key = match &app.pipe_lines()[0] {
            crate::app::PipeLine::Head(h) => h.key.clone(),
            crate::app::PipeLine::Run(_) => panic!("header"),
        };
        app.pipe_expanded.insert(key);
        let out = render_to_string(&mut app, 150, 16);
        assert!(out.contains("Branch / Pipeline"), "heading reads the other way round");
        assert!(out.contains("─ CI") && out.contains("─ Integration"), "runs name their pipelines");
    }

    /// Every state is named. A tick and a cross were left to speak for themselves, which asked
    /// you to decode a symbol for the two outcomes that matter most — and said nothing at all
    /// once colour is gone.
    #[test]
    fn the_status_column_names_every_state() {
        use PipelineRunStatus::{Canceled, Failed, Running, Succeeded};
        let mut app = pipe_list(&[("CI", "nz/app", "main", "aaa", 10, Succeeded)]);

        for (status, word) in [(Succeeded, "Succeeded"), (Failed, "Failed"), (Canceled, "Canceled"), (Running, "Running")] {
            app.pipes[0].run.status = status;
            let out = render_to_string(&mut app, 150, 16);
            assert!(out.contains(word), "the header says {word}");
        }

        // The same rule has to hold for a run rendered as a child, not just for a header.
        let key = match &app.pipe_lines()[0] {
            crate::app::PipeLine::Head(h) => h.key.clone(),
            crate::app::PipeLine::Run(_) => panic!("header"),
        };
        app.pipe_expanded.insert(key);
        app.pipes[0].run.status = Failed;
        let out = render_to_string(&mut app, 150, 16);
        assert!(out.contains('└'), "the child is on screen");
        // Its header says "Failed" one line up, so the child shows the glyph alone — see
        // `a_groups_runs_carry_only_their_glyph`.
        assert_eq!(out.matches("Failed").count(), 1, "the state is named once for the group");
    }

    /// The header names where the pipeline stands; beneath it, ✓ / ✗ is what the eye scans
    /// for, so a group's runs carry only their glyph — no word, even where the state changes.
    #[test]
    fn a_groups_runs_carry_only_their_glyph() {
        use PipelineRunStatus::{Failed, Succeeded};
        let mut app = pipe_list(&[
            ("CI", "nz/app", "main", "aaa", 10, Succeeded),
            ("CI", "nz/app", "main", "bbb", 20, Succeeded),
            ("CI", "nz/app", "main", "ccc", 30, Failed),
        ]);
        let key = match &app.pipe_lines()[0] {
            crate::app::PipeLine::Head(h) => h.key.clone(),
            crate::app::PipeLine::Run(_) => panic!("header"),
        };
        app.pipe_expanded.insert(key);

        let out = render_to_string(&mut app, 150, 16);
        // The newest run passed, so the header reads "Succeeded"; nothing beneath it has a word.
        assert_eq!(out.matches("Succeeded").count(), 1, "only the header names the state");
        assert_eq!(out.matches("Failed").count(), 0, "not even the run that broke the streak");
        assert_eq!(out.matches('✓').count(), 3, "header plus the two passing runs, glyph each");
        assert_eq!(out.matches('✗').count(), 1, "the failed run is marked by its glyph");

        // Ungrouped, neighbouring runs are unrelated — an elided word would read as a missing
        // one, so every row spells its state out.
        app.pipe_group = crate::app::PipeGroup::Off;
        let out = render_to_string(&mut app, 150, 16);
        assert_eq!(out.matches("Succeeded").count(), 2, "a flat list repeats");
    }

    /// Approval is the rarest column of all — it should not hold the table open when nothing
    /// anywhere is waiting.
    #[test]
    fn the_approval_column_appears_only_when_a_gate_is_waiting() {
        use PipelineRunStatus::Succeeded as S;
        let mut app = pipe_list(&[("CI", "nz/app", "main", "aaa", 10, S)]);
        let out = render_to_string(&mut app, 150, 16);
        assert!(!out.contains("Approval"), "nothing waiting, so no column");

        app.pipes[0].awaiting_approval = true;
        let out = render_to_string(&mut app, 150, 16);
        assert!(out.contains("Approval") && out.contains("approval needed"), "a gate brings it back");
    }

    /// The arrow has to mark the column the sort actually applies to. The columns are picked
    /// per render, so a fixed key-to-index table points at whatever now sits at that index —
    /// it marked Repository while the list was sorted by start time.
    #[test]
    fn the_sort_arrow_follows_the_live_column_set() {
        use forgetop_core::config::SortPref;
        use PipelineRunStatus::Succeeded as S;
        let mut app = pipe_list(&[
            ("CI", "nz/app", "main", "aaa", 10, S),
            ("Integration", "nz/app", "v2.0", "bbb", 20, S),
        ]);

        // The heading that carries the arrow, whatever it is.
        let marked = |out: &str| -> String {
            out.lines()
                .find(|l| l.contains('▼') || l.contains('▲'))
                .and_then(|l| {
                    let cut = l.find(['▼', '▲'])?;
                    Some(l[..cut].split("   ").last().unwrap_or("").trim().to_string())
                })
                .unwrap_or_default()
        };

        app.pipe_sort = Some(SortPref { key: "started".into(), desc: true });
        assert_eq!(marked(&render_to_string(&mut app, 150, 16)), "Started", "grouped by pipeline");

        app.pipe_group = crate::app::PipeGroup::Trigger;
        assert_eq!(marked(&render_to_string(&mut app, 150, 16)), "Started", "and by trigger");

        app.pipe_group = crate::app::PipeGroup::Off;
        assert_eq!(marked(&render_to_string(&mut app, 150, 16)), "Started", "and ungrouped");

        app.pipe_sort = Some(SortPref { key: "pipeline".into(), desc: false });
        assert_eq!(marked(&render_to_string(&mut app, 150, 16)), "Pipeline", "the subject column");

        // Sorting by a column this arrangement does not show draws no arrow — better than
        // drawing one over the wrong heading.
        app.pipe_group = crate::app::PipeGroup::Pipeline;
        app.pipe_sort = Some(SortPref { key: "provider".into(), desc: false });
        let out = render_to_string(&mut app, 150, 16);
        assert!(!out.contains('▼') && !out.contains('▲'), "one provider, so no column and no arrow");
    }

    /// Grouping off must be the list the tab had before grouping existed — which means the
    /// branch is on screen, since there is no header or child row to carry it.
    #[test]
    fn the_ungrouped_list_still_has_a_branch_column() {
        use PipelineRunStatus::Succeeded as S;
        let mut app = pipe_list(&[("CI", "nz/app", "release/2.0", "aaa", 10, S)]);
        app.pipe_group = crate::app::PipeGroup::Off;
        let out = render_to_string(&mut app, 150, 16);
        assert!(out.contains("Branch"), "the column is there");
        assert!(out.contains("release/2.0"), "and the run's branch is in it");
        assert!(out.contains("Run ") || out.contains("Run\n"), "one run's number, so the heading is singular");
    }

    /// The repository reads first, immediately left of the pipeline/branch column — it was
    /// sitting out past Started, the far edge of the row, which is the last place you look
    /// for the one field that says where a run happened.
    #[test]
    fn the_repository_column_leads_the_row() {
        use PipelineRunStatus::Succeeded as S;
        let mut app = pipe_list(&[
            ("CI", "nz/app", "main", "aaa", 10, S),
            ("Integration", "nz/other", "v2.0", "bbb", 20, S),
        ]);

        // The whole screen comes back as one string, so the heading row is picked out by
        // splitting on the pane's own border — "Pipelines" in the tab bar is not a column.
        let heading = |out: &str| -> String {
            out.split('\u{2502}').find(|seg| seg.contains("Repository")).unwrap_or_default().to_string()
        };

        for group in [crate::app::PipeGroup::Pipeline, crate::app::PipeGroup::Trigger, crate::app::PipeGroup::Off] {
            app.pipe_group = group;
            let out = render_to_string(&mut app, 150, 16);
            let row = heading(&out);
            let repo = row.find("Repository").expect("the column is on screen");
            let subject = row.find(app.pipe_subject_heading(false)).expect("the subject column is on screen");
            assert!(repo < subject, "repository leads the subject column ({group:?}): {row:?}");
            assert!(repo < row.find("Started").expect("Started is on screen"), "and the timings ({group:?})");
        }
    }

    /// A group header is a row of the list, not chrome. Accent is what the borders, the pane
    /// titles and the live tab are painted in, so a Pipelines table using it for the subject
    /// read as a different application next to the Title column on the other two tabs.
    #[test]
    fn a_group_header_is_the_same_colour_as_every_other_list_title() {
        use crate::app::PipeHead;
        let theme = Theme::by_name("slate");
        let head = PipeHead {
            latest: 0,
            key: "k".into(),
            subject: "CI".into(),
            repo: "nz/app".into(),
            commit: String::new(),
            runs: 2,
            failed: 0,
            status: PipelineRunStatus::Succeeded,
            started: Some(Utc::now()),
            approval: false,
            expanded: false,
            provider: ProviderType::GitHub,
            connection: "GH".into(),
        };

        let (text, style) = head_cell(PipeCol::Subject, &head, &theme, 0, None);
        assert_eq!(text, "CI");
        assert_eq!(style.fg, Some(theme.fg), "plain foreground, as Pull Requests and Work Items title their rows");
        assert_ne!(style.fg, Some(theme.accent), "accent is chrome, not row content");
        assert!(style.add_modifier.contains(Modifier::BOLD), "bold is what still sets a roll-up apart from its runs");
    }

    /// Approval is only ever in the set because something needs attention, so it must not be
    /// the column that falls off a narrow pane.
    #[test]
    fn a_narrow_pane_drops_context_columns_before_the_waiting_gate() {
        use PipelineRunStatus::Succeeded as S;
        let mut app = pipe_list(&[
            ("Integration", "magna-nz/forgetop", "claude/changelog-release-notes", "aaa", 10, S),
            ("CI", "other-org/forgetop", "main", "bbb", 20, S),
        ]);
        app.pipes[0].awaiting_approval = true;
        app.pipes[1].provider = ProviderType::GitLab;
        app.pipes[1].connection = "GL".into();
        app.pipe_group = crate::app::PipeGroup::Trigger;

        let wide = render_to_string(&mut app, 150, 16);
        assert!(wide.contains("Repository") && wide.contains("approval needed"), "everything fits");

        let narrow = render_to_string(&mut app, 74, 16);
        assert!(narrow.contains("approval needed"), "the gate survives the squeeze");
        assert!(!narrow.contains("Repository"), "context gives way first");
    }

    /// A group is titled by its definition, never by one member's run name — on Azure that is
    /// a build number, so the title would change with the sort.
    #[test]
    fn a_group_title_does_not_depend_on_which_run_comes_first() {
        use PipelineRunStatus::Succeeded as S;
        let mut app = pipe_list(&[
            ("build", "nz/app", "main", "aaa", 10, S),
            ("build", "nz/app", "main", "bbb", 20, S),
            ("build", "nz/app", "main", "ccc", 30, S),
        ]);
        // Discovery failed, so no definition name; Azure fills run.name with a build number.
        for (n, p) in app.pipes.iter_mut().enumerate() {
            p.definition_name = None;
            p.run.name = Some(format!("20260924.{}", n + 1));
        }
        let title = |app: &App| match &app.pipe_lines()[0] {
            crate::app::PipeLine::Head(h) => h.subject.clone(),
            crate::app::PipeLine::Run(_) => panic!("header"),
        };
        let before = title(&app);
        app.pipes.reverse();
        assert_eq!(title(&app), before, "the title is stable under reordering");
        assert!(!before.starts_with("20260924"), "and is not a build number: {before}");
        assert_eq!(before, "build", "it is the definition the group is keyed on");
    }

    #[test]
    fn pipelines_footer_lists_trigger_without_navigation_hints() {
        let mut app = App::new("slate");
        app.screen = Screen::List;
        app.active = 2;
        let out = render_to_string(&mut app, 120, 24);
        assert!(out.contains("trigger"), "pipelines footer");
        assert!(!out.contains("drill-in") && !out.contains("sections"), "navigation is left to ? and Ctrl-K");
    }

    #[test]
    fn footer_leads_with_the_palette_and_leaves_the_rest_to_it() {
        let mut app = App::new("slate");
        app.screen = Screen::List;
        app.dashboard_url = Some("http://127.0.0.1:8177/?t=x".into());
        let out = render_to_string(&mut app, 160, 24);
        assert!(out.contains("Ctrl-K") && out.contains("search anywhere"), "footer advertises the palette");
        assert!(out.contains(" B ") && out.contains("dashboard"), "footer advertises the running dashboard");
        for gone in ["browser dashboard", "feedback", "save view", "find", "tabs", "sections"] {
            assert!(!out.contains(gone), "{gone} is found through the palette and ? help, not the footer");
        }
    }

    #[test]
    fn help_describes_feedback_as_a_github_issue_form() {
        let global = help_sections()
            .into_iter()
            .find(|(section, _)| *section == "Global")
            .expect("global help section")
            .1;
        assert_eq!(
            global.iter().find(|(key, _)| *key == "F"),
            Some(&("F", "Give feedback through the GitHub issue form"))
        );
    }

    #[test]
    fn footer_hides_dashboard_shortcuts_while_input_is_captured() {
        let mut app = App::new("slate");
        app.screen = Screen::List;
        app.dashboard_url = Some("http://127.0.0.1:8177/?t=x".into());
        app.filtering = true;

        let out = render_to_string(&mut app, 120, 24);
        assert!(!out.contains("dashboard"));
        assert!(!out.contains("palette"));
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


    /// The title column is elastic and would otherwise absorb every spare column, leaving the age
    /// pressed against the pane border.
    #[test]
    fn launchpad_rows_keep_a_gutter_before_the_pane_border() {
        use crate::launchpad::{Bucket, EntryItem};
        let mut app = App::new("slate");
        let mut pr = sample_pr();
        pr.repository = Some("magna-nz/forgetop".into());
        pr.title = "a title long enough that the flexible column wants every spare column".into();
        pr.updated_at = Some(Utc::now() - chrono::Duration::hours(4));
        app.lp = vec![lp_entry(Bucket::NeedsFixing, EntryItem::Pr(pr))];

        // The gutter holds across widths, since it is reserved before the title column is sized
        // rather than being whatever happens to be left over. (Below roughly 130 the fixed
        // columns alone overflow a half-width pane and the row truncates regardless — that is
        // pre-existing, and not what this pins.)
        for width in [140usize, 170, 200] {
            let out = render_to_string(&mut app, width as u16, 12);
            let chars: Vec<char> = out.chars().collect();
            let row: String = chars.chunks(width).nth(5).expect("row").iter().collect();
            let left: String = row.chars().skip(1).take_while(|&c| c != '│').collect();
            assert!(left.contains("4h"), "the row reaches its age column at width {width}");
            assert_eq!(left.len() - left.trim_end().len(), LP_TRAIL, "gutter kept at width {width}");
        }
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
            p.repository = Some("acme/payments".into());
            p
        };
        let run = {
            let mut r = sample_run();
            r.name = Some("nightly".into());
            r.repository = Some("acme/payments".into());
            r
        };
        let wi = WorkItem {
            repository: None,
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
        let out = render_to_string(&mut app, 170, 24);
        // Two named columns.
        assert!(out.contains("Needs you") && out.contains("Your work"), "two columns");
        // Buckets land in the right columns.
        assert!(out.contains("Needs your review") && out.contains("Needs fixing") && out.contains("Assigned to you"), "bucket headers");
        // Every row carries a type badge …
        assert!(out.contains("PR") && out.contains("CI") && out.contains("WI"), "type badges for each kind");
        // … and nav-style detail: PR number + change stats, WI id/state/type, pipeline branch.
        assert!(out.contains("#42") && out.contains("+10 -2"), "PR row shows number and change stats");
        assert!(out.contains("FOR-1") && out.contains("Todo") && out.contains("Bug"), "work-item row shows id, state, type");
        assert!(out.contains("CI Build") && out.contains("⑂ main"), "pipeline row shows pipeline name + branch");
        // The person column shows the PR author (not the provider).
        assert!(out.contains("Alice Ng"), "PR row shows the author");
    }

    /// One connection now spans an account, so the row has to say which repository it came from —
    /// the same reason the PR list grew a Repository column. The owner is dropped: it repeats.
    #[test]
    fn launchpad_rows_name_the_repository_they_came_from() {
        let mut app = App::new("slate");
        let pr = {
            let mut p = sample_pr();
            p.repository = Some("acme/payments".into());
            p
        };
        let run = {
            let mut r = sample_run();
            r.repository = Some("acme/billing".into());
            r.title = Some("Bump axum to 0.8".into());
            r
        };
        app.lp = vec![
            lp_entry(crate::launchpad::Bucket::NeedsReview, crate::launchpad::EntryItem::Pr(pr)),
            lp_entry(
                crate::launchpad::Bucket::NeedsFixing,
                crate::launchpad::EntryItem::Pipe { run, definition_name: Some("CI Build".into()) },
            ),
        ];
        let out = render_to_string(&mut app, 140, 24);
        assert!(out.contains("payments #42"), "PR row pairs the repository with its number");
        assert!(out.contains("billing · CI Build"), "pipeline row pairs the repository with its workflow");
        assert!(!out.contains("acme/"), "the owner repeats on every row, so it is dropped");
    }

    /// The run title (GitHub's `display_title`) says what was *built*; the workflow name moves
    /// beside the repository. Providers that expose no run title fall back to the workflow name,
    /// and it is then shown once rather than in both slots.
    #[test]
    fn pipeline_rows_title_the_commit_not_the_workflow() {
        let render = |title: Option<&str>| {
            let mut app = App::new("slate");
            let mut run = sample_run();
            run.repository = Some("acme/billing".into());
            run.name = None;
            run.title = title.map(Into::into);
            app.lp = vec![lp_entry(
                crate::launchpad::Bucket::RecentPipelines,
                crate::launchpad::EntryItem::Pipe { run, definition_name: Some("Integration".into()) },
            )];
            render_to_string(&mut app, 200, 24)
        };

        let out = render(Some("Bump axum to 0.8"));
        assert!(out.contains("Bump axum to 0.8"), "the run's own title is the row title");
        assert!(out.contains("billing · Integration"), "the workflow sits beside the repository");

        // GitLab and Azure DevOps expose no per-run title: the workflow becomes the title, and
        // must not also appear as its own qualifier (the "Integration | Integration" wart).
        let out = render(None);
        assert!(out.contains("Integration"), "falls back to the workflow name for the title");
        assert!(!out.contains("Integration · Integration") && !out.contains("· Integration"), "never shown twice");
    }

    /// "Needs fixing" asserts something the row has to be able to evidence. A conflict or a
    /// changes-requested review is named in words; failing checks read as the red roll-up.
    #[test]
    fn needs_fixing_rows_say_what_is_blocking_them() {
        let render = |checks, mergeable, votes: &[ReviewVote]| {
            let mut app = App::new("slate");
            let mut pr = sample_pr();
            pr.checks = checks;
            pr.mergeable = mergeable;
            pr.check_summary = Some(CheckSummary { successful: 2, failed: 3, in_progress: 0, neutral: 0 });
            pr.reviewers = votes
                .iter()
                .map(|&vote| Reviewer { user: User { id: "r".into(), display_name: "Rae".into(), handle: None, avatar_url: None }, vote, is_required: false })
                .collect();
            app.lp = vec![lp_entry(crate::launchpad::Bucket::NeedsFixing, crate::launchpad::EntryItem::Pr(pr))];
            render_to_string(&mut app, 140, 24)
        };

        assert!(render(CheckStatus::Passed, MergeableState::Conflicting, &[]).contains("⚠ conflicts"), "a conflict is named");
        assert!(
            render(CheckStatus::Passed, MergeableState::Mergeable, &[ReviewVote::Rejected]).contains("⚠ changes"),
            "a changes-requested review is named"
        );
        // Red checks are already legible as the roll-up, so that case keeps the counts + diffstat.
        let out = render(CheckStatus::Failed, MergeableState::Mergeable, &[]);
        assert!(out.contains("✗ 2/5") && out.contains("+10 -2"), "failing checks show the roll-up and the size");
        // A conflict outranks the rest, so only one reason is ever shown.
        let both = render(CheckStatus::Failed, MergeableState::Conflicting, &[ReviewVote::Rejected]);
        assert!(both.contains("⚠ conflicts") && !both.contains("⚠ changes") && !both.contains("✗ 2/5"), "one reason wins");
    }

    /// A merged pull request keeps whatever `mergeable` and reviewer votes it had when it was
    /// open, so asking what blocks it gives a stale answer. Nothing blocks a merged PR — the row
    /// must show its size, never a blocker.
    #[test]
    fn merged_and_closed_rows_never_claim_to_be_blocked() {
        let render = |status| {
            let mut app = App::new("slate");
            let mut pr = sample_pr();
            pr.status = status;
            pr.repository = Some("acme/payments".into());
            // Exactly the state that would read as blocked while the PR was still open.
            pr.mergeable = MergeableState::Conflicting;
            pr.checks = CheckStatus::Failed;
            pr.reviewers = vec![Reviewer {
                user: User { id: "r".into(), display_name: "Rae".into(), handle: None, avatar_url: None },
                vote: ReviewVote::Rejected,
                is_required: false,
            }];
            app.lp = vec![lp_entry(crate::launchpad::Bucket::RecentlyMerged, crate::launchpad::EntryItem::Pr(pr))];
            render_to_string(&mut app, 140, 24)
        };

        for status in [PullRequestStatus::Merged, PullRequestStatus::Closed] {
            let out = render(status);
            assert!(out.contains("+10 -2"), "{status:?} row shows its size");
            assert!(!out.contains("⚠"), "{status:?} row claims no blocker");
            assert!(!out.contains("✗ 2/5") && !out.contains("· —"), "{status:?} row shows no check roll-up");
        }
    }

    /// Every row in the right-hand column is yours by construction, so the author name would just
    /// repeat down the pane. The width goes to the title instead.
    #[test]
    fn your_work_column_drops_the_author() {
        let mut app = App::new("slate");
        let pr = {
            let mut p = sample_pr();
            p.repository = Some("acme/payments".into());
            p
        };
        app.lp = vec![lp_entry(crate::launchpad::Bucket::YourOpenPrs, crate::launchpad::EntryItem::Pr(pr.clone()))];
        assert!(!render_to_string(&mut app, 140, 24).contains("Alice Ng"), "no author in Your work");

        // The left column still names who is asking something of you.
        let mut app = App::new("slate");
        app.lp = vec![lp_entry(crate::launchpad::Bucket::NeedsReview, crate::launchpad::EntryItem::Pr(pr))];
        assert!(render_to_string(&mut app, 140, 24).contains("Alice Ng"), "author kept in Needs you");
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
            filter: None,
        });
        let out = render_to_string(&mut app, 100, 24);
        assert!(out.contains("Visible tabs"), "toggle title");
        assert!(out.contains("▶"), "green arrow marks a visible section");
        assert!(out.contains("toggle"), "toggle footer hint");
    }

    #[test]
    fn a_frame_is_drawable_before_any_data_arrives() {
        // The regression this guards: startup used to await the whole fetch before the first
        // draw, so the terminal sat blank for as long as the network took. An app with no
        // data must still render, and must say it is working.
        let mut app = App::new("slate");
        app.loading = true;
        app.reloading = true;
        let out = render_to_string(&mut app, 100, 24);

        assert!(out.contains("forgetop"), "the chrome must be drawn with no data: {out}");
        assert!(out.contains("Refreshing"), "the user must be told a fetch is in flight: {out}");
    }

    #[test]
    fn awaiting_setup_card_tells_you_where_to_go_and_how_to_back_out() {
        let mut app = App::new("slate");
        app.awaiting_browser_setup = true;
        app.dashboard_url = Some("http://127.0.0.1:8177/?t=abc".into());
        let out = render_to_string(&mut app, 100, 30);

        assert!(out.contains("Setting up in your browser"), "{out}");
        assert!(out.contains("access tokens"), "{out}");
        // The address must be the settings route, with the token query preserved.
        assert!(out.contains("#settings"), "{out}");
        // Both ways out are offered: do it here instead, or reopen the browser.
        assert!(out.contains("set up here instead"), "{out}");
        assert!(out.contains("reopen the dashboard"), "{out}");
    }

    #[test]
    fn awaiting_setup_card_says_how_to_start_the_dashboard_when_it_is_not_running() {
        let mut app = App::new("slate");
        app.awaiting_browser_setup = true;
        app.dashboard_url = None;
        let out = render_to_string(&mut app, 100, 30);
        assert!(out.contains("forgetop --dashboard"), "{out}");
    }

    #[test]
    fn the_wizard_outranks_the_awaiting_card() {
        // Picking "set up here instead" while waiting must not leave the card on top.
        let mut app = App::new("slate");
        app.awaiting_browser_setup = true;
        app.wizard = Some(crate::wizard::Wizard::new());
        let out = render_to_string(&mut app, 100, 30);
        assert!(out.contains("Add connection"), "{out}");
        assert!(!out.contains("Setting up in your browser"), "{out}");
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
    fn the_bind_step_renders_a_checklist_with_everything_ticked() {
        let mut app = App::new("slate");
        let mut w = crate::wizard::Wizard::new();
        // Walk to the bind step: provider, display name, repository, token.
        w.handle(crate::app::Key::Enter);
        w.handle(crate::app::Key::Enter);
        w.handle(crate::app::Key::Enter);
        for c in "ghp_x".chars() {
            w.handle(crate::app::Key::Char(c));
        }
        w.handle(crate::app::Key::Enter);
        app.wizard = Some(w);

        let out = render_to_string(&mut app, 100, 30);
        assert!(out.contains("Sections to populate"), "{out}");
        assert!(out.contains("[x] Pull Requests"), "rows start ticked: {out}");
        // The old single-choice affordance is gone; the help line explains the replacement.
        assert!(!out.contains("Don't bind now"), "{out}");
        assert!(out.contains("nothing ticked skips"), "{out}");
        assert!(out.contains("space"), "the hint must name the toggle key: {out}");
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
    fn work_item_description_renders_as_readable_lines_not_markup() {
        // The string here is exactly what `azure::map_work_item` produces for an Azure work item
        // (see `azure::tests::flattens_the_html_description_azure_returns`) — issue #162 was this
        // pane showing the raw `<div>`/`<br>` markup instead, on one unbroken line.
        let mut app = App::new("slate");
        app.screen = Screen::WiView(Box::new(crate::app::WiView {
            timeline: Vec::new(),
            connection_id: "azure".into(),
            wi: WorkItem {
                repository: Some("Payments".into()),
                id: "4821".into(),
                identifier: Some("4821".into()),
                title: "Refresh token dropped after 60 minutes".into(),
                description: Some("The refresh token is dropped after 60 minutes.\n\nRepro steps\n- Sign in\n- Wait > 1 hour".into()),
                state: "Active".into(),
                state_category: WorkItemStateCategory::Started,
                work_item_type: Some("Bug".into()),
                assignee: None,
                created_at: None,
                updated_at: None,
                url: None,
            },
            threads: vec![],
            scroll: 0,
        }));
        let view = render_to_string(&mut app, 140, 30);
        for tag in ["<div", "<br", "<span", "&nbsp;", "&gt;", "&amp;"] {
            assert!(!view.contains(tag), "markup {tag} leaked into the work item pane");
        }
        // Each line of the description lands on its own row, rather than running together.
        assert!(view.contains("The refresh token is dropped after 60 minutes."), "first line renders");
        assert!(view.contains("Repro steps"), "heading renders on its own line");
        assert!(view.contains("- Sign in"), "list item renders");
        assert!(view.contains("- Wait > 1 hour"), "a decoded `&gt;` renders as the character");
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
            timeline: Vec::new(),
            connection_id: "c".into(),
            wi: WorkItem {
                repository: None,
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
    fn a_refresh_over_seeded_rows_says_how_old_they_are() {
        let mut app = App::new("slate");
        app.reloading = true;
        app.data_age = Some(Utc::now() - chrono::Duration::minutes(15));
        let out = render_to_string(&mut app, 120, 24);
        assert!(out.contains("showing 15m old"), "a refresh over cached rows reports their age");
    }

    /// Sub-minute ages must not read "showing now old".
    #[test]
    fn a_just_cached_age_reads_as_prose() {
        let mut app = App::new("slate");
        app.reloading = true;
        app.data_age = Some(Utc::now());
        let out = render_to_string(&mut app, 120, 24);
        assert!(out.contains("showing cached"), "sub-minute ages get their own wording");
        assert!(!out.contains("now old"), "never 'showing now old'");
    }

    /// The indicator is a statement about cache-seeded rows, so it must vanish the moment
    /// live data lands — otherwise it keeps claiming staleness that no longer exists.
    #[test]
    fn the_age_indicator_disappears_once_live_data_lands() {
        let mut app = App::new("slate");
        app.reloading = true;
        app.data_age = None;
        let out = render_to_string(&mut app, 120, 24);
        assert!(out.contains("Refreshing"), "still refreshing");
        assert!(!out.contains("showing"), "no age once data_age is cleared");
    }

    /// Its home is the tab row, immediately left of Notifications.
    #[test]
    fn refreshing_sits_next_to_notifications_in_the_tab_row() {
        let mut app = App::new("slate");
        app.status = "9 PRs · 10 work items · 8 runs".into();
        app.reloading = true;
        let rows = render_to_rows(&mut app, 160, 24);

        let tab_row = &rows[1];
        let r = tab_row.find("Refreshing").expect("'Refreshing…' is on the tab row");
        let n = tab_row.find("Notifications").expect("Notifications is on the tab row");
        assert!(r < n, "it sits to the left of Notifications: {tab_row}");
        // The footer gives its right side back to the standing status line.
        let footer = rows.last().expect("the footer");
        assert!(footer.contains("9 PRs"), "the footer shows the status again: {footer}");
        assert!(!footer.contains("Refreshing"), "not in the footer as well: {footer}");
        // No refresh glyph anywhere — the animated word is the whole indicator.
        assert!(!rows.concat().contains("⟳"), "no spinner");
    }

    /// A terminal too narrow for tabs + Notifications + the indicator keeps it in the
    /// footer rather than overprinting the tabs.
    #[test]
    fn a_narrow_terminal_keeps_refreshing_in_the_footer() {
        let mut app = App::new("slate");
        app.reloading = true;
        let rows = render_to_rows(&mut app, 100, 24);

        let tab_row = &rows[1];
        assert!(!tab_row.contains("Refreshing"), "no room up there: {tab_row}");
        let footer = rows.last().expect("the footer");
        assert!(footer.contains("Refreshing"), "so the footer carries it: {footer}");
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
    fn patch_line_styles_markup_kinds_with_modifiers() {
        use crate::highlight::{Lang, LineHighlighter};
        let theme = Theme::by_name("slate");
        let mut hl = LineHighlighter::new(Lang::Markdown).unwrap();
        let line = patch_line_hl(&theme, "+# Title", Some(&mut hl));

        // The add marker still keeps its own green, unbolded.
        assert_eq!(line.spans[0].content, "+");
        assert!(!line.spans[0].style.add_modifier.contains(Modifier::BOLD));
        // The heading itself is magenta *and* bold — the modifier a plain colour can't carry.
        let head = line.spans.iter().find(|s| s.content.contains("Title")).expect("a heading span");
        assert_eq!(head.style.fg, Some(theme.magenta));
        assert!(head.style.add_modifier.contains(Modifier::BOLD), "heading is bold");
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
        use crate::palette::{self, Group, PaletteItem, PaletteKind, PaletteTarget, Tone};

        let target = |kind, id: &str| PaletteTarget::Item { kind, id: id.into(), connection_id: "c".into() };
        let items = vec![
            PaletteItem {
                tone: Some(Tone::Good),
                ..PaletteItem::new(Group::Item, target(PaletteKind::Pr, "1"), "Add the widget").with_subtitle("alice · GitHub")
            },
            PaletteItem {
                tone: Some(Tone::Bad),
                ..PaletteItem::new(Group::Item, target(PaletteKind::Pipe, "2"), "CI Build").with_subtitle("main")
            },
        ];
        let results = palette::rank("", &items);
        let mut app = App::new("slate");
        app.overlay = Some(Overlay::Palette { query: "a".into(), candidates: items, results, selected: 0 });

        let out = render_to_string(&mut app, 100, 30);
        assert!(out.contains("Search everything"), "panel title");
        assert!(out.contains("Add the widget"), "a result title is shown");
        assert!(out.contains("CI Build"), "results from every type are shown");
        assert!(out.contains("match"), "the match count is shown");
        assert!(out.contains("● "), "status dots are rendered");
        assert!(out.contains("PR") && out.contains("CI"), "type badges are shown");
    }

    #[test]
    fn palette_shows_group_headers_key_badges_and_keeps_the_selection_visible_at_80_columns() {
        use crate::overlay::Overlay;
        use crate::palette::{self, GoTo, Group, PaletteItem, PaletteTarget};

        let mut items = vec![PaletteItem::new(Group::Action, PaletteTarget::Key(crate::app::Key::Char('a')), "Approve")
            .with_subtitle("PR view")
            .with_key("a")];
        // Enough go-to rows that the last one sits beyond the window.
        for i in 0..30 {
            items.push(PaletteItem::new(Group::GoTo, PaletteTarget::GoTo(GoTo::Help), format!("Destination {i}")));
        }
        let results = palette::rank("", &items);
        let last = results.len() - 1;
        let mut app = App::new("slate");
        app.overlay = Some(Overlay::Palette { query: String::new(), candidates: items.clone(), results: results.clone(), selected: 0 });
        let out = render_to_string(&mut app, 80, 30);
        assert!(out.contains("ACTIONS") && out.contains("GO TO"), "group headers are shown");
        assert!(out.contains(" a "), "the key badge is shown");
        assert!(out.contains("> actions  : commands  @ people  # ids  ? keys"), "the prefix legend is shown");
        assert!(out.contains("^K"), "the close hint is shown");

        app.overlay = Some(Overlay::Palette { query: String::new(), candidates: items, results, selected: last });
        let out = render_to_string(&mut app, 80, 30);
        assert!(out.contains("Destination 29"), "the selected row is scrolled into view");
        assert!(!out.contains("Destination 0 "), "rows above the window are dropped");
    }

    fn notif_row(id: &str, kind: NotificationKind, title: &str, unread: bool) -> crate::app::NotifRow {
        crate::app::NotifRow {
            connection_id: "github".into(),
            connection: "GitHub".into(),
            provider: ProviderType::GitHub,
            notification: Notification {
                repository: None,
                id: id.into(),
                kind,
                item_type: NotificationItemType::PullRequest,
                item_id: Some("1".into()),
                title: title.into(),
                context: "northwind/payments".into(),
                url: Some("http://x".into()),
                unread,
                updated_at: None,
            },
        }
    }

    #[test]
    fn inbox_lists_notifications_and_header_shows_unread_count() {
        use crate::app::Screen;
        let mut app = App::new("slate");
        app.inbox = vec![
            notif_row("n1", NotificationKind::ReviewRequested, "Refactor the retry queue", true),
            notif_row("n2", NotificationKind::CiFailed, "Bump the deps", false),
        ];
        app.screen = Screen::Inbox;
        let out = render_to_string(&mut app, 130, 24);
        assert!(out.contains("Refactor the retry queue"), "notification title renders");
        assert!(out.contains("review"), "kind label renders");
        assert!(out.contains("Inbox"), "inbox panel title");
        assert!(out.contains("Notifications (1)"), "far-right nav item shows the unread count");
    }

    #[test]
    fn header_indicator_reads_zero_when_inbox_empty() {
        let mut app = App::new("slate");
        let out = render_to_string(&mut app, 120, 24);
        assert!(out.contains("Notifications (0)"), "grey (0) nav item when there's nothing");
    }

    #[test]
    fn help_shows_sections_from_both_columns_at_once() {
        use crate::overlay::Overlay;
        let mut app = App::new("slate");
        app.overlay = Some(Overlay::Help { scroll: 0 });
        let out = render_to_string(&mut app, 120, 44);
        assert!(out.contains("Keybindings"), "help panel title");
        // Early and late sections both visible on one screen → the two-column layout works.
        assert!(out.contains("Global"), "first section");
        assert!(out.contains("Pipelines"), "a later section without scrolling");
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

    #[test]
    fn shed_columns_leaves_a_wide_list_alone() {
        let headers = ["", "Provider", "Title"];
        let cells = vec![vec![("●".into(), Style::default()), ("GitHub".into(), Style::default()), ("A title".into(), Style::default())]];
        let (h, c, sort) = shed_columns(&headers, cells, 2, Some((1, false)), 200, &[1]);
        assert_eq!(h, headers.to_vec());
        assert_eq!(c[0].len(), 3);
        assert_eq!(sort, Some((1, false)));
    }

    #[test]
    fn shed_columns_drops_in_order_until_the_title_fits_and_moves_the_sort_arrow() {
        let headers = ["", "Provider", "Repository", "Title", "Updated"];
        let long = "x".repeat(40);
        let cells = vec![vec![
            ("●".into(), Style::default()),
            ("GitHub Enterprise".into(), Style::default()),
            ("payments".into(), Style::default()),
            (long, Style::default()),
            ("3h".into(), Style::default()),
        ]];
        // Room for the title only once Provider is gone.
        let (h, c, sort) = shed_columns(&headers, cells, 3, Some((4, true)), 62, &[1, 4, 2]);
        assert_eq!(h, vec!["", "Repository", "Title", "Updated"], "Provider shed first, and only it");
        assert_eq!(c[0].len(), 4);
        assert_eq!(sort, Some((3, true)), "the arrow follows Updated to its new index");

        let cells = vec![vec![("●".into(), Style::default()); 5]];
        let (_, _, sort) = shed_columns(&headers, cells, 3, Some((1, false)), 10, &[1]);
        assert_eq!(sort, None, "a shed column takes its arrow with it");
    }

    #[test]
    fn pr_sort_arrow_sits_on_the_sorted_column() {
        // The list gained a Repository column at index 2; the key → column table has to follow.
        for (key, header) in [("number", "#"), ("title", "Title"), ("author", "Author"), ("checks", "State"), ("updated", "Updated")] {
            let headers = ["", "Provider", "Repository", "#", "Title", "Author", "State", "±", "Updated"];
            assert_eq!(sort_header_col(0, key).map(|i| headers[i]), Some(header), "sort key {key}");
        }
    }

    #[test]
    fn an_open_prs_write_actions_get_yellow_chips_and_navigation_stays_blue() {
        let mut app = App::new("slate");
        app.screen = Screen::PrView(Box::new(pr_view(0, vec![], vec![])));
        let (w, h) = (200, 24);
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let footer: String = (0..w).map(|x| buf[(x, h - 1)].symbol().to_string()).collect();
        let chip_bg = |label: &str| {
            let at = footer.find(&format!("  {label}")).unwrap_or_else(|| panic!("footer has {label}: {footer}"));
            let x = footer[..at].chars().count() as u16 - 2; // the key letter inside its chip
            buf[(x, h - 1)].bg
        };
        let theme = Theme::by_name("slate");
        for label in ["approve", "reject", "merge", "comment"] {
            assert_eq!(chip_bg(label), theme.yellow, "{label} is a write action");
        }
        assert_eq!(chip_bg("tabs"), theme.accent, "navigation keeps the blue chip");
    }

    fn event(who: &str, kind: TimelineEventKind, summary: &str, hours_ago: i64) -> TimelineEvent {
        TimelineEvent {
            actor: Some(User { id: who.into(), display_name: who.into(), handle: None, avatar_url: None }),
            kind,
            summary: summary.into(),
            at: Some(Utc::now() - chrono::Duration::hours(hours_ago)),
        }
    }

    #[test]
    fn activity_reads_each_providers_summary_as_a_verb_and_a_target() {
        use TimelineEventKind as K;
        let parts = |k, s: &str| activity_parts(k, s);
        assert_eq!(parts(K::StateChanged, "changed status to In Progress"), ("state".into(), Some("In Progress".into())));
        // A state whose own name contains " to " keeps it whole.
        assert_eq!(parts(K::StateChanged, "changed status to Ready to Deploy"), ("state".into(), Some("Ready to Deploy".into())));
        assert_eq!(parts(K::Assigned, "assigned this to Sam Rivera"), ("assigned".into(), Some("Sam Rivera".into())));
        assert_eq!(parts(K::Assigned, "unassigned this"), ("unassigned".into(), None));
        assert_eq!(parts(K::Labeled, "added the bug label"), ("labeled".into(), Some("bug".into())));
        assert_eq!(parts(K::Other, "created this"), ("created".into(), None));
        assert_eq!(parts(K::Other, "opened this pull request"), ("opened".into(), None));
        assert_eq!(parts(K::Approved, "approved these changes"), ("approved".into(), None));
        assert_eq!(parts(K::Merged, "completed this pull request"), ("completed".into(), None));
        assert_eq!(parts(K::Closed, "declined this"), ("declined".into(), None));
        assert_eq!(parts(K::Other, "set resolution to Won't Do"), ("resolution".into(), Some("Won't Do".into())));
        assert_eq!(parts(K::StateChanged, "changed status to "), ("state".into(), None));
        // Azure's "waiting for author" vote keeps its words rather than a generic "reviewed".
        assert_eq!(parts(K::Reviewed, "is waiting for the author"), ("is waiting for the author".into(), None));
    }

    #[test]
    fn the_work_item_view_lists_its_activity_under_the_comments() {
        use crate::app::Screen;
        let mut app = App::new("slate");
        app.screen = Screen::WiView(Box::new(crate::app::WiView {
            connection_id: "c".into(),
            wi: WorkItem {
                repository: None,
                id: "77".into(),
                identifier: Some("#77".into()),
                title: "Right-size the staging cluster".into(),
                description: None,
                state: "In Progress".into(),
                state_category: WorkItemStateCategory::Started,
                work_item_type: Some("Task".into()),
                assignee: None,
                created_at: None,
                updated_at: None,
                url: None,
            },
            threads: vec![],
            timeline: vec![
                event("Sam Rivera", TimelineEventKind::Other, "created this", 48),
                event("Sam Rivera", TimelineEventKind::StateChanged, "changed status to In Progress", 24),
                event("Priya Nair", TimelineEventKind::Assigned, "assigned this to Sam Rivera", 5),
            ],
            scroll: 0,
        }));
        let rows = render_to_rows(&mut app, 120, 30);
        let find = |needle: &str| rows.iter().position(|r| r.contains(needle)).unwrap_or_else(|| panic!("{needle:?} is on screen"));
        assert!(rows[find("Comments (0)")].contains("No comments."), "an empty thread is one line");
        assert!(find("Activity") > find("Comments (0)"), "activity sits under the comments");
        assert!(rows[find("created")].contains("2d") && rows[find("created")].contains("Sam Rivera"));
        assert!(rows[find("state")].contains("→ In Progress"), "a state change names where it went");
        assert!(rows[find("assigned")].contains("Priya Nair") && rows[find("assigned")].contains("→ Sam Rivera"));
        // Actors and verbs line up in columns.
        let col = |needle: &str, of: &str| rows[find(needle)].find(of).unwrap();
        assert_eq!(col("state", "→"), col("assigned", "→"), "targets start in one column");
    }

    #[test]
    fn the_pr_conversation_ends_with_activity_and_hides_it_when_there_is_none() {
        use crate::app::Screen;
        let mut app = App::new("slate");
        app.screen = Screen::PrView(Box::new(pr_view(0, vec![], vec![])));
        assert!(!render_to_string(&mut app, 120, 40).contains("Activity"), "no timeline, no heading");
        if let Screen::PrView(v) = &mut app.screen {
            v.timeline = vec![event("Marcus Lee", TimelineEventKind::Approved, "approved these changes", 2)];
        }
        let rows = render_to_rows(&mut app, 120, 40);
        let activity = rows.iter().position(|r| r.contains("Activity")).expect("Activity heading");
        let comments = rows.iter().position(|r| r.contains("Comments (")).expect("Comments heading");
        assert!(activity > comments, "activity follows the comments");
        assert!(rows[activity + 1].contains("Marcus Lee") && rows[activity + 1].contains("approved"));
    }

    #[test]
    fn scrolling_to_the_end_reaches_the_activity_even_when_lines_wrap() {
        use crate::app::Screen;
        let mut app = App::new("slate");
        app.screen = Screen::PrView(Box::new(pr_view(0, vec![], vec![])));
        if let Screen::PrView(v) = &mut app.screen {
            // One paragraph that wraps over many rows at this width.
            v.pr.description = Some("word ".repeat(400));
            v.timeline = vec![event("Marcus Lee", TimelineEventKind::Merged, "merged this", 1)];
            v.scroll = u16::MAX; // "as far as it goes"
        }
        let out = render_to_string(&mut app, 80, 30);
        assert!(out.contains("Marcus Lee") && out.contains("merged"), "the last activity row is reachable");
    }

    #[test]
    fn an_open_work_items_footer_keeps_every_key_over_the_counts() {
        use crate::app::Screen;
        let mut app = App::new("slate");
        app.status = "9 PRs · 10 work items · 8 runs".into();
        app.screen = Screen::WiView(Box::new(crate::app::WiView {
            connection_id: "c".into(),
            wi: WorkItem {
                repository: None,
                id: "77".into(),
                identifier: Some("#77".into()),
                title: "t".into(),
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
            timeline: vec![],
            scroll: 0,
        }));
        let rows = render_to_rows(&mut app, 120, 20);
        let footer = rows.last().unwrap();
        for key in ["update state", "assign", "edit", "comment", "back"] {
            assert!(footer.contains(key), "{key:?} is in the footer: {footer}");
        }
    }
}
