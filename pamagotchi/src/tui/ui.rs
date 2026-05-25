use super::app::App;
use super::focus::FocusId;
use super::widgets::{Button, InputBox, MessageList, ShortKey};
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

    let quit_btn = Button {
        label: "quit",
        shortkey: Some(ShortKey::Esc),
        focused: app.focus.is(FocusId::Quit),
    };
    let quit_w = quit_btn.width();
    frame.render_widget(quit_btn, Rect::new(layout[2].x, layout[2].y, quit_w, 1));

    let gw_btn = Button {
        label: "settings",
        shortkey: None,
        focused: app.focus.is(FocusId::Gateway),
    };
    let gw_x = layout[2].x + quit_w + 1;
    let gw_w = gw_btn.width();
    frame.render_widget(gw_btn, Rect::new(gw_x, layout[2].y, gw_w, 1));
}
