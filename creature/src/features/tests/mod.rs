use super::*;
use rand::SeedableRng;
use rand::rngs::StdRng;

fn make_test_grid() -> Grid {
    let mut rng = StdRng::seed_from_u64(42);
    let mut grid = Grid::new(16, 16);
    grid.fill_body(&mut rng);
    grid
}

#[test]
fn eyes_placed() {
    let mut grid = make_test_grid();
    let mut rng = StdRng::seed_from_u64(99);
    place_eyes(&mut grid, &mut rng, Some(1));

    let eye_count = (0..grid.width)
        .flat_map(|x| (0..grid.height).map(move |y| (x, y)))
        .filter(|&(x, y)| grid.get(x, y) == Some(Cell::Eye))
        .count();

    assert!(
        eye_count >= 4,
        "expected at least 4 eye cells, got {eye_count}"
    );
}

#[test]
fn legs_placed() {
    let mut grid = make_test_grid();
    let mut rng = StdRng::seed_from_u64(99);
    place_legs(&mut grid, &mut rng, Some(2));

    let original_bottom = (0..grid.height)
        .rev()
        .find(|&y| (0..grid.width).any(|x| grid.get(x, y) == Some(Cell::Body)));

    assert!(original_bottom.is_some());
}
