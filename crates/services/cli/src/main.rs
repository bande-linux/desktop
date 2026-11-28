use std::io::{self, Write};
use std::process::ExitCode;

use bande_intents::Manifests;

fn main() -> ExitCode {
    bande_log::init("bande");
    let words: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|word| word.to_string_lossy().into_owned())
        .collect();
    let written = match bande_cli::run(&Manifests::installed(), bande_bus::session, &words) {
        Ok(text) => writeln!(io::stdout(), "{text}").map(|()| ExitCode::SUCCESS),
        Err(text) => writeln!(io::stderr(), "{text}").map(|()| ExitCode::FAILURE),
    };
    written.unwrap_or_else(|error| {
        if error.kind() != io::ErrorKind::BrokenPipe {
            tracing::error!("cannot print the answer: {error}");
        }
        ExitCode::FAILURE
    })
}
