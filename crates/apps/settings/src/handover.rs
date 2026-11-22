use std::process::{Command, Stdio};

use crate::follow::{Update, Updates, spawn};

const HELPER: &str = "/usr/libexec/bande-network-helper";
const PKEXEC: &str = "/usr/bin/pkexec";
const DISMISSED: [i32; 2] = [126, 127];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandOver {
    Confirm(String),
    Applying(String),
    Done(String),
    Failed(String),
}

impl HandOver {
    pub fn interface(&self) -> &str {
        match self {
            Self::Confirm(interface)
            | Self::Applying(interface)
            | Self::Done(interface)
            | Self::Failed(interface) => interface,
        }
    }
}

fn run(interface: String) -> Option<HandOver> {
    let output = Command::new(PKEXEC)
        .args([HELPER, "manage", &interface])
        .stdin(Stdio::null())
        .output();
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            tracing::warn!("cannot run {HELPER} for {interface}: {error}");
            return Some(HandOver::Failed(interface));
        }
    };
    if output
        .status
        .code()
        .is_some_and(|code| DISMISSED.contains(&code))
    {
        return None;
    }
    let report = String::from_utf8_lossy(&output.stdout);
    let errors = String::from_utf8_lossy(&output.stderr);
    if output.status.success() {
        tracing::info!("handed {interface} to NetworkManager:\n{}", report.trim());
        return Some(HandOver::Done(interface));
    }
    tracing::warn!(
        "did not hand {interface} to NetworkManager: {} {}",
        report.trim(),
        errors.trim()
    );
    Some(HandOver::Failed(interface))
}

pub fn hand_over(interface: String, updates: Updates) {
    spawn("hand-over", move || {
        updates.send(Update::HandOver(run(interface)));
    });
}
