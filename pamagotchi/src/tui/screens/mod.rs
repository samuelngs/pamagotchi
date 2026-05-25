pub mod chat;
pub mod settings;

use super::app::App;
use crossterm::event::KeyEvent;
use ratatui::Frame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenAction {
    None,
    Quit,
    Navigate(super::app::Screen),
    Back,
}

pub fn render(frame: &mut Frame, app: &mut App) {
    match app.screen {
        super::app::Screen::Chat => chat::render(frame, app),
        super::app::Screen::Settings => settings::render(frame, app),
    }
}

pub async fn handle_key(app: &mut App, key: KeyEvent) -> ScreenAction {
    match app.screen {
        super::app::Screen::Chat => chat::handle_key(app, key).await,
        super::app::Screen::Settings => settings::handle_key(app, key),
    }
}
