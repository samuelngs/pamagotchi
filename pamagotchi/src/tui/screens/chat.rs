use crate::tui::app::App;
use crate::tui::focus::FocusId;
use crate::tui::widgets::{Button, InputBox, MessageList, ShortKey};
use super::ScreenAction;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    app.input_width = area.width as usize;
    let input_focused = app.focus.is(FocusId::Input);
    let input_height = InputBox::height(app.input_line_count());

    let layout = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(input_height),
        Constraint::Length(1),
    ])
    .split(area);

    frame.render_widget(
        MessageList {
            messages: &app.messages,
            scroll: app.messages_scroll,
        },
        layout[0],
    );

    let input_box = InputBox {
        text: &app.input,
        cursor: app.cursor,
        focused: input_focused,
        highlighted: input_focused,
        scroll: app.input_scroll,
    };
    let cursor_pos = input_box.cursor_position(layout[1]);
    frame.render_widget(input_box, layout[1]);
    if let Some(pos) = cursor_pos {
        frame.set_cursor_position(pos);
    }

    render_buttons(frame, app, layout[2]);
}

fn render_buttons(frame: &mut Frame, app: &App, area: Rect) {
    let quit_btn = Button {
        label: "quit",
        shortkey: Some(ShortKey::Esc),
        focused: app.focus.is(FocusId::Quit),
    };
    let quit_w = quit_btn.width();
    frame.render_widget(quit_btn, Rect::new(area.x, area.y, quit_w, 1));

    let settings_btn = Button {
        label: "settings",
        shortkey: None,
        focused: app.focus.is(FocusId::Settings),
    };
    let settings_x = area.x + quit_w + 1;
    let settings_w = settings_btn.width();
    frame.render_widget(settings_btn, Rect::new(settings_x, area.y, settings_w, 1));
}

pub async fn handle_key(app: &mut App, key: KeyEvent) -> ScreenAction {
    match key.code {
        KeyCode::Esc => return ScreenAction::Quit,
        KeyCode::Tab => { app.focus.next(); return ScreenAction::None; }
        KeyCode::BackTab => { app.focus.prev(); return ScreenAction::None; }
        _ => {}
    }

    match app.focus.current() {
        FocusId::Input => match key.code {
            KeyCode::Enter
                if key.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
            {
                app.insert_newline();
            }
            KeyCode::Enter => app.submit_input().await,
            KeyCode::Backspace if key.modifiers.contains(KeyModifiers::ALT) => {
                app.delete_word();
            }
            KeyCode::Backspace => app.delete_char(),
            KeyCode::Left => app.move_cursor_left(),
            KeyCode::Right => app.move_cursor_right(),
            KeyCode::Up => app.move_cursor_up(),
            KeyCode::Down => {
                if app.cursor_at_last_line() {
                    app.focus.next();
                } else {
                    app.move_cursor_down();
                }
            }
            KeyCode::PageUp => app.scroll_up(20),
            KeyCode::PageDown => app.scroll_down(20),
            KeyCode::Char('j') | KeyCode::Char('J')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                app.insert_newline();
            }
            KeyCode::Char(c) => app.insert_char(c),
            _ => {}
        },
        FocusId::Quit => match key.code {
            KeyCode::Enter => return ScreenAction::Quit,
            KeyCode::Up => app.focus.set(FocusId::Input),
            KeyCode::Left => app.focus.prev(),
            KeyCode::Right => app.focus.next(),
            _ => {}
        },
        FocusId::Settings => match key.code {
            KeyCode::Enter => {
                return ScreenAction::Navigate(crate::tui::app::Screen::Settings);
            }
            KeyCode::Up => app.focus.set(FocusId::Input),
            KeyCode::Left => app.focus.prev(),
            KeyCode::Right => app.focus.next(),
            _ => {}
        },
    }

    app.ensure_cursor_visible();
    ScreenAction::None
}
