use std::path::PathBuf;
use std::process::{Command, Stdio};

use bande_network::ifupdown::Refusal;

use crate::follow::{Update, Updates, spawn};

const HELPER: &str = "/usr/libexec/bande-network-helper";
const PKEXEC: &str = "/usr/bin/pkexec";
const REFUSED: i32 = 3;
const DISMISSED: [i32; 2] = [126, 127];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changed {
    pub file: PathBuf,
    pub backup: Option<PathBuf>,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandOver {
    Asking(String),
    Preview(String, Vec<Changed>),
    Applying(String),
    Done(String, Vec<Changed>, String),
    Refused(String, String),
    Failed(String, String),
}

impl HandOver {
    pub fn interface(&self) -> &str {
        match self {
            Self::Asking(interface)
            | Self::Preview(interface, _)
            | Self::Applying(interface)
            | Self::Done(interface, ..)
            | Self::Refused(interface, _)
            | Self::Failed(interface, _) => interface,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandOverCommand {
    Preview(String),
    Apply(String),
}

fn parse(output: &str) -> (Vec<Changed>, Option<String>, Option<String>) {
    let (mut changed, mut connection, mut refused) = (Vec::<Changed>::new(), None, None);
    for line in output.lines() {
        let (kind, value) = line.split_once(' ').unwrap_or((line, ""));
        match kind {
            "file" => changed.push(Changed {
                file: PathBuf::from(value),
                backup: None,
                lines: Vec::new(),
            }),
            "backup" => {
                if let Some(last) = changed.last_mut() {
                    last.backup = Some(PathBuf::from(value));
                }
            }
            "line" => {
                if let Some(last) = changed.last_mut() {
                    last.lines.push(value.to_owned());
                }
            }
            "connection" => connection = Some(value.to_owned()),
            "refused" => {
                refused = Some(
                    Refusal::from_code(value).map_or_else(|| value.to_owned(), |r| r.to_string()),
                )
            }
            _ => {}
        }
    }
    (changed, connection, refused)
}

fn run(command: &HandOverCommand) -> Option<HandOver> {
    let (verb, interface) = match command {
        HandOverCommand::Preview(interface) => ("preview", interface),
        HandOverCommand::Apply(interface) => ("manage", interface),
    };
    let output = Command::new(PKEXEC)
        .args([HELPER, verb, interface])
        .stdin(Stdio::null())
        .output();
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            return Some(HandOver::Failed(interface.clone(), error.to_string()));
        }
    };
    let (changed, connection, refused) = parse(&String::from_utf8_lossy(&output.stdout));
    let code = output.status.code();
    if code.is_some_and(|code| DISMISSED.contains(&code)) {
        return None;
    }
    if let (Some(REFUSED), Some(reason)) = (code, refused) {
        return Some(HandOver::Refused(interface.clone(), reason));
    }
    if !output.status.success() {
        let reason = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Some(HandOver::Failed(interface.clone(), reason));
    }
    Some(match (command, connection) {
        (HandOverCommand::Apply(_), Some(connection)) => {
            HandOver::Done(interface.clone(), changed, connection)
        }
        _ => HandOver::Preview(interface.clone(), changed),
    })
}

pub fn hand_over(command: HandOverCommand, updates: Updates) {
    spawn("hand-over", move || {
        updates.send(Update::HandOver(run(&command)));
    });
}
