use super::app::{App, ChatFocus, Screen};
use super::widgets::{ActorList, Button, DebugPanel, InputBox, MessageList, ShortKey};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Padding, Paragraph};

pub fn render(frame: &mut Frame, app: &App) {
    match app.screen {
        Screen::Dashboard => render_dashboard(frame, app),
        Screen::Chat => render_chat(frame, app),
    }
}

fn render_dashboard(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let layout = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Length(1), // gap
        Constraint::Min(0),   // actor list
        Constraint::Length(1), // buttons
    ])
    .split(area);

    // Title
    frame.render_widget(
        Paragraph::new(Span::styled(
            "pamagotchi",
            Style::default().fg(Color::White).bold(),
        ))
        .block(Block::default().padding(Padding::horizontal(1))),
        layout[0],
    );

    // Actor list
    let actor_selected = if app.selected < app.actors.len() {
        Some(app.selected)
    } else {
        None
    };
    frame.render_widget(
        ActorList {
            actors: &app.actors,
            selected: actor_selected,
            elapsed_ms: app.elapsed_ms(),
        },
        layout[2],
    );

    // Bottom button bar
    let row = layout[3];
    let mut x = row.x;

    let buttons: Vec<Button> = vec![
        Button {
            label: "quit",
            focused: app.selected == app.actors.len(),
            focusable: true,
            shortkey: Some(ShortKey::Esc),
        },
        Button {
            label: "create actor",
            focused: app.selected == app.actors.len() + 1,
            focusable: true,
            shortkey: Some(ShortKey::Key("c")),
        },
        Button {
            label: "navigate",
            focused: false,
            focusable: false,
            shortkey: Some(ShortKey::UpDown),
        },
        Button {
            label: "select",
            focused: false,
            focusable: false,
            shortkey: Some(ShortKey::Enter),
        },
    ];

    for btn in buttons {
        let w = btn.width();
        frame.render_widget(btn, Rect::new(x, row.y, w, 1));
        x += w + 1;
    }
}

fn render_chat(frame: &mut Frame, app: &App) {
    let area = frame.area();

    if app.verbose {
        let columns = Layout::horizontal([
            Constraint::Percentage(65),
            Constraint::Percentage(35),
        ])
        .split(area);

        render_chat_panel(frame, columns[0], app);
        frame.render_widget(
            DebugPanel {
                lines: &app.debug_lines,
            },
            columns[1],
        );
    } else {
        render_chat_panel(frame, area, app);
    }
}

fn render_chat_panel(frame: &mut Frame, area: Rect, app: &App) {
    let input_height = InputBox::height(app.input_line_count());

    let layout = Layout::vertical([
        Constraint::Min(0),          // messages
        Constraint::Length(input_height), // input
        Constraint::Length(1),       // back button
    ])
    .split(area);

    // Messages
    frame.render_widget(
        MessageList {
            messages: &app.messages,
            scroll: app.messages_scroll,
            actor: app.actors.get(app.selected),
        },
        layout[0],
    );

    // Input
    let input_highlighted = matches!(app.chat_focus, ChatFocus::Input) || app.input_focused;
    let input_box = InputBox {
        text: &app.input,
        cursor: app.cursor,
        focused: app.input_focused,
        highlighted: input_highlighted,
        scroll: app.input_scroll,
    };
    let cursor_pos = input_box.cursor_position(layout[1]);
    frame.render_widget(input_box, layout[1]);
    if let Some(pos) = cursor_pos {
        frame.set_cursor_position(pos);
    }

    // Back button
    let row = layout[2];
    frame.render_widget(
        Button {
            label: "back",
            focused: matches!(app.chat_focus, ChatFocus::BackButton),
            focusable: true,
            shortkey: Some(ShortKey::Esc),
        },
        Rect::new(row.x, row.y, row.width, 1),
    );
}
