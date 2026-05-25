use crate::tui::app::App;
use crate::tui::widgets::{Button, ShortKey};
use super::ScreenAction;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;

pub fn render(frame: &mut Frame, _app: &mut App) {
    let area = frame.area();

    let layout = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    let title = Line::from("Settings").style(Style::default().fg(Color::White).bold());
    frame.render_widget(title, Rect::new(layout[0].x + 1, layout[0].y + 1, layout[0].width, 1));

    let back_btn = Button {
        label: "back",
        shortkey: Some(ShortKey::Esc),
        focused: true,
    };
    let back_w = back_btn.width();
    frame.render_widget(back_btn, Rect::new(layout[1].x, layout[1].y, back_w, 1));
}

pub fn handle_key(_app: &mut App, key: KeyEvent) -> ScreenAction {
    match key.code {
        KeyCode::Esc | KeyCode::Enter => ScreenAction::Back,
        _ => ScreenAction::None,
    }
}
