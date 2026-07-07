//! forgetop terminal UI: ratatui + crossterm. We own the input loop (immediate mode),
//! so there are no framework focus fights — every keystroke is dispatched by us.

pub mod app;
pub mod overlay;
pub mod theme;
pub mod ui;
pub mod wizard;

use std::io::{self, Stdout};
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

type Term = Terminal<CrosstermBackend<Stdout>>;

/// Set up the terminal, run the loop against `deps`, and always restore the terminal.
pub async fn run(deps: AppDeps, theme_name: &str) -> Result<()> {
    let mut terminal = setup_terminal().map_err(forgetop_core::Error::from)?;

    let mut app = App::new(theme_name);
    app.apply_hidden_sections(&deps.config.snapshot().ui.hidden_sections);
    app.apply_hidden_work_item_states(&deps.config.snapshot().ui.hidden_work_item_states);
    app.reload_all(&deps).await;

    // First run: nothing configured yet — drop straight into the add-connection wizard.
    if deps.config.snapshot().connections.is_empty() {
        app.start_add_connection();
        app.toast = Some("Welcome to forgetop — add your first connection".into());
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

    loop {
        terminal.draw(|f| ui::render(f, app)).map_err(forgetop_core::Error::from)?;
        if app.should_quit {
            break;
        }

        tokio::select! {
            key = rx.recv() => match key {
                Some(key) => app.on_key(key, deps).await,
                None => break, // reader thread gone
            },
            _ = ticker.tick() => app.reload_all(deps).await,
        }
    }
    Ok(())
}

/// Runs on a dedicated thread: blocks on crossterm, maps events to [`Key`], sends them on.
fn input_reader(tx: mpsc::UnboundedSender<Key>) {
    loop {
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
    if mods.contains(KeyModifiers::CONTROL) && matches!(code, KeyCode::Char('c')) {
        return Key::Quit;
    }
    // Keep keys semantic but preserve raw characters, so the app can interpret them
    // as navigation in normal mode or as literal text while an input overlay is open.
    match code {
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Tab => Key::Tab,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Escape,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Char(c) => Key::Char(c),
        _ => Key::None,
    }
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
