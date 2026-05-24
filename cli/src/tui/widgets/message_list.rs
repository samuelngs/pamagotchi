use super::greeting_item::GreetingItem;
use super::message_item::MessageItem;
use crate::tui::app::{ActorInfo, ChatMessage};
use ratatui::prelude::*;

pub struct MessageList<'a> {
    pub messages: &'a [ChatMessage],
    pub scroll: usize,
    pub actor: Option<&'a ActorInfo>,
}

impl Widget for MessageList<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let greeting_h = self
            .actor
            .map(|_| GreetingItem::height())
            .unwrap_or(0);

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
        let gaps = if self.actor.is_some() {
            msg_items.len() as u16
        } else {
            msg_items.len().saturating_sub(1) as u16
        };
        let total_height = greeting_h + messages_h + gaps;
        let max_scroll = total_height.saturating_sub(area.height);
        let scroll = max_scroll.saturating_sub(self.scroll as u16);

        let mut y_offset = 0i32;

        // Greeting
        if let Some(actor) = self.actor {
            let h = greeting_h;
            let item_y = y_offset - scroll as i32;

            if item_y + h as i32 > 0 && item_y < area.height as i32 {
                let render_y = item_y.max(0) as u16;
                let render_h = (h as i32 - (render_y as i32 - item_y))
                    .min(area.height as i32 - render_y as i32)
                    as u16;

                if render_h > 0 {
                    GreetingItem {
                        id: &actor.id,
                        platform_count: actor.platform_count,
                    }
                    .render(
                        Rect::new(area.x, area.y + render_y, area.width, render_h),
                        buf,
                    );
                }
            }
            y_offset += h as i32;
        }

        // Messages
        for (i, item) in msg_items.iter().enumerate() {
            if i > 0 || self.actor.is_some() {
                y_offset += 1; // gap
            }
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
