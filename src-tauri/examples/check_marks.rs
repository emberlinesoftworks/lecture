// runs the mark test from start to finish on a recording made by scripts/test-marks.sh
//
// usage: cargo run --example check_marks -- <model> <wav>

use std::path::Path;

use lecture_lib::{marks, transcriber};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: check_marks <model> <wav>");
        std::process::exit(1);
    }

    // no progress needed here
    let lines = transcriber::transcribe(Path::new(&args[1]), Path::new(&args[2]), std::sync::Arc::new(|_| {}))
        .expect("transcription failed");

    println!("\nTranscript:");
    for line in &lines {
        println!(
            "  {:>5.1}s  (quiet {:>4}ms before)  {}",
            line.start_ms as f64 / 1000.0,
            line.pause_before_ms,
            line.text
        );
    }
    println!();

    // press right after the sentence on the left, the mark should start at the one on the right
    let checks = [
        ("as waste", "Here's the important part"),
        ("each other", "Each phospholipid"),
    ];
    let mut failed = false;

    for (pressed_after, expected_start) in checks {
        let Some(index) = lines.iter().position(|line| line.text.to_lowercase().contains(pressed_after)) else {
            println!("FAIL  couldn't find \"{pressed_after}\" in the transcript");
            failed = true;
            continue;
        };

        // a real press lands in the pause after the sentence, before the next one starts
        let mark_ms = match lines.get(index + 1) {
            Some(next) => next.start_ms - 300,
            None => lines[index].end_ms + 800,
        };

        let range = &marks::marked_ranges(&lines, &[mark_ms as u64])[0];
        let first = &lines[range.first_line].text;
        let last = &lines[range.last_line].text;
        let passed = first.to_lowercase().starts_with(&expected_start.to_lowercase());

        println!("{}  mark after \"{pressed_after}\"", if passed { "PASS" } else { "FAIL" });
        println!("      covers: \"{first}\"");
        println!("          to: \"{last}\"");
        if !passed {
            println!("      expected it to start at \"{expected_start}...\"");
            failed = true;
        }
    }

    if failed {
        std::process::exit(1);
    }
}
