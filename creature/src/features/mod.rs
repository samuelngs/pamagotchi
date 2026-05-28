use rand::Rng;

use crate::grid::{Cell, Grid};

/// Place eyes in the upper region of the body.
///
/// Eyes are 2×2 empty blocks cut into the body (creating "holes" that show
/// the background). This works with single-color rendering — the dark
/// background peeks through the eye holes.
///
/// PRNG draws: 1 for count (if not specified), then index selection draws.
pub fn place_eyes(grid: &mut Grid, rng: &mut impl Rng, eye_count: Option<u8>) {
    let _count = eye_count.unwrap_or_else(|| rng.random_range(1..=2));

    let half = grid.width / 2;

    // Find body vertical bounds
    let top_body = (0..grid.height)
        .find(|&y| (0..grid.width).any(|x| grid.get(x, y) == Some(Cell::Body)))
        .unwrap_or(0);
    let bot_body = (0..grid.height)
        .rev()
        .find(|&y| (0..grid.width).any(|x| grid.get(x, y) == Some(Cell::Body)))
        .unwrap_or(grid.height);

    let body_height = bot_body - top_body;

    // Eyes in upper 45% of body
    let eye_y_end = top_body + (body_height * 45 / 100).max(4);

    // Collect candidates: 1×2 slit with strict padding on all sides.
    // Each candidate needs:
    //   - 2 body cells vertically (the eye itself)
    //   - 1+ body above, 1+ body below
    //   - 1+ body left, 1+ body right
    //   - at least 3px from mirror axis (so mirrored eyes don't touch)
    let max_x = half.saturating_sub(3);
    let mut candidates: Vec<(u32, u32)> = Vec::new();
    for y in (top_body + 3)..eye_y_end {
        for x in 2..=max_x {
            let eye_ok = (0..2).all(|dy| grid.get(x, y + dy) == Some(Cell::Body));
            let pad_above = (1..=2).all(|d| grid.get(x, y - d) == Some(Cell::Body));
            let pad_below = (2..=3).all(|d| grid.get(x, y + d) == Some(Cell::Body));
            let pad_left = x > 0 && grid.get(x - 1, y) == Some(Cell::Body);
            let pad_right = grid.get(x + 1, y) == Some(Cell::Body);

            if eye_ok && pad_above && pad_below && pad_left && pad_right {
                candidates.push((x, y));
            }
        }
    }

    if candidates.is_empty() {
        return;
    }

    let idx = rng.random_range(0..candidates.len());
    let (x, y) = candidates[idx];

    grid.set(x, y, Cell::Eye);
    grid.set(x, y + 1, Cell::Eye);

    let mx = grid.width - 1 - x;
    grid.set(mx, y, Cell::Eye);
    grid.set(mx, y + 1, Cell::Eye);
}

/// Place legs along the bottom edge of the body.
///
/// Finds the bottom row of body cells and extends uniform-width stubs
/// downward. Legs are always mirrored and evenly spaced.
///
/// PRNG draws: 1 for count (if not specified), 1 for offset jitter.
pub fn place_legs(grid: &mut Grid, rng: &mut impl Rng, leg_count: Option<u8>) {
    let half_count = leg_count.unwrap_or_else(|| rng.random_range(1..=2));

    // Find bottom row of body
    let bottom_y = match (0..grid.height)
        .rev()
        .find(|&y| (0..grid.width).any(|x| grid.get(x, y) == Some(Cell::Body)))
    {
        Some(y) => y,
        None => return,
    };

    // Find left-half body cells on bottom row
    let half = grid.width / 2;
    let mut body_xs: Vec<u32> = Vec::new();
    for x in 1..half {
        if grid.get(x, bottom_y) == Some(Cell::Body) {
            body_xs.push(x);
        }
    }

    if body_xs.is_empty() {
        return;
    }

    // Distribute legs evenly across the body width
    let leg_length = 2u32;
    let jitter: i32 = rng.random_range(-1..=1);

    for i in 0..half_count {
        // Evenly space legs across available body positions
        let frac = (i as f32 + 1.0) / (half_count as f32 + 1.0);
        let target_idx = ((frac * body_xs.len() as f32) as i32 + jitter)
            .clamp(0, body_xs.len() as i32 - 1) as usize;
        let x = body_xs[target_idx];
        let mx = grid.width - 1 - x;

        for dy in 1..=leg_length {
            let ly = bottom_y + dy;
            if ly < grid.height {
                grid.set(x, ly, Cell::Body);
                grid.set(mx, ly, Cell::Body);
            }
        }
    }
}

#[cfg(test)]
mod tests;
