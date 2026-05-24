use crate::grid::{Cell, Grid};
use crate::palette::{Palette, Rgb};

/// Render a creature grid + palette to an ANSI truecolor string using half-blocks.
///
/// Each terminal character cell encodes two vertical pixels via `▀` (upper half block).
/// Foreground color = top pixel, background color = bottom pixel.
/// Empty cells reset to terminal default (transparent).
pub fn render_to_string(grid: &Grid, palette: &Palette) -> String {
    let mut out = String::new();

    // Process rows in pairs
    let mut y = 0;
    while y < grid.height {
        let top_y = y;
        let bot_y = y + 1;

        for x in 0..grid.width {
            let top_cell = grid.get(x, top_y);
            let bot_cell = if bot_y < grid.height {
                grid.get(x, bot_y)
            } else {
                Some(Cell::Empty)
            };

            let top_color = cell_color(top_cell, palette);
            let bot_color = cell_color(bot_cell, palette);

            push_cell(&mut out, top_color, bot_color);
        }

        push_reset(&mut out);
        out.push('\n');
        y += 2;
    }

    out
}

fn cell_color(cell: Option<Cell>, palette: &Palette) -> Option<Rgb> {
    match cell? {
        Cell::Empty | Cell::Eye => None,
        _ => Some(palette.body),
    }
}

fn push_cell(out: &mut String, top: Option<Rgb>, bot: Option<Rgb>) {
    match (top, bot) {
        (None, None) => out.push(' '),
        (Some(c), None) => {
            push_fg(out, c);
            out.push('▀');
            push_reset(out);
        }
        (None, Some(c)) => {
            push_fg(out, c);
            out.push('▄');
            push_reset(out);
        }
        (Some(fg), Some(bg)) if fg == bg => {
            push_fg(out, fg);
            out.push('█');
            push_reset(out);
        }
        (Some(fg), Some(bg)) => {
            push_fg(out, fg);
            push_bg(out, bg);
            out.push('▀');
            push_reset(out);
        }
    }
}

fn push_fg(out: &mut String, c: Rgb) {
    use std::fmt::Write;
    let _ = write!(out, "\x1b[38;2;{};{};{}m", c.r, c.g, c.b);
}

fn push_bg(out: &mut String, c: Rgb) {
    use std::fmt::Write;
    let _ = write!(out, "\x1b[48;2;{};{};{}m", c.r, c.g, c.b);
}

fn push_reset(out: &mut String) {
    out.push_str("\x1b[0m");
}

/// Render a row of creatures side by side, separated by spacing columns.
pub fn render_row(creatures: &[(&Grid, &Palette)], spacing: u32) -> String {
    if creatures.is_empty() {
        return String::new();
    }

    let max_height = creatures
        .iter()
        .map(|(g, _)| g.height)
        .max()
        .unwrap_or(0);

    let mut out = String::new();
    let mut y = 0;

    while y < max_height {
        let top_y = y;
        let bot_y = y + 1;

        for (ci, (grid, palette)) in creatures.iter().enumerate() {
            if ci > 0 {
                for _ in 0..spacing {
                    out.push(' ');
                }
            }

            for x in 0..grid.width {
                let top_cell = if top_y < grid.height {
                    grid.get(x, top_y)
                } else {
                    Some(Cell::Empty)
                };
                let bot_cell = if bot_y < grid.height {
                    grid.get(x, bot_y)
                } else {
                    Some(Cell::Empty)
                };

                let top_color = cell_color(top_cell, palette);
                let bot_color = cell_color(bot_cell, palette);

                push_cell(&mut out, top_color, bot_color);
            }
        }

        push_reset(&mut out);
        out.push('\n');
        y += 2;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_contains_halfblock() {
        let mut grid = Grid::new(4, 4);
        let palette = Palette {
            body: Rgb::new(200, 100, 100),
            outline: Rgb::new(50, 25, 25),
            eye: Rgb::new(240, 240, 240),
            pupil: Rgb::new(20, 20, 20),
        };
        grid.set(1, 1, Cell::Body);
        grid.set(2, 1, Cell::Body);
        grid.set(1, 2, Cell::Body);
        grid.set(2, 2, Cell::Body);

        let rendered = render_to_string(&grid, &palette);
        assert!(rendered.contains('▀') || rendered.contains('▄'));
    }

    #[test]
    fn render_ends_with_reset() {
        let grid = Grid::new(4, 4);
        let palette = Palette {
            body: Rgb::new(200, 100, 100),
            outline: Rgb::new(50, 25, 25),
            eye: Rgb::new(240, 240, 240),
            pupil: Rgb::new(20, 20, 20),
        };

        let rendered = render_to_string(&grid, &palette);
        let trimmed = rendered.trim_end_matches('\n');
        assert!(
            trimmed.ends_with("\x1b[0m") || trimmed.chars().all(|c| c == ' ' || c == '\n'),
            "output should end with reset or be all spaces"
        );
    }
}
