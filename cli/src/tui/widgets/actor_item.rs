use super::character_view::CharacterView;
use crate::tui::theme;
use ratatui::prelude::*;
use ratatui::widgets::Block;

const CREATURE_SIZE: u32 = 3;

pub struct ActorItem<'a> {
    pub id: &'a str,
    pub platform_count: usize,
    pub selected: bool,
    pub elapsed_ms: u64,
}

impl ActorItem<'_> {
    pub fn height() -> u16 {
        let (_, h) = CharacterView::dimensions(CREATURE_SIZE);
        h + 1 // +1 for top cap
    }
}

impl Widget for ActorItem<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < Self::height() || area.width < 10 {
            return;
        }

        let bg = if self.selected {
            theme::INPUT_BG
        } else {
            theme::INPUT_BG_DIM
        };

        // Top cap
        let cap = "▄".repeat(area.width as usize);
        buf.set_string(area.x, area.y, &cap, Style::default().fg(bg));

        // Content background
        let content_h = area.height - 1;
        let content = Rect::new(area.x, area.y + 1, area.width, content_h);
        Block::default()
            .style(Style::default().bg(bg))
            .render(content, buf);

        // Creature
        let (cw, _) = CharacterView::dimensions(CREATURE_SIZE);
        let color = if self.selected {
            None
        } else {
            Some(Color::DarkGray)
        };
        CharacterView {
            seed: self.id,
            size: CREATURE_SIZE,
            animated: self.selected,
            elapsed_ms: self.elapsed_ms,
            color,
        }
        .render(
            Rect::new(content.x + 1, content.y, cw, content_h),
            buf,
        );

        // Text: id + model
        let text_x = content.x + 1 + cw + 1;
        if text_x >= content.x + content.width {
            return;
        }

        let id_style = if self.selected {
            Style::default().fg(Color::White).bg(bg).bold()
        } else {
            Style::default().fg(Color::Gray).bg(bg)
        };
        buf.set_string(text_x, content.y, self.id, id_style);

        let platforms = if self.platform_count == 1 {
            "1 platform".to_string()
        } else {
            format!("{} platforms", self.platform_count)
        };
        buf.set_string(
            text_x,
            content.y + 1,
            &platforms,
            Style::default().fg(Color::DarkGray).bg(bg),
        );
    }
}
