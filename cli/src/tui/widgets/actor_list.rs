use super::actor_item::ActorItem;
use crate::tui::app::ActorInfo;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

pub struct ActorList<'a> {
    pub actors: &'a [ActorInfo],
    pub selected: Option<usize>,
    pub elapsed_ms: u64,
}

impl Widget for ActorList<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.actors.is_empty() {
            Paragraph::new(Span::styled(
                "no actors configured",
                Style::default().fg(Color::DarkGray),
            ))
            .render(area, buf);
            return;
        }

        let item_h = ActorItem::height();
        let gap = 1u16;

        for (i, actor) in self.actors.iter().enumerate() {
            let y = area.y + (i as u16 * (item_h + gap));
            if y + item_h > area.y + area.height {
                break;
            }
            ActorItem {
                id: &actor.id,
                platform_count: actor.platform_count,
                selected: self.selected == Some(i),
                elapsed_ms: self.elapsed_ms,
            }
            .render(Rect::new(area.x, y, area.width, item_h), buf);
        }
    }
}
