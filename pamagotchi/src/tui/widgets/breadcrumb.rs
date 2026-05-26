use ratatui::prelude::*;

pub struct Breadcrumb<'a> {
    pub items: &'a [&'a str],
}

impl Widget for Breadcrumb<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut x = area.x;
        let max_x = area.x.saturating_add(area.width);

        for (idx, item) in self.items.iter().enumerate() {
            if x >= max_x {
                break;
            }

            let style = if idx + 1 == self.items.len() {
                Style::default().fg(Color::White).bold()
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let remaining = max_x.saturating_sub(x) as usize;
            let text = truncate_to_width(item, remaining);
            buf.set_string(x, area.y, &text, style);
            x = x.saturating_add(text.len() as u16);

            if idx + 1 < self.items.len() && x < max_x {
                let separator = " > ";
                let remaining = max_x.saturating_sub(x) as usize;
                let text = truncate_to_width(separator, remaining);
                buf.set_string(x, area.y, &text, Style::default().fg(Color::DarkGray));
                x = x.saturating_add(text.len() as u16);
            }
        }
    }
}

fn truncate_to_width(text: &str, width: usize) -> String {
    text.chars().take(width).collect()
}
