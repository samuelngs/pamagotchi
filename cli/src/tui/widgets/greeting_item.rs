use super::character_view::CharacterView;
use ratatui::prelude::*;

const CREATURE_SIZE: u32 = 3;

pub struct GreetingItem<'a> {
    pub id: &'a str,
    pub platform_count: usize,
}

impl GreetingItem<'_> {
    pub fn height() -> u16 {
        let (_, ch) = CharacterView::dimensions(CREATURE_SIZE);
        1 + ch + 1 // 1 empty row + creature + prompt line
    }
}

impl Widget for GreetingItem<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (cw, ch) = CharacterView::dimensions(CREATURE_SIZE);
        if area.height < 1 || area.width < cw {
            return;
        }

        let offset_y = area.y + 1;
        if offset_y >= area.y + area.height {
            return;
        }
        let remaining = area.height - 1;
        let creature_h = ch.min(remaining);
        CharacterView {
            seed: self.id,
            size: CREATURE_SIZE,
            animated: false,
            elapsed_ms: 0,
            color: None,
        }
        .render(Rect::new(area.x, offset_y, cw, creature_h), buf);

        let text_x = area.x + cw + 1;
        if text_x >= area.x + area.width {
            return;
        }

        buf.set_string(
            text_x,
            offset_y,
            self.id,
            Style::default().fg(Color::Gray),
        );

        if offset_y + 1 < area.y + area.height {
            let platforms = if self.platform_count == 1 {
                "1 platform connected".to_string()
            } else {
                format!("{} platforms connected", self.platform_count)
            };
            buf.set_string(
                text_x,
                offset_y + 1,
                &platforms,
                Style::default().fg(Color::White),
            );
        }

        let prompt_y = offset_y + creature_h;
        if prompt_y < area.y + area.height {
            buf.set_string(
                area.x,
                prompt_y,
                "What's on your mind?",
                Style::default().fg(Color::DarkGray).italic(),
            );
        }
    }
}
