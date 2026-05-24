use runtime::config::Config;
use std::time::Instant;

pub enum Screen {
    Dashboard,
    Chat,
}

pub enum ChatFocus {
    Input,
    BackButton,
}

pub struct ActorInfo {
    pub id: String,
    pub platform_count: usize,
}

pub struct ChatMessage {
    pub sender: String,
    pub content: String,
    pub is_self: bool,
}

pub struct App {
    pub config: Config,
    pub screen: Screen,
    pub actors: Vec<ActorInfo>,
    pub selected: usize,
    pub input: String,
    pub cursor: usize,
    pub input_focused: bool,
    pub input_scroll: usize,
    pub chat_focus: ChatFocus,
    pub messages: Vec<ChatMessage>,
    pub messages_scroll: usize,
    pub verbose: bool,
    pub debug_lines: Vec<String>,
    pub started_at: Instant,
}

impl App {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            screen: Screen::Dashboard,
            actors: Vec::new(),
            selected: 0,
            input: String::new(),
            cursor: 0,
            input_focused: false,
            input_scroll: 0,
            chat_focus: ChatFocus::Input,
            messages: Vec::new(),
            messages_scroll: 0,
            verbose: false,
            debug_lines: Vec::new(),
            started_at: Instant::now(),
        }
    }

    pub fn load_actors(&mut self) {
        self.actors = self
            .config
            .actors
            .iter()
            .map(|entry| ActorInfo {
                id: entry.id.clone(),
                platform_count: entry.platforms.len() + 1,
            })
            .collect();
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    // -- Dashboard navigation --

    pub fn select_prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn select_next(&mut self) {
        // actors + quit + create actor
        if self.selected < self.actors.len() + 1 {
            self.selected += 1;
        }
    }

    pub fn selected_actor(&self) -> Option<&ActorInfo> {
        self.actors.get(self.selected)
    }

    // -- Screen transitions --

    pub fn enter_chat(&mut self) {
        if self.actors.is_empty() {
            return;
        }
        self.screen = Screen::Chat;
        self.messages.clear();
        self.input.clear();
        self.cursor = 0;
        self.input_focused = false;
        self.input_scroll = 0;
        self.chat_focus = ChatFocus::Input;
        self.debug_lines.clear();
        self.messages_scroll = 0;
    }

    pub fn exit_chat(&mut self) {
        self.screen = Screen::Dashboard;
    }

    // -- Input editing --

    pub fn focus_input(&mut self) {
        self.input_focused = true;
    }

    pub fn unfocus_input(&mut self) {
        self.input_focused = false;
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

    pub fn ensure_cursor_visible(&mut self) {
        let cy = self.input[..self.cursor].matches('\n').count();
        let max_visible = 10;
        if cy < self.input_scroll {
            self.input_scroll = cy;
        } else if cy >= self.input_scroll + max_visible {
            self.input_scroll = cy - max_visible + 1;
        }
    }

    pub fn input_line_count(&self) -> usize {
        self.input.matches('\n').count() + 1
    }

    pub fn submit_input(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return;
        }

        if let Some(cmd) = text.strip_prefix('/') {
            self.handle_command(cmd);
        } else {
            self.messages.push(ChatMessage {
                sender: "you".into(),
                content: text,
                is_self: true,
            });
            self.messages_scroll = 0;
        }

        self.input.clear();
        self.cursor = 0;
        self.input_scroll = 0;
    }

    // -- Message scroll --

    pub fn scroll_up(&mut self, lines: usize) {
        self.messages_scroll = self.messages_scroll.saturating_add(lines);
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.messages_scroll = self.messages_scroll.saturating_sub(lines);
    }

    // -- Commands --

    fn handle_command(&mut self, cmd: &str) {
        match cmd.trim() {
            "verbose on" => self.verbose = true,
            "verbose off" => self.verbose = false,
            "verbose" => self.verbose = !self.verbose,
            _ => {
                self.debug_lines.push(format!("unknown command: /{cmd}"));
            }
        }
    }
}
