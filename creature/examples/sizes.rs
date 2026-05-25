use std::io::{self, Write};
use std::thread;
use std::time::{Duration, Instant};

use creature::animate::{frame_index_at, AnimationFrame};
use creature::render::{render_to_string, render_to_string_quadrant};
use creature::{Creature, CreatureConfig};

fn main() {
    let seed: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let animate = std::env::args().any(|a| a == "--animate" || a == "-a");

    let sizes: Vec<u32> = vec![3, 4, 5, 8, 12, 16];
    let creatures: Vec<Creature> = sizes
        .iter()
        .map(|&size| {
            Creature::generate(&CreatureConfig {
                size,
                seed: seed.into(),
                ..Default::default()
            })
        })
        .collect();

    if !animate {
        println!("\n  seed={seed} — size = creature row height\n");
        for (i, creature) in creatures.iter().enumerate() {
            let size = sizes[i];
            let mode = if creature.grid.width > creature.grid.height {
                "quadrant"
            } else {
                "half-block"
            };
            println!(
                "--- size={size} ({}x{} px, {mode}) ---",
                creature.grid.width, creature.grid.height
            );
            print!("{}", creature.render());
        }
        return;
    }

    let all_frames: Vec<Vec<AnimationFrame>> =
        creatures.iter().map(|c| c.idle_frames()).collect();

    let render_frame = |frames: &[Vec<AnimationFrame>],
                        creatures: &[Creature],
                        elapsed: u64|
     -> String {
        let mut out = format!("\n  seed={seed} — animated sizes  (ctrl+c to quit)\n\n");
        for (i, (flist, creature)) in frames.iter().zip(creatures.iter()).enumerate() {
            let fi = frame_index_at(flist, elapsed);
            let grid = &flist[fi].grid;
            let size = sizes[i];
            let mode = if grid.width > grid.height {
                "quadrant"
            } else {
                "half-block"
            };
            out.push_str(&format!(
                "--- size={size} ({}x{} px, {mode}) ---\n",
                grid.width, grid.height
            ));
            if grid.width > grid.height {
                out.push_str(&render_to_string_quadrant(grid, &creature.palette));
            } else {
                out.push_str(&render_to_string(grid, &creature.palette));
            }
        }
        out
    };

    print!("\x1b[?25l\x1b[2J\x1b[H");
    io::stdout().flush().unwrap();

    ctrlc::set_handler(move || {
        print!("\x1b[?25h");
        io::stdout().flush().ok();
        std::process::exit(0);
    })
    .ok();

    let start = Instant::now();
    loop {
        let elapsed = start.elapsed().as_millis() as u64;
        let out = render_frame(&all_frames, &creatures, elapsed);
        print!("\x1b[H");
        for line in out.lines() {
            print!("{}\x1b[K\n", line);
        }
        io::stdout().flush().unwrap();
        thread::sleep(Duration::from_millis(30));
    }
}
