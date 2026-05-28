//! Procedural pixel-art creature generator.
//!
//! Generates small retro-style sprites from a seed, fully deterministic:
//! same seed + config = identical creature, every time.

pub mod animate;
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
    fn from(n: u64) -> Self {
        Seed::Number(n)
    }
}

impl From<&str> for Seed {
    fn from(s: &str) -> Self {
        Seed::Text(s.to_string())
    }
}

impl From<String> for Seed {
    fn from(s: String) -> Self {
        Seed::Text(s)
    }
}

const CANONICAL_W: u32 = 16;
const CANONICAL_H: u32 = 16;
const QUADRANT_THRESHOLD: u32 = 8;

/// Configuration for creature generation.
#[derive(Debug, Clone)]
pub struct CreatureConfig {
    /// Display size in character rows (default: 8, min: 3).
    /// Small sizes (< 8) use quadrant rendering with 2:1 pixel aspect.
    /// Large sizes (≥ 8) use half-block rendering with square pixels.
    pub size: u32,
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
            size: QUADRANT_THRESHOLD,
            seed: Seed::default(),
            palette: None,
            leg_count: None,
            eye_count: None,
        }
    }
}

fn fill_compact_holes(grid: &mut Grid) {
    let is_filled = |c: Option<Cell>| {
        matches!(
            c,
            Some(Cell::Body | Cell::Eye | Cell::Pupil | Cell::Outline)
        )
    };

    fn row_span(
        grid: &Grid,
        y: u32,
        is_filled: impl Fn(Option<Cell>) -> bool,
    ) -> Option<(u32, u32)> {
        let f = (0..grid.width).find(|&x| is_filled(grid.get(x, y)))?;
        let l = (0..grid.width).rfind(|&x| is_filled(grid.get(x, y)))?;
        Some((f, l))
    }

    // Bottom-to-top: widen narrow rows to taper smoothly from body below
    for y in (0..grid.height.saturating_sub(1)).rev() {
        if !((0..grid.width).any(|x| is_filled(grid.get(x, y)))) {
            continue;
        }
        if let Some((bf, bl)) = row_span(grid, y + 1, &is_filled) {
            let target_f = (bf + 1).min(grid.width - 1);
            let target_l = bl.saturating_sub(1);
            if target_f <= target_l {
                for x in target_f..=target_l {
                    if grid.get(x, y) == Some(Cell::Empty) {
                        grid.set(x, y, Cell::Body);
                    }
                }
            }
        }
    }

    // Fill remaining horizontal interior gaps per row
    for y in 0..grid.height {
        if let Some((f, l)) = row_span(grid, y, &is_filled) {
            for x in f..=l {
                if grid.get(x, y) == Some(Cell::Empty) {
                    grid.set(x, y, Cell::Body);
                }
            }
        }
    }
}

fn compress_eyes(grid: &mut Grid) {
    for x in 0..grid.width {
        let mut y = 0;
        while y < grid.height {
            if grid.get(x, y) == Some(Cell::Eye) {
                y += 1;
                while y < grid.height && grid.get(x, y) == Some(Cell::Eye) {
                    grid.set(x, y, Cell::Body);
                    y += 1;
                }
            } else {
                y += 1;
            }
        }
    }
}

/// A generated creature: grid data + color palette.
#[derive(Debug, Clone)]
pub struct Creature {
    canonical: Grid,
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

        let palette = config
            .palette
            .clone()
            .unwrap_or_else(|| Palette::generate(&mut rng));

        let mut grid = Grid::new(CANONICAL_W, CANONICAL_H);
        grid.fill_body(&mut rng);
        features::place_eyes(&mut grid, &mut rng, config.eye_count);
        features::place_legs(&mut grid, &mut rng, config.leg_count);

        let sz = config.size.max(3);
        let compact = sz < QUADRANT_THRESHOLD;
        let (pixel_w, pixel_h) = if compact {
            (4 * sz, 2 * sz)
        } else {
            (2 * sz, 2 * sz)
        };
        let mut display = grid.scale(pixel_w, pixel_h);

        if compact {
            fill_compact_holes(&mut display);
            compress_eyes(&mut display);
        }

        Self {
            canonical: grid,
            grid: display,
            palette,
        }
    }

    pub fn render(&self) -> String {
        if self.grid.width > self.grid.height {
            render::render_to_string_quadrant(&self.grid, &self.palette)
        } else {
            render::render_to_string(&self.grid, &self.palette)
        }
    }

    /// Print the rendered creature to stdout.
    pub fn print(&self) {
        print!("{}", self.render());
    }

    pub fn idle_frames(&self) -> Vec<animate::AnimationFrame> {
        let frames = animate::idle_frames(&self.canonical);
        let (w, h) = (self.grid.width, self.grid.height);
        let compact = w > h;
        frames
            .into_iter()
            .map(|f| {
                let mut grid = f.grid.scale(w, h);
                if compact {
                    fill_compact_holes(&mut grid);
                    compress_eyes(&mut grid);
                }
                animate::AnimationFrame {
                    grid,
                    duration_ms: f.duration_ms,
                }
            })
            .collect()
    }

    /// Get the raw cell at a grid position.
    pub fn cell_at(&self, x: u32, y: u32) -> Option<Cell> {
        self.grid.get(x, y)
    }
}

#[cfg(test)]
mod tests;
