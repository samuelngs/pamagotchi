use creature::{Creature, CreatureConfig};
use ratatui::prelude::*;

pub struct CreatureView<'a> {
    pub seed: &'a str,
    pub size: u32,
    pub animated: bool,
    pub elapsed_ms: u64,
    pub color: Option<Color>,
}

impl CreatureView<'_> {
    pub fn dimensions(size: u32) -> (u16, u16) {
        let creature = Creature::generate(&CreatureConfig {
            size: size.max(3),
            seed: "measure".into(),
            ..Default::default()
        });
        let rendered = creature.render();
        let lines = parse_lines(&rendered);
        let (_, w, h) = trim_bounds(&lines);
        (w, h)
    }
}

impl Widget for CreatureView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let creature = Creature::generate(&CreatureConfig {
            size: self.size.max(3),
            seed: self.seed.into(),
            ..Default::default()
        });

        let rendered = if self.animated {
            let frames = creature.idle_frames();
            if !frames.is_empty() {
                let idx = creature::animate::frame_index_at(&frames, self.elapsed_ms);
                let grid = &frames[idx].grid;
                if grid.width > grid.height {
                    creature::render::render_to_string_quadrant(grid, &creature.palette)
                } else {
                    creature::render::render_to_string(grid, &creature.palette)
                }
            } else {
                creature.render()
            }
        } else {
            creature.render()
        };

        let color = self.color.unwrap_or_else(|| {
            Color::Rgb(
                creature.palette.body.r,
                creature.palette.body.g,
                creature.palette.body.b,
            )
        });

        let lines = parse_lines(&rendered);
        let (left_margin, content_w, _) = trim_bounds(&lines);

        for (row, cs) in lines.iter().enumerate() {
            if row as u16 >= area.height {
                break;
            }
            let trimmed = if cs.len() > left_margin {
                &cs[left_margin..]
            } else {
                &[]
            };
            for (col, &ch) in trimmed.iter().enumerate() {
                if col as u16 >= content_w || area.x + col as u16 >= area.x + area.width {
                    break;
                }
                let cell = &mut buf[(area.x + col as u16, area.y + row as u16)];
                cell.set_char(ch);
                if ch != ' ' {
                    cell.set_fg(color);
                }
            }
        }
    }
}

fn parse_lines(rendered: &str) -> Vec<Vec<char>> {
    rendered
        .lines()
        .map(|l| strip_ansi(l).chars().collect())
        .collect()
}

fn trim_bounds(lines: &[Vec<char>]) -> (usize, u16, u16) {
    let left_margin = lines
        .iter()
        .filter(|cs| cs.iter().any(|&c| c != ' '))
        .map(|cs| cs.iter().take_while(|&&c| c == ' ').count())
        .min()
        .unwrap_or(0);

    let mut w = 0u16;
    let mut h = 0u16;
    for cs in lines {
        let after = if cs.len() > left_margin {
            let slice = &cs[left_margin..];
            let trail = slice.iter().rev().take_while(|&&c| c == ' ').count();
            slice.len() - trail
        } else {
            0
        };
        if after > 0 {
            h += 1;
        }
        w = w.max(after as u16);
    }

    (left_margin, w, h)
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut in_escape = false;
    for ch in s.chars() {
        if ch == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if ch.is_ascii_alphabetic() {
                in_escape = false;
            }
        } else {
            out.push(ch);
        }
    }
    out
}
