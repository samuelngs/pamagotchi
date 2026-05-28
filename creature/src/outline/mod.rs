use crate::grid::{Cell, Grid};

/// Add outline cells around body cells.
///
/// Any `Empty` cell orthogonally adjacent to a `Body`, `Eye`, or `Pupil` cell
/// becomes `Outline`. Iteration is row-major (deterministic, no PRNG needed).
pub fn apply_outline(grid: &mut Grid) {
    let w = grid.width;
    let h = grid.height;

    // Collect positions to outline first (avoid mutating while scanning)
    let mut outline_positions: Vec<(u32, u32)> = Vec::new();

    for y in 0..h {
        for x in 0..w {
            if grid.get(x, y) != Some(Cell::Empty) {
                continue;
            }

            let neighbors = [
                if x > 0 { grid.get(x - 1, y) } else { None },
                grid.get(x + 1, y),
                if y > 0 { grid.get(x, y - 1) } else { None },
                grid.get(x, y + 1),
            ];

            let adjacent_to_filled = neighbors
                .iter()
                .any(|c| matches!(c, Some(Cell::Body | Cell::Eye | Cell::Pupil)));

            if adjacent_to_filled {
                outline_positions.push((x, y));
            }
        }
    }

    for (x, y) in outline_positions {
        grid.set(x, y, Cell::Outline);
    }
}

#[cfg(test)]
mod tests;
