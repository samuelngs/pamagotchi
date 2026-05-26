mod app;
mod event;
mod focus;
mod screens;
pub mod theme;
mod widgets;

use app::{App, Screen};
use screens::ScreenAction;

use crossterm::ExecutableCommand;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::prelude::*;
use std::io;

pub async fn run(port: u16) -> anyhow::Result<()> {
    let mut app = App::new(port);
    app.connect().await?;

    terminal::enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut events = event::EventHandler::new(100);

    let result = run_loop(&mut terminal, &mut app, &mut events).await;

    terminal::disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    events: &mut event::EventHandler,
) -> anyhow::Result<()> {
    loop {
        terminal.draw(|frame| screens::render(frame, app))?;

        match events.next().await? {
            event::Event::Key(key) => {
                if key.kind != crossterm::event::KeyEventKind::Press {
                    continue;
                }
                match screens::handle_key(app, key).await {
                    ScreenAction::Quit => break,
                    ScreenAction::Navigate(screen) => {
                        app.screen = screen;
                        if screen == Screen::Gateways {
                            if app.gateways.is_empty() {
                                app.focus.set(focus::FocusId::GatewayBack);
                            } else {
                                app.focus.set(focus::FocusId::GatewayList);
                            }
                            app.request_gateway_list().await;
                        } else if screen == Screen::GatewayDetail {
                            app.focus.set(focus::FocusId::GatewayDetailBack);
                        }
                    }
                    ScreenAction::Back => match app.screen {
                        Screen::GatewayDetail => {
                            app.screen = Screen::Gateways;
                            app.focus.set(focus::FocusId::GatewayList);
                        }
                        Screen::Gateways => {
                            app.screen = Screen::Settings;
                            app.focus.set(focus::FocusId::Settings);
                        }
                        Screen::Settings | Screen::Chat => {
                            app.screen = Screen::Chat;
                            app.focus.set(focus::FocusId::Settings);
                        }
                    },
                    ScreenAction::None => {}
                }
            }
            event::Event::Tick => {
                app.poll_api();
            }
            event::Event::Resize => {}
        }
    }

    Ok(())
}
