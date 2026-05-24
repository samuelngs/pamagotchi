use rand::Rng;

/// The state of a single cell in the creature grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    Empty,
    Body,
    Eye,
    Pupil,
    Outline,
}

/// A 2D grid of cells representing a creature sprite.
#[derive(Debug, Clone)]
pub struct Grid {
    pub width: u32,
    pub height: u32,
    cells: Vec<Cell>,
}

impl Grid {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            cells: vec![Cell::Empty; (width * height) as usize],
        }
    }

    pub fn get(&self, x: u32, y: u32) -> Option<Cell> {
        if x < self.width && y < self.height {
            Some(self.cells[(y * self.width + x) as usize])
        } else {
            None
        }
    }

    pub fn set(&mut self, x: u32, y: u32, cell: Cell) {
        if x < self.width && y < self.height {
            self.cells[(y * self.width + x) as usize] = cell;
        }
    }

    /// Fill body using stochastic distance-field technique (ZzSprite-inspired).
    ///
    /// For each pixel in the left half, a random radius is drawn and compared
    /// against the pixel's distance from center. Pixels near center are almost
    /// always filled (solid core), edge pixels are probabilistic (organic
    /// boundary), and far pixels are never filled. Then mirror horizontally.
    ///
    /// After fill: remove isolated pixels, keep largest connected component.
    ///
    /// Draw order: row-major over left half (y ascending, x ascending).
    pub fn fill_body(&mut self, rng: &mut impl Rng) {
        let half = self.width / 2;

        // Body region — leave room for legs below
        let y_start = 1;
        let y_end = self.height - 4;

        // Body center at the mirror axis, vertically centered in body region
        let cy = (y_start as f32 + y_end as f32) * 0.5;

        // Radii — slightly wider than tall for squat cute proportions
        let rx = half as f32 * 0.75;
        let ry = (y_end - y_start) as f32 * 0.55;

        for y in 0..self.height {
            for x in 0..half {
                if y < y_start || y > y_end {
                    let _: f32 = rng.random_range(0.0..1.0);
                    continue;
                }

                // Distance from center. x is measured from the mirror axis
                // (right edge of left half), so x=half-1 is center, x=0 is far left.
                let dx = (half - 1 - x) as f32 / rx;
                let dy = (y as f32 - cy) / ry;

                // Squircle-ish: use higher power for dx to flatten sides
                // This creates more rectangular bodies with rounded corners
                let dist_sq = dx.powi(4) + dy * dy;

                // Stochastic radius test: fill if random_threshold > distance²
                // Center pixels (dist≈0) almost always fill.
                // Edge pixels (dist≈1) sometimes fill, creating organic boundary.
                let random_r: f32 = rng.random_range(0.0..1.0);
                // Use pow(0.5) = sqrt to bias toward larger radii → denser blobs
                let threshold = random_r.sqrt();

                if threshold > dist_sq {
                    self.set(x, y, Cell::Body);
                    self.set(self.width - 1 - x, y, Cell::Body);
                }
            }
        }

        self.remove_isolated();
        self.flood_connect();
        // Smooth edges + fill holes iteratively for clean shapes
        for _ in 0..3 {
            self.fill_interior_holes();
            self.smooth_edges();
        }
        self.remove_isolated();
    }

    /// Remove isolated pixels (0 orthogonal body neighbors).
    fn remove_isolated(&mut self) {
        let w = self.width;
        let h = self.height;
        let mut to_remove = Vec::new();

        for y in 0..h {
            for x in 0..w {
                if self.get(x, y) != Some(Cell::Body) {
                    continue;
                }
                let neighbors = self.count_body_neighbors(x, y);
                if neighbors == 0 {
                    to_remove.push((x, y));
                }
            }
        }

        for (x, y) in to_remove {
            self.set(x, y, Cell::Empty);
            // Mirror
            self.set(self.width - 1 - x, y, Cell::Empty);
        }
    }

    /// Fill small interior holes (empty cells surrounded by body on 3+ sides).
    /// This removes visual noise from small gaps inside the body.
    fn fill_interior_holes(&mut self) {
        let w = self.width;
        let h = self.height;
        let half = w / 2;
        let mut to_fill = Vec::new();

        for y in 0..h {
            for x in 0..half {
                if self.get(x, y) != Some(Cell::Empty) {
                    continue;
                }
                let neighbors = self.count_body_neighbors(x, y);
                if neighbors >= 3 {
                    to_fill.push((x, y));
                }
            }
        }

        for (x, y) in to_fill {
            self.set(x, y, Cell::Body);
            self.set(self.width - 1 - x, y, Cell::Body);
        }
    }

    /// Smooth jagged edges: remove 1-pixel protrusions (body with only 1 neighbor)
    /// and fill concavities (empty with 3+ body neighbors).
    fn smooth_edges(&mut self) {
        let w = self.width;
        let h = self.height;
        let half = w / 2;

        // Remove protrusions
        let mut to_remove = Vec::new();
        for y in 0..h {
            for x in 0..half {
                if self.get(x, y) == Some(Cell::Body) {
                    let n = self.count_body_neighbors(x, y);
                    if n <= 1 {
                        to_remove.push((x, y));
                    }
                }
            }
        }
        for (x, y) in to_remove {
            self.set(x, y, Cell::Empty);
            self.set(self.width - 1 - x, y, Cell::Empty);
        }

        // Fill concavities
        let mut to_fill = Vec::new();
        for y in 0..h {
            for x in 0..half {
                if self.get(x, y) == Some(Cell::Empty) {
                    let n = self.count_body_neighbors(x, y);
                    if n >= 3 {
                        to_fill.push((x, y));
                    }
                }
            }
        }
        for (x, y) in to_fill {
            self.set(x, y, Cell::Body);
            self.set(self.width - 1 - x, y, Cell::Body);
        }
    }

    fn count_body_neighbors(&self, x: u32, y: u32) -> u32 {
        let mut count = 0;
        let is_body = |c: Option<Cell>| matches!(c, Some(Cell::Body));
        if x > 0 && is_body(self.get(x - 1, y)) { count += 1; }
        if is_body(self.get(x + 1, y)) { count += 1; }
        if y > 0 && is_body(self.get(x, y - 1)) { count += 1; }
        if is_body(self.get(x, y + 1)) { count += 1; }
        count
    }

    /// Keep only the largest connected component of body cells.
    fn flood_connect(&mut self) {
        let w = self.width as usize;
        let h = self.height as usize;
        let mut labels = vec![0u32; w * h];
        let mut label_id = 0u32;
        let mut component_sizes: Vec<(u32, u32)> = Vec::new();

        for y in 0..h {
            for x in 0..w {
                if self.cells[y * w + x] == Cell::Body && labels[y * w + x] == 0 {
                    label_id += 1;
                    let size = self.flood_fill_label(&mut labels, x, y, label_id);
                    component_sizes.push((label_id, size));
                }
            }
        }

        if let Some(&(best_label, _)) = component_sizes.iter().max_by_key(|(_, s)| s) {
            for y in 0..h {
                for x in 0..w {
                    if self.cells[y * w + x] == Cell::Body && labels[y * w + x] != best_label {
                        self.cells[y * w + x] = Cell::Empty;
                    }
                }
            }
        }
    }

    fn flood_fill_label(&self, labels: &mut [u32], start_x: usize, start_y: usize, label: u32) -> u32 {
        let w = self.width as usize;
        let h = self.height as usize;
        let mut stack = vec![(start_x, start_y)];
        let mut count = 0u32;

        while let Some((x, y)) = stack.pop() {
            let idx = y * w + x;
            if labels[idx] != 0 || self.cells[idx] != Cell::Body {
                continue;
            }
            labels[idx] = label;
            count += 1;

            if x > 0 { stack.push((x - 1, y)); }
            if x + 1 < w { stack.push((x + 1, y)); }
            if y > 0 { stack.push((x, y - 1)); }
            if y + 1 < h { stack.push((x, y + 1)); }
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    #[test]
    fn symmetry() {
        let mut rng = StdRng::seed_from_u64(42);
        let mut grid = Grid::new(16, 16);
        grid.fill_body(&mut rng);

        for y in 0..16 {
            for x in 0..8 {
                assert_eq!(
                    grid.get(x, y),
                    grid.get(15 - x, y),
                    "asymmetry at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn has_body_cells() {
        let mut rng = StdRng::seed_from_u64(123);
        let mut grid = Grid::new(16, 16);
        grid.fill_body(&mut rng);

        let body_count = (0..16)
            .flat_map(|y| (0..16).map(move |x| (x, y)))
            .filter(|&(x, y)| grid.get(x, y) == Some(Cell::Body))
            .count();

        assert!(body_count > 20, "too few body cells: {body_count}");
    }

    #[test]
    fn no_isolated_pixels() {
        let mut rng = StdRng::seed_from_u64(42);
        let mut grid = Grid::new(16, 16);
        grid.fill_body(&mut rng);

        for y in 0..16u32 {
            for x in 0..16u32 {
                if grid.get(x, y) == Some(Cell::Body) {
                    let n = grid.count_body_neighbors(x, y);
                    assert!(n > 0, "isolated body pixel at ({x}, {y})");
                }
            }
        }
    }
}
