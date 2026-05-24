mod app;
mod event;
pub mod theme;
mod ui;
mod widgets;

use app::App;
use event::EventHandler;
use runtime::config::Config;

use crossterm::event::{KeyCode, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use std::io;

pub async fn run(config: Config) -> anyhow::Result<()> {
    let mut app = App::new(config);
    app.load_actors();

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
                match app.screen {
                    app::Screen::Dashboard => match key.code {
                        KeyCode::Esc => break,
                        KeyCode::Up | KeyCode::Char('k') => app.select_prev(),
                        KeyCode::Down | KeyCode::Char('j') => app.select_next(),
                        KeyCode::Char('c') => {
                            // TODO: create actor
                        }
                        KeyCode::Enter => {
                            if app.selected == app.actors.len() {
                                break; // quit
                            } else if app.selected == app.actors.len() + 1 {
                                // TODO: create actor
                            } else {
                                app.enter_chat();
                            }
                        }
                        _ => {}
                    },
                    app::Screen::Chat => {
                        if app.input_focused {
                            match key.code {
                                KeyCode::Esc => app.unfocus_input(),
                                KeyCode::Enter
                                    if key.modifiers.intersects(
                                        KeyModifiers::SHIFT | KeyModifiers::ALT,
                                    ) =>
                                {
                                    app.insert_newline();
                                }
                                KeyCode::Enter => app.submit_input(),
                                KeyCode::Backspace => app.delete_char(),
                                KeyCode::Left => app.move_cursor_left(),
                                KeyCode::Right => app.move_cursor_right(),
                                KeyCode::Up => app.move_cursor_up(),
                                KeyCode::Down => app.move_cursor_down(),
                                KeyCode::Char('j') | KeyCode::Char('J')
                                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    app.insert_newline();
                                }
                                KeyCode::Char(c) => app.insert_char(c),
                                _ => {}
                            }
                            app.ensure_cursor_visible();
                        } else {
                            match &app.chat_focus {
                                app::ChatFocus::Input => match key.code {
                                    KeyCode::Esc => app.exit_chat(),
                                    KeyCode::Enter => app.focus_input(),
                                    KeyCode::Down | KeyCode::Char('j') => {
                                        app.chat_focus = app::ChatFocus::BackButton;
                                    }
                                    KeyCode::PageUp => app.scroll_up(20),
                                    KeyCode::PageDown => app.scroll_down(20),
                                    KeyCode::Char(c) => {
                                        app.focus_input();
                                        app.insert_char(c);
                                    }
                                    _ => {}
                                },
                                app::ChatFocus::BackButton => match key.code {
                                    KeyCode::Esc | KeyCode::Enter => app.exit_chat(),
                                    KeyCode::Up | KeyCode::Char('k') => {
                                        app.chat_focus = app::ChatFocus::Input;
                                    }
                                    _ => {}
                                },
                            }
                        }
                    }
                }
            }
            event::Event::Tick => {}
            event::Event::Resize(_, _) => {}
        }
    }
    Ok(())
}
