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

    assert_eq!(
        squint_eyes,
        orig_eyes / 2,
        "squint should halve eye cell count"
    );
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
