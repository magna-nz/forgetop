//! forgetop terminal UI: ratatui + crossterm. We own the input loop (immediate mode),
//! so there are no framework focus fights — every keystroke is dispatched by us.

pub mod app;
pub mod theme;
pub mod ui;

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
    app.reload_all(&deps).await;

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
    match code {
        KeyCode::Up | KeyCode::Char('k') => Key::Up,
        KeyCode::Down | KeyCode::Char('j') => Key::Down,
        KeyCode::Left | KeyCode::Char('h') => Key::Left,
        KeyCode::Right | KeyCode::Char('l') => Key::Right,
        KeyCode::Tab => Key::Tab,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Escape,
        KeyCode::Char('f') => Key::Filter,
        KeyCode::Char('r') => Key::Refresh,
        KeyCode::Char('t') => Key::Theme,
        KeyCode::Char('q') => Key::Quit,
        KeyCode::Char(c @ '1'..='3') => Key::Num(c as usize - '1' as usize),
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
