use crate::tui::theme;
use ratatui::prelude::*;
use ratatui::widgets::Block;

pub struct ListItemButton<'a> {
    pub label: &'a str,
    pub selected: bool,
}

impl ListItemButton<'_> {
    pub fn height() -> u16 {
        3
    }
}

impl Widget for ListItemButton<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 3 || area.width < 4 {
            return;
        }

        let bg = if self.selected {
            theme::INPUT_BG
        } else {
            theme::INPUT_BG_DIM
        };
        let fg = if self.selected {
            Color::White
        } else {
            Color::DarkGray
        };

        // Top cap
        let cap = "▄".repeat(area.width as usize);
        buf.set_string(area.x, area.y, &cap, Style::default().fg(bg));

        // Content
        let content_y = area.y + 1;
        Block::default()
            .style(Style::default().bg(bg))
            .render(Rect::new(area.x, content_y, area.width, 1), buf);
        buf.set_string(
            area.x + 1,
            content_y,
            self.label,
            Style::default().fg(fg).bg(bg),
        );

        // Bottom cap
        buf.set_string(
            area.x,
            area.y + 2,
            &cap,
            Style::default().fg(theme::BASE_BG).bg(bg),
        );
    }
}
