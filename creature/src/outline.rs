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

            let adjacent_to_filled = neighbors.iter().any(|c| {
                matches!(c, Some(Cell::Body | Cell::Eye | Cell::Pupil))
            });

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
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    #[test]
    fn outline_surrounds_body() {
        let mut rng = StdRng::seed_from_u64(42);
        let mut grid = Grid::new(16, 16);
        grid.fill_body(&mut rng);
        apply_outline(&mut grid);

        // Every body cell should have no adjacent Empty cells
        // (they should all be Outline or Body now)
        for y in 0..16u32 {
            for x in 0..16u32 {
                if grid.get(x, y) == Some(Cell::Body) {
                    let neighbors = [
                        if x > 0 { grid.get(x - 1, y) } else { None },
                        grid.get(x + 1, y),
                        if y > 0 { grid.get(x, y - 1) } else { None },
                        grid.get(x, y + 1),
                    ];
                    for n in &neighbors {
                        if let Some(c) = n {
                            assert_ne!(
                                *c,
                                Cell::Empty,
                                "body cell at ({x},{y}) has adjacent empty"
                            );
                        }
                    }
                }
            }
        }
    }
}
