use std::path::PathBuf;
use std::process::ExitCode;

use pinentry_bande::{PROMPT, ask, hand_over, in_bande_session, serve};
use rustix::process::{DumpableBehavior, set_dumpable_behavior};

const NAMESPACE: &str = "pinentry-bande";
const FALLBACKS: [&str; 6] = [
    "/usr/bin/pinentry-gnome3",
    "/usr/bin/pinentry-qt",
    "/usr/bin/pinentry-gtk-2",
    "/usr/bin/pinentry-fltk",
    "/usr/bin/pinentry-curses",
    "/usr/bin/pinentry-tty",
];

fn main() -> ExitCode {
    bande_log::init(NAMESPACE);
    if set_dumpable_behavior(DumpableBehavior::NotDumpable).is_err() {
        return ExitCode::FAILURE;
    }
    let mut arguments = Vec::new();
    let mut prompt = PathBuf::from(PROMPT);
    let mut given = std::env::args().skip(1);
    while let Some(argument) = given.next() {
        match argument.as_str() {
            "--prompt" => match given.next() {
                Some(program) => prompt = PathBuf::from(program),
                None => return ExitCode::FAILURE,
            },
            _ => arguments.push(argument),
        }
    }
    if !in_bande_session() || !prompt.exists() {
        return hand_over(&FALLBACKS, &arguments);
    }
    let stdin = std::io::stdin();
    match serve(stdin.lock(), std::io::stdout().lock(), |kind, texts| {
        ask(&prompt, kind, texts)
    }) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
