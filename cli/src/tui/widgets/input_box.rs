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
        let (cx, cy) = cursor_pos(self.text, self.cursor);
        let visible_cy = cy.saturating_sub(self.scroll);
        Some(Position::new(
            area.x + 1 + 2 + cx as u16,
            area.y + 1 + visible_cy as u16,
        ))
    }
}

impl Widget for InputBox<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = if self.highlighted {
            theme::INPUT_BG
        } else {
            theme::INPUT_BG_DIM
        };

        // Top cap
        let cap = "▄".repeat(area.width as usize);
        buf.set_string(area.x, area.y, &cap, Style::default().fg(bg));

        // Content area
        let content_bg = Rect::new(area.x, area.y + 1, area.width, area.height.saturating_sub(2));
        Block::default()
            .style(Style::default().bg(bg))
            .render(content_bg, buf);

        // Bottom cap
        let bot_y = area.y + area.height.saturating_sub(1);
        let bot_cap = "▄".repeat(area.width as usize);
        buf.set_string(
            area.x,
            bot_y,
            &bot_cap,
            Style::default().fg(theme::BASE_BG).bg(bg),
        );

        // Text content
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
            let all_lines: Vec<&str> = self.text.split('\n').collect();
            let visible_count = content.height as usize;
            let start = self.scroll;
            let end = all_lines.len().min(start + visible_count);

            for (vi, i) in (start..end).enumerate() {
                let prefix = if i == 0 {
                    Span::styled(prompt, prompt_style)
                } else {
                    Span::styled("  ", text_style)
                };
                let line = Line::from(vec![prefix, Span::styled(all_lines[i], text_style)]);
                buf.set_line(content.x, content.y + vi as u16, &line, content.width);
            }
        }
    }
}

fn cursor_pos(text: &str, byte_offset: usize) -> (usize, usize) {
    let before = &text[..byte_offset];
    let y = before.matches('\n').count();
    let x = before
        .rfind('\n')
        .map_or(before.len(), |pos| before.len() - pos - 1);
    (x, y)
}
