use rand::Rng;

/// RGB color triplet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// Color palette for a creature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub body: Rgb,
    pub outline: Rgb,
    pub eye: Rgb,
    pub pupil: Rgb,
}

impl Palette {
    /// Generate a harmonious palette from the seeded PRNG.
    ///
    /// Draws exactly 3 values from `rng`: base hue, saturation jitter, value jitter.
    pub fn generate(rng: &mut impl Rng) -> Self {
        let hue: f32 = rng.random_range(0.0..360.0);
        let sat: f32 = rng.random_range(0.4..0.7);
        let val: f32 = rng.random_range(0.6..0.85);

        let body = hsv_to_rgb(hue, sat, val);
        let outline = hsv_to_rgb(hue, sat.min(1.0), (val * 0.35).min(1.0));
        let eye = Rgb::new(240, 240, 240);
        let pupil = Rgb::new(20, 20, 20);

        Self {
            body,
            outline,
            eye,
            pupil,
        }
    }
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Rgb {
    let c = v * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - ((h_prime % 2.0) - 1.0).abs());
    let (r1, g1, b1) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    Rgb::new(
        ((r1 + m) * 255.0) as u8,
        ((g1 + m) * 255.0) as u8,
        ((b1 + m) * 255.0) as u8,
    )
}

#[cfg(test)]
mod tests;
