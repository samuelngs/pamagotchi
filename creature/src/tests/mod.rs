use super::*;
use std::hash::{Hash, Hasher};

impl Hash for Cell {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (*self as u8).hash(state);
    }
}

fn grid_hash(creature: &Creature) -> u64 {
    let mut hasher = DefaultHasher::new();
    creature.grid.width.hash(&mut hasher);
    creature.grid.height.hash(&mut hasher);
    for y in 0..creature.grid.height {
        for x in 0..creature.grid.width {
            creature.grid.get(x, y).hash(&mut hasher);
        }
    }
    creature.palette.body.r.hash(&mut hasher);
    creature.palette.body.g.hash(&mut hasher);
    creature.palette.body.b.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn determinism_same_seed() {
    let config = CreatureConfig {
        seed: 42.into(),
        ..Default::default()
    };
    let a = Creature::generate(&config);
    let b = Creature::generate(&config);

    assert_eq!(grid_hash(&a), grid_hash(&b));
}

#[test]
fn determinism_different_seed() {
    let a = Creature::generate(&CreatureConfig {
        seed: 1.into(),
        ..Default::default()
    });
    let b = Creature::generate(&CreatureConfig {
        seed: 2.into(),
        ..Default::default()
    });

    assert_ne!(grid_hash(&a), grid_hash(&b));
}

#[test]
fn render_stability() {
    let config = CreatureConfig {
        seed: 42.into(),
        ..Default::default()
    };
    let a = Creature::generate(&config);
    let b = Creature::generate(&config);

    assert_eq!(a.render(), b.render());
}

#[test]
fn golden_hash_seed_42() {
    let config = CreatureConfig {
        seed: 42.into(),
        ..Default::default()
    };
    let creature = Creature::generate(&config);
    let h = grid_hash(&creature);
    assert_eq!(
        h, 18020317725994607546,
        "golden hash changed — generation logic must have been modified"
    );
}

#[test]
fn symmetry_preserved_with_features() {
    let config = CreatureConfig {
        seed: 42.into(),
        eye_count: Some(1),
        leg_count: Some(2),
        ..Default::default()
    };
    let creature = Creature::generate(&config);

    for y in 0..creature.grid.height {
        for x in 0..creature.grid.width / 2 {
            let left = creature.grid.get(x, y);
            let right = creature.grid.get(creature.grid.width - 1 - x, y);

            // Eyes and pupils mirror, but pupil/eye might swap sides for visual effect
            let symmetric = match (left, right) {
                (Some(Cell::Eye), Some(Cell::Pupil)) | (Some(Cell::Pupil), Some(Cell::Eye)) => true,
                (a, b) => a == b,
            };

            assert!(symmetric, "asymmetry at ({x}, {y}): {left:?} vs {right:?}");
        }
    }
}

#[test]
fn body_connectivity() {
    let config = CreatureConfig {
        seed: 42.into(),
        ..Default::default()
    };
    let creature = Creature::generate(&config);

    // BFS from first body cell — all body cells should be reachable
    let w = creature.grid.width;
    let h = creature.grid.height;

    let is_filled = |c: Option<Cell>| matches!(c, Some(Cell::Body | Cell::Eye | Cell::Pupil));

    let mut start = None;
    let mut total_filled = 0u32;
    for y in 0..h {
        for x in 0..w {
            if is_filled(creature.grid.get(x, y)) {
                if start.is_none() {
                    start = Some((x, y));
                }
                total_filled += 1;
            }
        }
    }

    let (sx, sy) = start.expect("creature has no body cells");
    let mut visited = vec![false; (w * h) as usize];
    let mut stack = vec![(sx, sy)];
    let mut reached = 0u32;

    while let Some((x, y)) = stack.pop() {
        let idx = (y * w + x) as usize;
        if visited[idx] {
            continue;
        }
        visited[idx] = true;
        if !is_filled(creature.grid.get(x, y)) {
            continue;
        }
        reached += 1;

        if x > 0 {
            stack.push((x - 1, y));
        }
        if x + 1 < w {
            stack.push((x + 1, y));
        }
        if y > 0 {
            stack.push((x, y - 1));
        }
        if y + 1 < h {
            stack.push((x, y + 1));
        }
    }

    assert_eq!(reached, total_filled, "body is not fully connected");
}
