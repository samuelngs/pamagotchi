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
                        assert_ne!(*c, Cell::Empty, "body cell at ({x},{y}) has adjacent empty");
                    }
                }
            }
        }
    }
}
