use character::{Creature, CreatureConfig};

fn main() {
    let seed: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    println!("\n  seed={seed} — size = character row height\n");
    for size in [3, 4, 5, 8, 12, 16] {
        let creature = Creature::generate(&CreatureConfig {
            size,
            seed: seed.into(),
            ..Default::default()
        });
        let mode = if creature.grid.width > creature.grid.height { "quadrant" } else { "half-block" };
        println!("--- size={size} ({}x{} px, {mode}) ---", creature.grid.width, creature.grid.height);
        print!("{}", creature.render());
    }
}
