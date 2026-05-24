use crate::tui::theme;
use ratatui::prelude::*;
use ratatui::widgets::Block;

pub struct MessageItem<'a> {
    pub content: &'a str,
    pub is_self: bool,
    pub width: u16,
}

impl MessageItem<'_> {
    pub fn height(&self) -> u16 {
        self.content_height()
    }

    fn content_height(&self) -> u16 {
        let w = self.content_width() as usize;
        if w == 0 {
            return 1;
        }
        let mut h = 0u16;
        for line in self.content.split('\n') {
            h += wrap_text(line, w).len() as u16;
        }
        h.max(1)
    }

    fn content_width(&self) -> u16 {
        self.width
    }
}

impl Widget for MessageItem<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let content_w = self.content_width() as usize;
        if content_w == 0 {
            return;
        }

        if self.is_self {
            render_self(&self, area, buf, content_w);
        } else {
            render_agent(&self, area, buf, content_w);
        }
    }
}

fn render_self(item: &MessageItem, area: Rect, buf: &mut Buffer, content_w: usize) {
    let bg = theme::INPUT_BG_DIM;

    Block::default()
        .style(Style::default().bg(bg))
        .render(area, buf);

    let mut row = 0u16;
    for line in item.content.split('\n') {
        for wline in wrap_text(line, content_w) {
            if row >= area.height {
                break;
            }
            buf.set_string(
                area.x,
                area.y + row,
                &wline,
                Style::default().fg(Color::White).bg(bg),
            );
            row += 1;
        }
    }
}

fn render_agent(item: &MessageItem, area: Rect, buf: &mut Buffer, content_w: usize) {
    let mut row = 0u16;
    for line in item.content.split('\n') {
        for wline in wrap_text(line, content_w) {
            if row >= area.height {
                return;
            }
            buf.set_string(
                area.x,
                area.y + row,
                &wline,
                Style::default().fg(Color::White),
            );
            row += 1;
        }
    }
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 || text.is_empty() {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current = word.to_string();
        } else if current.len() + 1 + word.len() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = word.to_string();
        }
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}
