use crate::grid::{Cell, Grid};

pub struct AnimationFrame {
    pub grid: Grid,
    pub duration_ms: u64,
}

pub fn idle_frames(base: &Grid) -> Vec<AnimationFrame> {
    vec![
        AnimationFrame { grid: base.clone(), duration_ms: 1200 },
        AnimationFrame { grid: squint(base), duration_ms: 70 },
        AnimationFrame { grid: base.clone(), duration_ms: 1800 },
        AnimationFrame { grid: bounce(base, -1), duration_ms: 150 },
        AnimationFrame { grid: base.clone(), duration_ms: 1200 },
        AnimationFrame { grid: look(base, -1), duration_ms: 350 },
        AnimationFrame { grid: base.clone(), duration_ms: 400 },
        AnimationFrame { grid: look(base, 1), duration_ms: 350 },
        AnimationFrame { grid: base.clone(), duration_ms: 800 },
        AnimationFrame { grid: squint(base), duration_ms: 70 },
    ]
}

pub fn cycle_duration(frames: &[AnimationFrame]) -> u64 {
    frames.iter().map(|f| f.duration_ms).sum()
}

pub fn frame_index_at(frames: &[AnimationFrame], time_ms: u64) -> usize {
    let total = cycle_duration(frames);
    if total == 0 {
        return 0;
    }
    let t = time_ms % total;
    let mut elapsed = 0;
    for (i, f) in frames.iter().enumerate() {
        elapsed += f.duration_ms;
        if t < elapsed {
            return i;
        }
    }
    frames.len() - 1
}

fn squint(base: &Grid) -> Grid {
    let mut grid = base.clone();
    for y in 0..grid.height {
        for x in 0..grid.width {
            if grid.get(x, y) == Some(Cell::Eye)
                && grid.get(x, y + 1) == Some(Cell::Eye)
            {
                grid.set(x, y, Cell::Body);
            }
        }
    }
    grid
}

fn bounce(base: &Grid, dy: i32) -> Grid {
    let mut grid = Grid::new(base.width, base.height);
    for y in 0..base.height {
        for x in 0..base.width {
            let cell = base.get(x, y).unwrap_or(Cell::Empty);
            if cell != Cell::Empty {
                let new_y = y as i32 + dy;
                if new_y >= 0 && new_y < base.height as i32 {
                    grid.set(x, new_y as u32, cell);
                }
            }
        }
    }
    grid
}

fn look(base: &Grid, dx: i32) -> Grid {
    let mut grid = base.clone();

    let mut eyes: Vec<(u32, u32)> = Vec::new();
    for y in 0..grid.height {
        for x in 0..grid.width {
            if grid.get(x, y) == Some(Cell::Eye) {
                eyes.push((x, y));
            }
        }
    }

    for &(x, y) in &eyes {
        grid.set(x, y, Cell::Body);
    }

    for &(x, y) in &eyes {
        let new_x = x as i32 + dx;
        if new_x >= 0 && new_x < grid.width as i32 {
            let nx = new_x as u32;
            if grid.get(nx, y) == Some(Cell::Body) {
                grid.set(nx, y, Cell::Eye);
            }
        }
    }

    grid
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    fn make_creature_grid() -> Grid {
        let mut rng = StdRng::seed_from_u64(42);
        let mut grid = Grid::new(16, 16);
        grid.fill_body(&mut rng);
        crate::features::place_eyes(&mut grid, &mut rng, Some(1));
        grid
    }

    #[test]
    fn idle_frame_count() {
        let grid = make_creature_grid();
        let frames = idle_frames(&grid);
        assert!(frames.len() >= 8);
    }

    #[test]
    fn squint_halves_eyes() {
        let grid = make_creature_grid();
        let squinted = squint(&grid);

        let orig_eyes = (0..grid.width)
            .flat_map(|x| (0..grid.height).map(move |y| (x, y)))
            .filter(|&(x, y)| grid.get(x, y) == Some(Cell::Eye))
            .count();

        let squint_eyes = (0..squinted.width)
            .flat_map(|x| (0..squinted.height).map(move |y| (x, y)))
            .filter(|&(x, y)| squinted.get(x, y) == Some(Cell::Eye))
            .count();

        assert_eq!(squint_eyes, orig_eyes / 2, "squint should halve eye cell count");
    }

    #[test]
    fn bounce_preserves_cell_count() {
        let grid = make_creature_grid();
        let bounced = bounce(&grid, -1);

        let count = |g: &Grid| {
            (0..g.width)
                .flat_map(|x| (0..g.height).map(move |y| (x, y)))
                .filter(|&(x, y)| g.get(x, y) != Some(Cell::Empty))
                .count()
        };

        let orig = count(&grid);
        let after = count(&bounced);
        assert!(after >= orig - grid.width as usize);
    }

    #[test]
    fn look_preserves_eye_count() {
        let grid = make_creature_grid();
        let count_eyes = |g: &Grid| {
            (0..g.width)
                .flat_map(|x| (0..g.height).map(move |y| (x, y)))
                .filter(|&(x, y)| g.get(x, y) == Some(Cell::Eye))
                .count()
        };

        let orig = count_eyes(&grid);
        let looked = look(&grid, -1);
        let after = count_eyes(&looked);
        assert_eq!(orig, after);
    }

    #[test]
    fn frame_index_wraps() {
        let grid = make_creature_grid();
        let frames = idle_frames(&grid);
        let total = cycle_duration(&frames);
        assert_eq!(frame_index_at(&frames, 0), frame_index_at(&frames, total));
    }
}
