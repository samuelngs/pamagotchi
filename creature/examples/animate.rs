use std::io::{self, Write};
use std::thread;
use std::time::{Duration, Instant};

use creature::animate::{cycle_duration, frame_index_at};
use creature::render::{render_row, render_to_string};
use creature::{Creature, CreatureConfig};

fn main() {
    let single_seed: Option<u64> = std::env::args().nth(1).and_then(|s| s.parse().ok());

    print!("\x1b[?25l\x1b[2J\x1b[H");
    io::stdout().flush().unwrap();

    ctrlc::set_handler(move || {
        print!("\x1b[?25h");
        io::stdout().flush().ok();
        std::process::exit(0);
    })
    .ok();

    if let Some(seed) = single_seed {
        animate_single(seed);
    } else {
        animate_gallery();
    }
}

fn animate_single(seed: u64) {
    let creature = Creature::generate(&CreatureConfig {
        seed: seed.into(),
        ..Default::default()
    });

    let frames = creature.idle_frames();
    let rendered: Vec<String> = frames
        .iter()
        .map(|f| render_to_string(&f.grid, &creature.palette))
        .collect();

    let header = format!("  seed={seed}  (ctrl+c to quit)\n");
    let line_count = rendered[0].lines().count() + header.lines().count() + 1;
    let start = Instant::now();

    println!("{}", header);
    print!("{}", rendered[0]);
    io::stdout().flush().unwrap();

    loop {
        let elapsed = start.elapsed().as_millis() as u64;
        let fi = frame_index_at(&frames, elapsed);
        print!("\x1b[{}A", line_count);
        println!("{}", header);
        print!("{}", rendered[fi]);
        io::stdout().flush().unwrap();
        thread::sleep(Duration::from_millis(30));
    }
}

fn animate_gallery() {
    let per_row = 5usize;
    let total = 15usize;
    let spacing = 2u32;

    let creatures: Vec<Creature> = (0..total)
        .map(|seed| {
            Creature::generate(&CreatureConfig {
                seed: (seed as u64).into(),
                ..Default::default()
            })
        })
        .collect();

    let all_frames: Vec<Vec<_>> = creatures.iter().map(|c| c.idle_frames()).collect();
    let total_cycle = cycle_duration(&all_frames[0]);

    let offsets: Vec<u64> = (0..total)
        .map(|i| {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            i.hash(&mut h);
            h.finish() % total_cycle
        })
        .collect();

    // Measure line count from a single render
    let mut sample = String::from("\n  Creature Gallery  (ctrl+c to quit)\n\n");
    for row_start in (0..total).step_by(per_row) {
        let row_end = (row_start + per_row).min(total);
        let refs: Vec<_> = (row_start..row_end)
            .map(|i| (&all_frames[i][0].grid, &creatures[i].palette))
            .collect();
        sample.push_str(&render_row(&refs, spacing));
        sample.push_str("  ");
        for (j, idx) in (row_start..row_end).enumerate() {
            if j > 0 {
                for _ in 0..spacing {
                    sample.push(' ');
                }
            }
            sample.push_str(&format!("{:^16}", format!("seed={idx}")));
        }
        sample.push_str("\n\n");
    }
    let line_count = sample.lines().count();

    print!("{}", sample);
    io::stdout().flush().unwrap();

    let start = Instant::now();

    loop {
        let elapsed = start.elapsed().as_millis() as u64;

        let mut out = String::from("\n  Creature Gallery  (ctrl+c to quit)\n\n");
        for row_start in (0..total).step_by(per_row) {
            let row_end = (row_start + per_row).min(total);
            let refs: Vec<_> = (row_start..row_end)
                .map(|i| {
                    let fi = frame_index_at(&all_frames[i], elapsed + offsets[i]);
                    (&all_frames[i][fi].grid, &creatures[i].palette)
                })
                .collect();
            out.push_str(&render_row(&refs, spacing));
            out.push_str("  ");
            for (j, idx) in (row_start..row_end).enumerate() {
                if j > 0 {
                    for _ in 0..spacing {
                        out.push(' ');
                    }
                }
                out.push_str(&format!("{:^16}", format!("seed={idx}")));
            }
            out.push_str("\n\n");
        }

        print!("\x1b[{}A", line_count);
        print!("{}", out);
        io::stdout().flush().unwrap();

        thread::sleep(Duration::from_millis(30));
    }
}
