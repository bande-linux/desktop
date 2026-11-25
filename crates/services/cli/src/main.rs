use std::process::ExitCode;

use bande_intents::Manifests;

fn main() -> ExitCode {
    bande_log::init("bande");
    let words: Vec<String> = std::env::args().skip(1).collect();
    let answer = bande_cli::run(&Manifests::installed(), bande_bus::session, &words);
    match answer {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(text) => {
            eprintln!("{text}");
            ExitCode::FAILURE
        }
    }
}
