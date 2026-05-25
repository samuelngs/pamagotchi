use super::greeting::Greeting;
use super::message_item::MessageItem;
use crate::tui::app::ChatMessage;
use ratatui::prelude::*;

pub struct MessageList<'a> {
    pub messages: &'a [ChatMessage],
    pub scroll: usize,
}

impl Widget for MessageList<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let greeting_h = Greeting::height();

        let msg_items: Vec<MessageItem> = self
            .messages
            .iter()
            .map(|msg| MessageItem {
                content: &msg.content,
                is_self: msg.is_self,
                width: area.width,
            })
            .collect();

        let messages_h: u16 = msg_items.iter().map(|m| m.height()).sum();
        let gaps = msg_items.len() as u16;
        let total_height = greeting_h + messages_h + gaps;
        let max_scroll = total_height.saturating_sub(area.height);
        let scroll = max_scroll.saturating_sub(self.scroll as u16);

        let mut y_offset = 0i32;

        let h = greeting_h;
        let item_y = y_offset - scroll as i32;
        if item_y + h as i32 > 0 && item_y < area.height as i32 {
            let render_y = item_y.max(0) as u16;
            let render_h = (h as i32 - (render_y as i32 - item_y))
                .min(area.height as i32 - render_y as i32)
                as u16;
            if render_h > 0 {
                Greeting.render(
                    Rect::new(area.x, area.y + render_y, area.width, render_h),
                    buf,
                );
            }
        }
        y_offset += h as i32;

        for item in &msg_items {
            y_offset += 1;
            let h = item.height();
            let item_y = y_offset - scroll as i32;

            if item_y + h as i32 > 0 && item_y < area.height as i32 {
                let render_y = item_y.max(0) as u16;
                let render_h = (h as i32 - (render_y as i32 - item_y))
                    .min(area.height as i32 - render_y as i32)
                    as u16;

                if render_h > 0 {
                    MessageItem {
                        content: item.content,
                        is_self: item.is_self,
                        width: item.width,
                    }
                    .render(
                        Rect::new(area.x, area.y + render_y, area.width, render_h),
                        buf,
                    );
                }
            }
            y_offset += h as i32;
        }
    }
}
