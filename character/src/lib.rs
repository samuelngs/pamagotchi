//! Procedural pixel-art creature generator.
//!
//! Generates small retro-style sprites from a seed, fully deterministic:
//! same seed + config = identical creature, every time.

pub mod features;
pub mod grid;
pub mod outline;
pub mod palette;
pub mod render;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use grid::{Cell, Grid};
use palette::Palette;
use rand::SeedableRng;
use rand::rngs::StdRng;

#[derive(Debug, Clone)]
pub enum Seed {
    Number(u64),
    Text(String),
}

impl Seed {
    pub fn to_u64(&self) -> u64 {
        match self {
            Seed::Number(n) => *n,
            Seed::Text(s) => {
                let mut h = DefaultHasher::new();
                s.hash(&mut h);
                h.finish()
            }
        }
    }
}

impl Default for Seed {
    fn default() -> Self {
        Seed::Number(0)
    }
}

impl From<u64> for Seed {
    fn from(n: u64) -> Self { Seed::Number(n) }
}

impl From<&str> for Seed {
    fn from(s: &str) -> Self { Seed::Text(s.to_string()) }
}

impl From<String> for Seed {
    fn from(s: String) -> Self { Seed::Text(s) }
}

/// Configuration for creature generation.
#[derive(Debug, Clone)]
pub struct CreatureConfig {
    /// Grid width in pixels (default: 16).
    pub width: u32,
    /// Grid height in pixels (default: 16).
    pub height: u32,
    /// Sole source of randomness — determines the creature entirely.
    pub seed: Seed,
    /// Optional fixed palette. If `None`, one is generated from the seed.
    pub palette: Option<Palette>,
    /// Number of legs (1–2 pairs, mirrored). `None` = random from seed.
    pub leg_count: Option<u8>,
    /// Number of eyes (mirrored). `None` = random from seed.
    pub eye_count: Option<u8>,
}

impl Default for CreatureConfig {
    fn default() -> Self {
        Self {
            width: 16,
            height: 16,
            seed: Seed::default(),
            palette: None,
            leg_count: None,
            eye_count: None,
        }
    }
}

/// A generated creature: grid data + color palette.
#[derive(Debug, Clone)]
pub struct Creature {
    pub grid: Grid,
    pub palette: Palette,
}

impl Creature {
    /// Generate a creature deterministically from the given config.
    ///
    /// PRNG draw order (part of the determinism contract):
    /// 1. Palette (3 draws if no fixed palette)
    /// 2. Body fill (width/2 * height draws)
    /// 3. Eyes (1 draw for count if not specified, then index draws)
    /// 4. Legs (1 draw for count if not specified, then index draws)
    /// 5. Outline pass (no draws — purely deterministic from grid state)
    pub fn generate(config: &CreatureConfig) -> Self {
        let mut rng = StdRng::seed_from_u64(config.seed.to_u64());

        // 1. Palette
        let palette = config
            .palette
            .clone()
            .unwrap_or_else(|| Palette::generate(&mut rng));

        // 2. Body fill
        let mut grid = Grid::new(config.width, config.height);
        grid.fill_body(&mut rng);

        // 3. Eyes
        features::place_eyes(&mut grid, &mut rng, config.eye_count);

        // 4. Legs
        features::place_legs(&mut grid, &mut rng, config.leg_count);

        Self { grid, palette }
    }

    /// Render to an ANSI truecolor string using half-block encoding.
    pub fn render(&self) -> String {
        render::render_to_string(&self.grid, &self.palette)
    }

    /// Print the rendered creature to stdout.
    pub fn print(&self) {
        print!("{}", self.render());
    }

    /// Get the raw cell at a grid position.
    pub fn cell_at(&self, x: u32, y: u32) -> Option<Cell> {
        self.grid.get(x, y)
    }
}

#[cfg(test)]
mod tests {
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
        assert_eq!(h, 18020317725994607546, "golden hash changed — generation logic must have been modified");
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
                    (Some(Cell::Eye), Some(Cell::Pupil))
                    | (Some(Cell::Pupil), Some(Cell::Eye)) => true,
                    (a, b) => a == b,
                };

                assert!(
                    symmetric,
                    "asymmetry at ({x}, {y}): {left:?} vs {right:?}"
                );
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

        let is_filled = |c: Option<Cell>| {
            matches!(c, Some(Cell::Body | Cell::Eye | Cell::Pupil))
        };

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

            if x > 0 { stack.push((x - 1, y)); }
            if x + 1 < w { stack.push((x + 1, y)); }
            if y > 0 { stack.push((x, y - 1)); }
            if y + 1 < h { stack.push((x, y + 1)); }
        }

        assert_eq!(reached, total_filled, "body is not fully connected");
    }
}
