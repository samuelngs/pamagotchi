use super::*;

impl App {
    pub fn insert_char(&mut self, c: char) {
        self.cursor = clamp_to_char_boundary(&self.input, self.cursor);
        self.input.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn insert_newline(&mut self) {
        self.cursor = clamp_to_char_boundary(&self.input, self.cursor);
        self.input.insert(self.cursor, '\n');
        self.cursor += 1;
    }

    pub fn delete_char(&mut self) {
        self.cursor = clamp_to_char_boundary(&self.input, self.cursor);
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
        self.cursor = clamp_to_char_boundary(&self.input, self.cursor);
        if self.cursor == 0 {
            return;
        }
        let before = &self.input[..self.cursor];
        let end = before
            .trim_end_matches(|c: char| c.is_whitespace() && c != '\n')
            .len();
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
        self.cursor = clamp_to_char_boundary(&self.input, self.cursor);
        if self.cursor > 0 {
            self.cursor = self.input[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    pub fn move_cursor_right(&mut self) {
        self.cursor = clamp_to_char_boundary(&self.input, self.cursor);
        if self.cursor < self.input.len() {
            self.cursor = self.input[self.cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor + i)
                .unwrap_or(self.input.len());
        }
    }

    pub fn move_cursor_up(&mut self) {
        self.cursor = clamp_to_char_boundary(&self.input, self.cursor);
        let before = &self.input[..self.cursor];
        let current_line_start = before.rfind('\n').map_or(0, |pos| pos + 1);
        if current_line_start == 0 {
            return;
        }
        let col = self.input[current_line_start..self.cursor].chars().count();
        let prev_line_start = self.input[..current_line_start - 1]
            .rfind('\n')
            .map_or(0, |pos| pos + 1);
        let prev_line_end = current_line_start - 1;
        self.cursor = byte_offset_for_char_column(&self.input, prev_line_start, prev_line_end, col);
    }

    pub fn move_cursor_down(&mut self) {
        self.cursor = clamp_to_char_boundary(&self.input, self.cursor);
        let before = &self.input[..self.cursor];
        let current_line_start = before.rfind('\n').map_or(0, |pos| pos + 1);
        let col = self.input[current_line_start..self.cursor].chars().count();
        if let Some(offset) = self.input[self.cursor..].find('\n') {
            let next_line_start = self.cursor + offset + 1;
            let next_line_end = self.input[next_line_start..]
                .find('\n')
                .map_or(self.input.len(), |pos| next_line_start + pos);
            self.cursor =
                byte_offset_for_char_column(&self.input, next_line_start, next_line_end, col);
        }
    }

    pub fn cursor_at_last_line(&self) -> bool {
        let cursor = clamp_to_char_boundary(&self.input, self.cursor);
        !self.input[cursor..].contains('\n')
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
        if self.input_width > 4 {
            self.input_width - 4
        } else {
            1
        }
    }
}
