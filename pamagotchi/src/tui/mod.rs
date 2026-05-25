mod app;
mod event;
pub mod theme;
mod ui;
mod widgets;

use app::App;
use event::EventHandler;

use crossterm::event::{KeyCode, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use std::io;

pub async fn run(port: u16) -> anyhow::Result<()> {
    let mut app = App::new(port);
    app.connect().await?;

    terminal::enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut events = EventHandler::new(100);

    let result = run_loop(&mut terminal, &mut app, &mut events).await;

    terminal::disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    events: &mut EventHandler,
) -> anyhow::Result<()> {
    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        match events.next().await? {
            event::Event::Key(key) => {
                if key.kind != crossterm::event::KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Esc => break,
                    KeyCode::Enter
                        if key
                            .modifiers
                            .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
                    {
                        app.insert_newline();
                    }
                    KeyCode::Enter => app.submit_input().await,
                    KeyCode::Backspace => app.delete_char(),
                    KeyCode::Left => app.move_cursor_left(),
                    KeyCode::Right => app.move_cursor_right(),
                    KeyCode::Up => app.move_cursor_up(),
                    KeyCode::Down => app.move_cursor_down(),
                    KeyCode::PageUp => app.scroll_up(20),
                    KeyCode::PageDown => app.scroll_down(20),
                    KeyCode::Char('j') | KeyCode::Char('J')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        app.insert_newline();
                    }
                    KeyCode::Char(c) => app.insert_char(c),
                    _ => {}
                }
                app.ensure_cursor_visible();
            }
            event::Event::Tick => {
                app.poll_relay();
            }
            event::Event::Resize => {}
        }
    }

    Ok(())
}
