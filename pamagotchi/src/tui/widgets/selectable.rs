use super::button::ShortKey;
use crate::tui::theme;
use ratatui::prelude::*;

pub struct Selectable<'a> {
    pub label: &'a str,
    pub shortkey: Option<ShortKey>,
    pub focused: bool,
}

impl Selectable<'_> {
    pub fn width(&self) -> u16 {
        let label_w = self.label.len() as u16;
        let key_w = self.shortkey.as_ref().map_or(0, |k| k.display_width() + 3);
        2 + label_w + key_w
    }
}

impl Widget for Selectable<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let fg = if self.focused {
            theme::ACCENT
        } else {
            Color::DarkGray
        };
        let style = Style::default().fg(fg);

        buf.set_string(area.x, area.y, "•", style);
        buf.set_string(area.x + 2, area.y, self.label, style);

        if let Some(ref key) = self.shortkey {
            let key_x = area.x + 2 + self.label.len() as u16 + 1;
            let label = key.label();
            buf.set_string(key_x, area.y, &label, style);
        }
    }
}
