use std::collections::{HashMap, HashSet};
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
    let mut answers: HashMap<(String, bool), String> = HashMap::new();
    let mut changes: HashMap<(String, String), usize> = HashMap::new();
    let (mut total, mut wrong) = (0_usize, 0_usize);
    for document in text.split('\u{c}') {
        let mut seen = HashSet::new();
        let mut sentence_start = true;
        for piece in document.split_inclusive(|character: char| !character.is_alphabetic()) {
            let word = piece.trim_end_matches(|character: char| !character.is_alphabetic());
            if !word.is_empty() {
                total += 1;
                if seen.insert(word.to_lowercase()) {
                    let key = (word.to_owned(), sentence_start);
                    let answer = answers
                        .entry(key)
                        .or_insert_with(|| engine.correction(word, sentence_start));
                    if !answer.is_empty() {
                        wrong += 1;
                        *changes
                            .entry((word.to_owned(), answer.clone()))
                            .or_default() += 1;
                    }
                }
                sentence_start = false;
            }
            if piece.ends_with(['.', '!', '?', '\n']) {
                sentence_start = true;
            }
        }
    }
    let mut listed: Vec<_> = changes.into_iter().collect();
    listed.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    for ((word, corrected), count) in listed.iter().take(15) {
        println!("{count}\t{word}\t{corrected}");
    }
    println!(
        "words {total}, changed {wrong}, per 1000 {:.3}",
        wrong as f64 * 1000.0 / total as f64
    );
    ExitCode::SUCCESS
}
