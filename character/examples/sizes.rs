use character::{Creature, CreatureConfig};
use character::render::render_to_string;

fn main() {
    let seed: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    println!("\n  seed={seed} at different sizes\n");
    for size in [8, 12, 16, 24, 32] {
        let creature = Creature::generate(&CreatureConfig {
            width: size,
            height: size,
            seed: seed.into(),
            ..Default::default()
        });
        println!("--- {size}x{size} ---");
        print!("{}", render_to_string(&creature.grid, &creature.palette));
    }
}
