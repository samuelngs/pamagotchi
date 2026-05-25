use crate::tui::theme;
use ratatui::prelude::*;
use ratatui::widgets::Block;

pub enum ShortKey {
    Esc,
}

impl ShortKey {
    pub fn label(&self) -> &str {
        match self {
            ShortKey::Esc => "esc",
        }
    }

    pub fn display_width(&self) -> u16 {
        match self {
            ShortKey::Esc => 3,
        }
    }
}

pub struct Button<'a> {
    pub label: &'a str,
    pub shortkey: Option<ShortKey>,
}

impl Button<'_> {
    pub fn width(&self) -> u16 {
        let label_w = self.label.len() as u16;
        let key_w = self
            .shortkey
            .as_ref()
            .map_or(0, |k| k.display_width() + 2);
        let gap = if self.shortkey.is_some() { 1 } else { 0 };
        1 + label_w + gap + key_w
    }
}

impl Widget for Button<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = theme::INPUT_BG_DIM;
        let fg = Color::DarkGray;
        let keycap_bg = theme::KEYCAP_BG_DIM;

        let w = self.width();
        let btn_area = Rect::new(area.x, area.y, w.min(area.width), 1);
        Block::default()
            .style(Style::default().bg(bg))
            .render(btn_area, buf);

        let label_w = self.label.len() as u16;
        buf.set_string(
            area.x + 1,
            area.y,
            self.label,
            Style::default().fg(fg).bg(bg),
        );

        if let Some(ref key) = self.shortkey {
            let key_str = key.label();
            let key_dw = key.display_width();
            let key_x = area.x + 1 + label_w + 1;
            let key_area_w = key_dw + 2;

            Block::default()
                .style(Style::default().bg(keycap_bg))
                .render(Rect::new(key_x, area.y, key_area_w, 1), buf);

            buf.set_string(
                key_x + 1,
                area.y,
                key_str,
                Style::default().fg(fg).bg(keycap_bg),
            );
        }
    }
}
