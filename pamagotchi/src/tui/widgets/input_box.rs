use crate::tui::app::{visual_cursor_x, visual_cursor_y};
use crate::tui::theme;
use ratatui::prelude::*;
use ratatui::widgets::Block;

pub struct InputBox<'a> {
    pub text: &'a str,
    pub cursor: usize,
    pub focused: bool,
    pub highlighted: bool,
    pub scroll: usize,
}

impl InputBox<'_> {
    pub fn height(line_count: usize) -> u16 {
        line_count.min(10) as u16 + 2
    }

    pub fn cursor_position(&self, area: Rect) -> Option<Position> {
        if !self.focused {
            return None;
        }
        let wrap_width = self.text_width(area);
        let cx = visual_cursor_x(self.text, self.cursor, wrap_width);
        let cy = visual_cursor_y(self.text, self.cursor, wrap_width);
        let visible_cy = cy.saturating_sub(self.scroll);
        Some(Position::new(
            area.x + 1 + 2 + cx as u16,
            area.y + 1 + visible_cy as u16,
        ))
    }

    fn text_width(&self, area: Rect) -> usize {
        let content_width = area.width.saturating_sub(2) as usize;
        if content_width > 2 { content_width - 2 } else { 1 }
    }
}

impl Widget for InputBox<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = if self.highlighted {
            theme::INPUT_BG
        } else {
            theme::INPUT_BG_DIM
        };

        let cap = "▄".repeat(area.width as usize);
        buf.set_string(area.x, area.y, &cap, Style::default().fg(bg));

        let content_bg = Rect::new(area.x, area.y + 1, area.width, area.height.saturating_sub(2));
        Block::default()
            .style(Style::default().bg(bg))
            .render(content_bg, buf);

        let bot_y = area.y + area.height.saturating_sub(1);
        let bot_cap = "▄".repeat(area.width as usize);
        buf.set_string(
            area.x,
            bot_y,
            &bot_cap,
            Style::default().fg(theme::BASE_BG).bg(bg),
        );

        let content = Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            area.height.saturating_sub(2),
        );

        let prompt_color = if self.highlighted {
            theme::ACCENT
        } else {
            Color::DarkGray
        };
        let prompt_style = Style::default().fg(prompt_color).bg(bg);
        let text_style = Style::default().bg(bg);
        let prompt = "❯ ";
        let wrap_width = if content.width > 2 { content.width as usize - 2 } else { 1 };

        if self.text.is_empty() {
            let line = Line::from(vec![
                Span::styled(prompt, prompt_style),
                Span::styled(
                    "Type a message...",
                    Style::default().fg(Color::DarkGray).bg(bg),
                ),
            ]);
            buf.set_line(content.x, content.y, &line, content.width);
        } else {
            let mut visual_lines: Vec<(String, bool)> = Vec::new();
            for (li, logical_line) in self.text.split('\n').enumerate() {
                let is_first_logical = li == 0;
                if logical_line.is_empty() {
                    visual_lines.push((String::new(), is_first_logical && visual_lines.is_empty()));
                } else {
                    let chars: Vec<char> = logical_line.chars().collect();
                    for chunk in chars.chunks(wrap_width) {
                        let is_first_visual = is_first_logical && visual_lines.is_empty();
                        visual_lines.push((chunk.iter().collect(), is_first_visual));
                    }
                }
            }

            let visible_count = content.height as usize;
            let start = self.scroll;
            let end = visual_lines.len().min(start + visible_count);

            for (vi, i) in (start..end).enumerate() {
                let (ref text, show_prompt) = visual_lines[i];
                let prefix = if show_prompt {
                    Span::styled(prompt, prompt_style)
                } else {
                    Span::styled("  ", text_style)
                };
                let line = Line::from(vec![prefix, Span::styled(text.as_str(), text_style)]);
                buf.set_line(content.x, content.y + vi as u16, &line, content.width);
            }
        }
    }
}
