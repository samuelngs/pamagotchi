use super::*;

impl App {
    pub fn scroll_up(&mut self, lines: usize) {
        self.messages_scroll = self.messages_scroll.saturating_add(lines);
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.messages_scroll = self.messages_scroll.saturating_sub(lines);
    }

    pub fn debug_scroll_up(&mut self, lines: usize) {
        self.debug_scroll = self.debug_scroll.saturating_add(lines);
    }

    pub fn debug_scroll_down(&mut self, lines: usize) {
        self.debug_scroll = self.debug_scroll.saturating_sub(lines);
    }
}
