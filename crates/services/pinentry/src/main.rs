use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use pinentry_bande::{Answer, Kind, Texts, serve};
use rustix::process::{DumpableBehavior, set_dumpable_behavior};
use zeroize::Zeroizing;

const NAMESPACE: &str = "pinentry-bande";
const PROMPT: &str = "/usr/libexec/bande-secrets-prompt";
const LONGEST_ANSWER: u64 = 8192;
const FALLBACKS: [&str; 6] = [
    "/usr/bin/pinentry-gnome3",
    "/usr/bin/pinentry-qt",
    "/usr/bin/pinentry-gtk-2",
    "/usr/bin/pinentry-fltk",
    "/usr/bin/pinentry-curses",
    "/usr/bin/pinentry-tty",
];

fn in_bande_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some_and(|display| !display.is_empty())
        && bande_xdg::current_desktops()
            .iter()
            .any(|desktop| desktop == bande_xdg::CURRENT_DESKTOP)
}

fn single_line(text: &str) -> String {
    text.replace(['\r', '\n'], "\r").replace('\t', " ")
}

fn ask(prompt: &Path, kind: Kind, texts: &Texts) -> Answer {
    let spawned = Command::new(prompt)
        .arg(kind.flag())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            tracing::error!("cannot start {}: {error}", prompt.display());
            return Answer::Cancelled;
        }
    };
    let (Some(mut input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
        return Answer::Cancelled;
    };
    let mut header = String::new();
    for (key, value) in texts.lines() {
        header.push_str(&format!("{key}\t{}\n", single_line(value)));
    }
    header.push('\n');
    if let Err(error) = input
        .write_all(header.as_bytes())
        .and_then(|()| input.flush())
    {
        tracing::error!("cannot hand the request to the prompt: {error}");
        return Answer::Cancelled;
    }
    let mut line = Zeroizing::new(String::new());
    let read = BufReader::new(output)
        .take(LONGEST_ANSWER)
        .read_line(&mut line);
    if let Err(error) = input.write_all(b"done\n") {
        tracing::debug!("the prompt is gone: {error}");
    }
    drop(input);
    if let Err(error) = child.wait() {
        tracing::debug!("cannot wait for the prompt: {error}");
    }
    if let Err(error) = read {
        tracing::error!("cannot read the prompt's answer: {error}");
        return Answer::Cancelled;
    }
    let text = line.strip_suffix('\n').unwrap_or(&line);
    let (verb, secret) = text.split_once('\t').unwrap_or((text, ""));
    match (kind, verb) {
        (Kind::Passphrase, "allow") => Answer::Passphrase(Zeroizing::new(secret.to_owned())),
        (Kind::Confirm | Kind::Message, "allow") => Answer::Confirmed,
        (Kind::Confirm, "not-ok") => Answer::NotConfirmed,
        _ => Answer::Cancelled,
    }
}

fn fall_back(arguments: &[String]) -> ExitCode {
    for fallback in FALLBACKS {
        if Path::new(fallback).exists() {
            let error = Command::new(fallback).args(arguments).exec();
            tracing::error!("cannot start {fallback}: {error}");
        }
    }
    tracing::error!("no other pinentry to use outside a Bande session");
    ExitCode::FAILURE
}

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
        return fall_back(&arguments);
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
