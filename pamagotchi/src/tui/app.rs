use super::focus::FocusManager;
use relay::{RelayReceiver, RelaySender};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Chat,
    Settings,
}

pub struct ChatMessage {
    pub content: String,
    pub is_self: bool,
}

pub struct App {
    pub port: u16,
    pub screen: Screen,
    pub input: String,
    pub cursor: usize,
    pub input_scroll: usize,
    pub input_width: usize,
    pub messages: Vec<ChatMessage>,
    pub messages_scroll: usize,
    pub composing: bool,
    pub focus: FocusManager,
    pub relay_tx: Option<RelaySender>,
    pub relay_rx: Option<RelayReceiver>,
}

impl App {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            screen: Screen::Chat,
            input: String::new(),
            cursor: 0,
            input_scroll: 0,
            input_width: 0,
            messages: Vec::new(),
            messages_scroll: 0,
            composing: false,
            focus: FocusManager::new(),
            relay_tx: None,
            relay_rx: None,
        }
    }

    pub async fn connect(&mut self) -> anyhow::Result<()> {
        let (tx, rx) = relay::connect(self.port, "default").await?;
        self.relay_tx = Some(tx);
        self.relay_rx = Some(rx);
        Ok(())
    }

    pub fn poll_relay(&mut self) {
        let rx = match &mut self.relay_rx {
            Some(rx) => rx,
            None => return,
        };
        while let Some(event) = rx.try_recv() {
            match event {
                relay::RelayEvent::Message { content } => {
                    self.messages.push(ChatMessage {
                        content,
                        is_self: false,
                    });
                    self.messages_scroll = 0;
                    self.composing = false;
                }
                relay::RelayEvent::ComposingStarted => {
                    self.composing = true;
                }
                relay::RelayEvent::ComposingStopped => {
                    self.composing = false;
                }
                relay::RelayEvent::Subscribe { .. } => {}
            }
        }
    }

    pub fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn insert_newline(&mut self) {
        self.input.insert(self.cursor, '\n');
        self.cursor += 1;
    }

    pub fn delete_char(&mut self) {
        if self.cursor > 0 {
            let prev = self.input[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.input.drain(prev..self.cursor);
            self.cursor = prev;
        }
    }

    pub fn delete_word(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let before = &self.input[..self.cursor];
        let end = before.trim_end_matches(|c: char| c.is_whitespace() && c != '\n').len();
        if end == 0 {
            self.input.drain(0..self.cursor);
            self.cursor = 0;
            return;
        }
        let start = before[..end]
            .rfind(|c: char| c.is_whitespace())
            .map_or(0, |pos| pos + 1);
        self.input.drain(start..self.cursor);
        self.cursor = start;
    }

    pub fn move_cursor_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.input[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    pub fn move_cursor_right(&mut self) {
        if self.cursor < self.input.len() {
            self.cursor = self.input[self.cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor + i)
                .unwrap_or(self.input.len());
        }
    }

    pub fn move_cursor_up(&mut self) {
        let before = &self.input[..self.cursor];
        let current_line_start = before.rfind('\n').map_or(0, |pos| pos + 1);
        if current_line_start == 0 {
            return;
        }
        let col = self.cursor - current_line_start;
        let prev_line_start = self.input[..current_line_start - 1]
            .rfind('\n')
            .map_or(0, |pos| pos + 1);
        let prev_line_len = current_line_start - 1 - prev_line_start;
        self.cursor = prev_line_start + col.min(prev_line_len);
    }

    pub fn move_cursor_down(&mut self) {
        let before = &self.input[..self.cursor];
        let current_line_start = before.rfind('\n').map_or(0, |pos| pos + 1);
        let col = self.cursor - current_line_start;
        if let Some(offset) = self.input[self.cursor..].find('\n') {
            let next_line_start = self.cursor + offset + 1;
            let next_line_end = self.input[next_line_start..]
                .find('\n')
                .map_or(self.input.len(), |pos| next_line_start + pos);
            let next_line_len = next_line_end - next_line_start;
            self.cursor = next_line_start + col.min(next_line_len);
        }
    }

    pub fn cursor_at_last_line(&self) -> bool {
        !self.input[self.cursor..].contains('\n')
    }

    pub fn ensure_cursor_visible(&mut self) {
        let cy = visual_cursor_y(&self.input, self.cursor, self.wrap_width());
        let max_visible = 10;
        if cy < self.input_scroll {
            self.input_scroll = cy;
        } else if cy >= self.input_scroll + max_visible {
            self.input_scroll = cy - max_visible + 1;
        }
    }

    pub fn input_line_count(&self) -> usize {
        visual_line_count(&self.input, self.wrap_width())
    }

    fn wrap_width(&self) -> usize {
        if self.input_width > 4 { self.input_width - 4 } else { 1 }
    }

    pub async fn submit_input(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return;
        }

        self.messages.push(ChatMessage {
            content: text.clone(),
            is_self: true,
        });
        self.messages_scroll = 0;

        if let Some(tx) = &self.relay_tx {
            let _ = tx.send(relay::RelayEvent::Message { content: text }).await;
        }

        self.input.clear();
        self.cursor = 0;
        self.input_scroll = 0;
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.messages_scroll = self.messages_scroll.saturating_add(lines);
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.messages_scroll = self.messages_scroll.saturating_sub(lines);
    }
}

fn wrapped_line_count(line: &str, width: usize) -> usize {
    if width == 0 || line.is_empty() {
        return 1;
    }
    let char_count = line.chars().count();
    (char_count + width - 1) / width
}

pub fn visual_line_count(text: &str, width: usize) -> usize {
    if width == 0 {
        return text.matches('\n').count() + 1;
    }
    text.split('\n').map(|l| wrapped_line_count(l, width)).sum()
}

pub fn visual_cursor_y(text: &str, byte_offset: usize, width: usize) -> usize {
    let before = &text[..byte_offset];
    if width == 0 {
        return before.matches('\n').count();
    }
    let lines: Vec<&str> = before.split('\n').collect();
    let mut y = 0;
    for (i, line) in lines.iter().enumerate() {
        if i < lines.len() - 1 {
            y += wrapped_line_count(line, width);
        } else {
            y += line.chars().count() / width;
        }
    }
    y
}

pub fn visual_cursor_x(text: &str, byte_offset: usize, width: usize) -> usize {
    let before = &text[..byte_offset];
    let last_line = before.rsplit('\n').next().unwrap_or(before);
    let col = last_line.chars().count();
    if width == 0 { col } else { col % width }
}
