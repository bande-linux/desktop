use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use pinentry_bande::{Answer, Kind, PROMPT, Texts, ask, hand_over, in_bande_session};
use rustix::process::{DumpableBehavior, set_dumpable_behavior};

const NAMESPACE: &str = "bande-ssh-askpass";
const FALLBACKS: [&str; 3] = [
    "/usr/bin/ksshaskpass",
    "/usr/lib/openssh/gnome-ssh-askpass",
    "/usr/bin/lxqt-openssh-askpass",
];

fn kind() -> Kind {
    match std::env::var("SSH_ASKPASS_PROMPT").as_deref() {
        Ok("confirm") => Kind::Confirm,
        Ok("none") => Kind::Message,
        _ => Kind::Passphrase,
    }
}

fn texts(kind: Kind, message: &str) -> Texts {
    let message = message.trim();
    let (title, description) = match kind {
        Kind::Passphrase => (None, message),
        Kind::Confirm | Kind::Message => {
            let (first, rest) = message.split_once('\n').unwrap_or((message, ""));
            (Some(first.trim().to_owned()), rest.trim())
        }
    };
    Texts {
        title,
        description: Some(description.to_owned()).filter(|description| !description.is_empty()),
        ..Texts::default()
    }
}

fn answer(passphrase: &str) -> ExitCode {
    let mut output = std::io::stdout().lock();
    let written = output
        .write_all(passphrase.as_bytes())
        .and_then(|()| output.write_all(b"\n"))
        .and_then(|()| output.flush());
    match written {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("cannot hand the passphrase to ssh: {error}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    bande_log::init(NAMESPACE);
    if set_dumpable_behavior(DumpableBehavior::NotDumpable).is_err() {
        return ExitCode::FAILURE;
    }
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let prompt = Path::new(PROMPT);
    if !in_bande_session() || !prompt.exists() {
        return hand_over(&FALLBACKS, &arguments);
    }
    let kind = kind();
    let message = arguments.first().map_or("", String::as_str);
    match ask(prompt, kind, &texts(kind, message)) {
        Answer::Passphrase(passphrase) => answer(&passphrase),
        Answer::Confirmed => ExitCode::SUCCESS,
        Answer::NotConfirmed | Answer::Cancelled => ExitCode::FAILURE,
    }
}
