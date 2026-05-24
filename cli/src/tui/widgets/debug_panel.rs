use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

pub struct DebugPanel<'a> {
    pub lines: &'a [String],
}

impl Widget for DebugPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(Span::styled(
                " debug ",
                Style::default().fg(Color::Yellow).bold(),
            ))
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(Color::DarkGray));

        let lines: Vec<Line> = if self.lines.is_empty() {
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    " waiting...",
                    Style::default().fg(Color::DarkGray).italic(),
                )),
            ]
        } else {
            self.lines
                .iter()
                .map(|l| {
                    Line::from(Span::styled(
                        format!(" {l}"),
                        Style::default().fg(Color::DarkGray),
                    ))
                })
                .collect()
        };

        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }
}
