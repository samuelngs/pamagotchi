use super::app::App;
use super::widgets::{Button, InputBox, MessageList, ShortKey};
use ratatui::prelude::*;

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
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
        focused: true,
        highlighted: true,
        scroll: app.input_scroll,
    };
    let cursor_pos = input_box.cursor_position(layout[1]);
    frame.render_widget(input_box, layout[1]);
    if let Some(pos) = cursor_pos {
        frame.set_cursor_position(pos);
    }

    frame.render_widget(
        Button {
            label: "quit",
            shortkey: Some(ShortKey::Esc),
        },
        Rect::new(layout[2].x, layout[2].y, layout[2].width, 1),
    );
}
