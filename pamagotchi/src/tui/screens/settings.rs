use super::ScreenAction;
use crate::tui::app::{App, SettingsSelection};
use crate::tui::widgets::{Breadcrumb, Button, Selectable, ShortKey};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let layout = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);

    frame.render_widget(
        Breadcrumb {
            items: &["actor", "settings"],
        },
        Rect::new(layout[0].x + 1, layout[0].y + 1, layout[0].width, 1),
    );

    let gateways_item = Selectable {
        label: "gateway(s)",
        shortkey: None,
        focused: app.settings_selection == SettingsSelection::Gateways,
    };
    let gateways_w = gateways_item.width();
    frame.render_widget(
        gateways_item,
        Rect::new(layout[0].x + 1, layout[0].y + 3, gateways_w, 1),
    );

    let debug_item = Selectable {
        label: "debug snapshot",
        shortkey: None,
        focused: app.settings_selection == SettingsSelection::Debug,
    };
    let debug_w = debug_item.width();
    frame.render_widget(
        debug_item,
        Rect::new(layout[0].x + 1, layout[0].y + 5, debug_w, 1),
    );

    let back_btn = Button {
        label: "back",
        shortkey: Some(ShortKey::Esc),
        focused: app.settings_selection == SettingsSelection::Back,
    };
    let back_w = back_btn.width();
    frame.render_widget(back_btn, Rect::new(layout[1].x, layout[1].y, back_w, 1));
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> ScreenAction {
    match key.code {
        KeyCode::Esc => ScreenAction::Back,
        KeyCode::Up | KeyCode::Down => {
            app.settings_selection = match (app.settings_selection, key.code) {
                (SettingsSelection::Gateways, KeyCode::Up) => SettingsSelection::Back,
                (SettingsSelection::Gateways, KeyCode::Down) => SettingsSelection::Debug,
                (SettingsSelection::Debug, KeyCode::Up) => SettingsSelection::Gateways,
                (SettingsSelection::Debug, KeyCode::Down) => SettingsSelection::Back,
                (SettingsSelection::Back, KeyCode::Up) => SettingsSelection::Debug,
                (SettingsSelection::Back, KeyCode::Down) => SettingsSelection::Gateways,
                (selection, _) => selection,
            };
            ScreenAction::None
        }
        KeyCode::Enter => match app.settings_selection {
            SettingsSelection::Gateways => {
                ScreenAction::Navigate(crate::tui::app::Screen::Gateways)
            }
            SettingsSelection::Debug => ScreenAction::Navigate(crate::tui::app::Screen::Debug),
            SettingsSelection::Back => ScreenAction::Back,
        },
        _ => ScreenAction::None,
    }
}
