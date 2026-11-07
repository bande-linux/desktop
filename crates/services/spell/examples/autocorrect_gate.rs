use std::collections::HashMap;
use std::process::ExitCode;

use bande_spell::Engine;

fn main() -> ExitCode {
    let Some(corpus) = std::env::args().nth(1) else {
        eprintln!("usage: autocorrect_gate CORPUS");
        return ExitCode::FAILURE;
    };
    let Ok(text) = std::fs::read_to_string(&corpus) else {
        eprintln!("cannot read {corpus}");
        return ExitCode::FAILURE;
    };
    let Ok(mut engine) = Engine::open() else {
        return ExitCode::FAILURE;
    };
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut total = 0_usize;
    for word in text.split(|character: char| !character.is_alphabetic()) {
        if !word.is_empty() {
            total += 1;
            *counts.entry(word).or_default() += 1;
        }
    }
    let mut changes = Vec::new();
    for (word, count) in counts {
        let corrected = engine.correction(word);
        if !corrected.is_empty() {
            changes.push((count, word, corrected));
        }
    }
    changes.sort();
    let wrong: usize = changes.iter().map(|(count, _, _)| count).sum();
    for (count, word, corrected) in &changes {
        println!("{count}\t{word}\t{corrected}");
    }
    println!(
        "words {total}, changed {wrong}, per 1000 {:.3}",
        wrong as f64 * 1000.0 / total as f64
    );
    ExitCode::SUCCESS
}
