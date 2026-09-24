//! forgetop terminal UI: ratatui + crossterm. We own the input loop (immediate mode),
//! so there are no framework focus fights — every keystroke is dispatched by us.

pub mod app;
pub mod diff;
pub mod highlight;
pub mod launchpad;
pub mod overlay;
pub mod palette;
pub mod theme;
pub mod ui;
pub mod wizard;

use std::io::{self, Stdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use forgetop_core::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;

pub use app::{App, AppDeps, Key};

const REFRESH_SECS: u64 = 30;
/// How often the shared animation frame advances (running spinner; the marquee
/// scroll advances every other frame). ~6.7 fps.
const ANIM_MS: u64 = 150;

type Term = Terminal<CrosstermBackend<Stdout>>;

/// Set while `$EDITOR` owns the terminal, so the input thread stops reading keys meant for it.
static INPUT_PAUSED: AtomicBool = AtomicBool::new(false);

/// Set up the terminal, run the loop against `deps`, and always restore the terminal.
pub async fn run(deps: AppDeps, theme_name: &str, dashboard_url: Option<String>) -> Result<()> {
    install_panic_hook();
    let mut terminal = setup_terminal().map_err(forgetop_core::Error::from)?;

    let mut app = App::new(theme_name);
    app.dashboard_url = dashboard_url;
    app.apply_hidden_sections(&deps.config.snapshot().ui.hidden_sections);
    app.apply_hidden_work_item_states(&deps.config.snapshot().ui.hidden_work_item_states);
    app.apply_preview_hidden(&deps.config.snapshot().ui.preview_hidden);
    app.apply_dismissed_launchpad_items(&deps.config.snapshot().ui.dismissed_launchpad_items);
    {
        let ui = deps.config.snapshot().ui;
        app.apply_sorts(ui.pr_sort, ui.work_item_sort, ui.pipeline_sort);
        app.apply_pipe_group(ui.pipeline_group);
        app.apply_views(ui.pr_views, ui.work_item_views, ui.pipeline_views);
        app.notifications = ui.notifications;
    }

    // First run — nothing configured, or no connection has a token yet. forgetop is a terminal
    // tool first, so ask here rather than silently handing the user to a browser: the picker
    // offers the in-terminal wizard or the dashboard, and `n` reopens it at any time.
    // A configured install goes straight to the terminal UI — the dashboard is never opened on
    // launch, only by `B`.
    let cfg = deps.config.snapshot();
    if cfg.connections.is_empty() || cfg.connections.iter().all(|c| c.credential_ref.is_none()) {
        app.open_setup_picker();
    }

    let result = event_loop(&mut terminal, &mut app, &deps).await;

    restore_terminal(&mut terminal).map_err(forgetop_core::Error::from)?;
    result
}

async fn event_loop(terminal: &mut Term, app: &mut App, deps: &AppDeps) -> Result<()> {
    // A blocking thread reads crossterm events and forwards the ones we care about.
    let (tx, mut rx) = mpsc::unbounded_channel::<Key>();
    std::thread::spawn(move || input_reader(tx));

    let mut ticker = tokio::time::interval(Duration::from_secs(REFRESH_SECS));
    ticker.tick().await; // consume the immediate first tick

    // A fast tick drives animations (the running-pipeline spinner and the selected-row
    // title marquee). Idle frames are identical, so ratatui writes nothing.
    let mut anim = tokio::time::interval(Duration::from_millis(ANIM_MS));
    anim.tick().await;

    // The first fetch is requested, not awaited. Awaiting it here meant the alternate screen
    // was entered and then nothing was drawn until every provider had answered — a blank
    // terminal for as long as the network took, with no spinner to say it was working.
    // Completed background jobs (refreshes) are delivered here and applied on the loop,
    // so network work never blocks rendering — the header spinner keeps animating.
    let (job_tx, mut job_rx) = mpsc::unbounded_channel::<app::AppEvent>();
    app.job_tx = Some(job_tx);

    // Paint the last run's data before asking the network for this run's. The fetch below still
    // goes out unconditionally — the cache decides what is on screen *while* it runs, never
    // whether it runs — so the only thing this removes is the blank screen, not the refresh.
    deps.cache.load().await;
    app.seed_from_cache(deps);

    app.request_reload(deps);

    // Connections and bindings are managed in the dashboard, which shares this process's
    // ConfigService. Without this the terminal sat on stale data until the next tick — up
    // to REFRESH_SECS of blank screen right after a user finished setup in the browser.
    let mut data_changed = deps.config.subscribe_data_changed();

    loop {
        terminal.draw(|f| ui::render(f, app)).map_err(forgetop_core::Error::from)?;
        if app.should_quit {
            // Writes made off the fetch path — marking notifications read, for one — only reach
            // the in-memory map; the flush below is in the job arm, which a quit need never pass
            // through. Without this, reading a notification and pressing `q` leaves the cache
            // saying unread, and the next launch repaints it that way.
            deps.cache.flush().await;
            break;
        }

        tokio::select! {
            key = rx.recv() => match key {
                Some(key) => {
                    app.on_key(key, deps).await;
                    // A handler asked for `$EDITOR`: hand it the terminal, then the text back.
                    if let Some(request) = app.editor_request.take() {
                        let result = edit_in_editor(terminal, &request.initial).await;
                        app.finish_editor(request, result, deps).await;
                    }
                }
                None => break, // reader thread gone
            },
            _ = ticker.tick() => app.request_reload(deps),
            // `changed()` errors only once every sender is dropped, which cannot happen
            // while `deps` is alive; the branch then disables itself rather than spinning.
            Ok(()) = data_changed.changed() => app.request_reload(deps),
            Some(event) = job_rx.recv() => {
                app.on_event(event, deps);
                // `on_event` has just written the fresh sections through to the cache's in-memory
                // map; persist them here, off the key path, so a crash or a kill -9 still leaves
                // the next launch something to paint. No-ops unless something actually changed.
                deps.cache.flush().await;
            }
            _ = anim.tick() => {
                app.tick_anim();
                app.tick_preview(deps);
                // Polls an open log pane while its job is live; idle otherwise.
                app.tick_logs(deps);
            }
        }
    }
    Ok(())
}

/// Runs on a dedicated thread: blocks on crossterm, maps events to [`Key`], sends them on.
fn input_reader(tx: mpsc::UnboundedSender<Key>) {
    loop {
        if INPUT_PAUSED.load(Ordering::SeqCst) {
            if tx.is_closed() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        // Poll so the thread can notice a closed channel even without input.
        match event::poll(Duration::from_millis(200)) {
            Ok(true) => {}
            Ok(false) => {
                if tx.is_closed() {
                    return;
                }
                continue;
            }
            Err(_) => return,
        }
        let Ok(evt) = event::read() else { return };
        let key = match evt {
            Event::Key(k) if k.kind != KeyEventKind::Release => map_key(k.code, k.modifiers),
            // Wake the loop so it redraws at the new terminal size (fixes zoom/resize).
            Event::Resize(_, _) => Key::Redraw,
            _ => Key::None,
        };
        if key != Key::None && tx.send(key).is_err() {
            return;
        }
    }
}

fn map_key(code: KeyCode, mods: KeyModifiers) -> Key {
    if mods.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char(c) = code {
            let c = c.to_ascii_lowercase();
            return if c == 'c' { Key::Quit } else { Key::Ctrl(c) };
        }
    }
    // Keep keys semantic but preserve raw characters, so the app can interpret them
    // as navigation in normal mode or as literal text while an input overlay is open.
    match code {
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        // Shift-Tab arrives as BackTab on most terminals, but some report Tab + SHIFT.
        KeyCode::BackTab => Key::BackTab,
        KeyCode::Tab if mods.contains(KeyModifiers::SHIFT) => Key::BackTab,
        KeyCode::Tab => Key::Tab,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Escape,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::Char(c) => Key::Char(c),
        _ => Key::None,
    }
}

/// How long the input thread may still be inside one `event::poll` after being paused.
const INPUT_POLL_MS: u64 = 200;

/// Suspends the TUI, opens `initial` in the user's editor, and returns what they saved.
///
/// The input thread is paused first and given longer than one poll to notice, or it would read
/// the keystrokes typed into the editor. The terminal is always put back, whatever the editor did.
async fn edit_in_editor(terminal: &mut Term, initial: &str) -> std::result::Result<String, String> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let path = std::env::temp_dir().join(format!("forgetop-edit-{}-{stamp}.md", std::process::id()));
    // `create_new` refuses a path that already exists (a planted symlink included).
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| e.to_string())?;
        file.write_all(initial.as_bytes()).map_err(|e| e.to_string())?;
    }

    INPUT_PAUSED.store(true, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(INPUT_POLL_MS + 50)).await;
    let _ = restore_terminal(terminal);

    let (program, args) = editor_command();
    let file = path.clone();
    let status = tokio::task::spawn_blocking(move || std::process::Command::new(&program).args(&args).arg(&file).status()).await;

    let resumed = enable_raw_mode().and_then(|()| execute!(io::stdout(), EnterAlternateScreen)).and_then(|()| terminal.clear());
    INPUT_PAUSED.store(false, Ordering::SeqCst);

    let text = std::fs::read_to_string(&path);
    let _ = std::fs::remove_file(&path);
    resumed.map_err(|e| e.to_string())?;
    match status {
        Ok(Ok(s)) if s.success() => text.map_err(|e| e.to_string()),
        Ok(Ok(s)) => Err(format!("the editor exited with {s}")),
        Ok(Err(e)) => Err(format!("couldn't start the editor ({e}) — set $EDITOR")),
        Err(e) => Err(e.to_string()),
    }
}

/// `$VISUAL`, else `$EDITOR`, else the platform's stock editor. Split on whitespace so a value
/// such as `code --wait` works.
fn editor_command() -> (String, Vec<String>) {
    let configured = ["VISUAL", "EDITOR"].iter().find_map(|v| std::env::var(v).ok().filter(|s| !s.trim().is_empty()));
    let fallback = if cfg!(windows) { "notepad" } else { "vi" };
    let line = configured.unwrap_or_else(|| fallback.to_string());
    let mut parts = line.split_whitespace().map(str::to_string);
    let program = parts.next().unwrap_or_else(|| fallback.to_string());
    (program, parts.collect())
}

/// On panic, leave the alternate screen + raw mode so the message is readable (not a
/// garbled terminal), record it to the log file, and point the user at it — then defer to
/// the default hook so the backtrace still prints.
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        forgetop_core::diag::log("panic", &info.to_string());
        eprintln!("\nforgetop crashed — details logged to {}", forgetop_core::diag::log_path().display());
        default(info);
    }));
}

fn setup_terminal() -> io::Result<Term> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(stdout))
}

fn restore_terminal(terminal: &mut Term) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()
}
