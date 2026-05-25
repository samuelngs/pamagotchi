use creature::{Creature, CreatureConfig};
use creature::render::render_row;

fn main() {
    let per_row = 5;
    let total = 15;
    let spacing = 2;

    println!("\n  Creature Gallery - seeds 0..{total}\n");

    for row_start in (0..total).step_by(per_row) {
        let row_end = (row_start + per_row as u64).min(total);

        let creatures: Vec<Creature> = (row_start..row_end)
            .map(|seed| {
                Creature::generate(&CreatureConfig {
                    seed: seed.into(),
                    ..Default::default()
                })
            })
            .collect();

        let refs: Vec<_> = creatures.iter().map(|c| (&c.grid, &c.palette)).collect();
        print!("{}", render_row(&refs, spacing));

        // Print seed labels
        print!("  ");
        for (i, seed) in (row_start..row_end).enumerate() {
            if i > 0 {
                for _ in 0..spacing {
                    print!(" ");
                }
            }
            print!("{:^16}", format!("seed={seed}"));
        }
        println!("\n");
    }
}
