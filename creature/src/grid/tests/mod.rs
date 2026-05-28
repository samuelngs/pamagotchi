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
