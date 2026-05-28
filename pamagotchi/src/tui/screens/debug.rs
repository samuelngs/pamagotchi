use super::ScreenAction;
use crate::tui::app::App;
use crate::tui::widgets::{Breadcrumb, Button, ShortKey};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Paragraph, Wrap};

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let layout = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    frame.render_widget(
        Breadcrumb {
            items: &["actor", "debug"],
        },
        Rect::new(layout[0].x + 1, layout[0].y + 1, layout[0].width, 1),
    );

    let snapshot = Paragraph::new(app.debug_snapshot.as_str())
        .wrap(Wrap { trim: false })
        .scroll((app.debug_scroll as u16, 0));
    frame.render_widget(snapshot, layout[1]);

    let back_btn = Button {
        label: "back",
        shortkey: Some(ShortKey::Esc),
        focused: true,
    };
    let back_w = back_btn.width();
    frame.render_widget(back_btn, Rect::new(layout[2].x, layout[2].y, back_w, 1));

    let refresh = "r refresh";
    let x = layout[2].x.saturating_add(back_w).saturating_add(2);
    frame.render_widget(refresh, Rect::new(x, layout[2].y, refresh.len() as u16, 1));
}

pub async fn handle_key(app: &mut App, key: KeyEvent) -> ScreenAction {
    match key.code {
        KeyCode::Esc => ScreenAction::Back,
        KeyCode::Char('r') | KeyCode::Char('R') => {
            app.request_debug_snapshot().await;
            ScreenAction::None
        }
        KeyCode::Up => {
            app.debug_scroll_down(1);
            ScreenAction::None
        }
        KeyCode::Down => {
            app.debug_scroll_up(1);
            ScreenAction::None
        }
        KeyCode::PageUp => {
            app.debug_scroll_down(20);
            ScreenAction::None
        }
        KeyCode::PageDown => {
            app.debug_scroll_up(20);
            ScreenAction::None
        }
        _ => ScreenAction::None,
    }
}
