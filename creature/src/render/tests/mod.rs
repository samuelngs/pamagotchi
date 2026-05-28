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
