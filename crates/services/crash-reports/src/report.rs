use std::fmt::Write as _;
use std::fs;

use jiff::Timestamp;
use jiff::tz::TimeZone;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    User(u32),
    System,
}

#[derive(Clone, Copy)]
pub enum Kind {
    Crash,
    NotResponding,
    Failed,
    KernelPanic,
    UnexpectedRestart,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Crash => "crash",
            Self::NotResponding => "not-responding",
            Self::Failed => "failed",
            Self::KernelPanic => "kernel-panic",
            Self::UnexpectedRestart => "unexpected-restart",
        }
    }
}

pub struct Report {
    owner: Owner,
    file_name: String,
    program: String,
    text: String,
}

impl Report {
    pub fn new(kind: Kind, owner: Owner, program: String, time: Timestamp, pid: &str) -> Self {
        let local = time.to_zoned(TimeZone::system());
        let safe: String = program
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let mut report = Self {
            owner,
            file_name: format!(
                "{}-{}-{safe}-{pid}.txt",
                local.strftime("%Y-%m-%d-%H%M%S"),
                kind.name(),
            ),
            program: program.clone(),
            text: String::new(),
        };
        report.detail("Kind", Some(kind.name()));
        report.detail("Program", Some(&program));
        report.detail(
            "Time",
            Some(&local.strftime("%Y-%m-%d %H:%M:%S %:z").to_string()),
        );
        report.detail("System", system_name().as_deref());
        report
    }

    pub fn detail(&mut self, label: &str, value: Option<&str>) {
        if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
            let _ = writeln!(self.text, "{label}: {value}");
        }
    }

    pub fn section(&mut self, title: &str, body: String) {
        if !body.trim().is_empty() {
            let _ = write!(self.text, "\n{title}:\n{body}");
            if !body.ends_with('\n') {
                self.text.push('\n');
            }
        }
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn owner(&self) -> Owner {
        self.owner
    }

    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

fn system_name() -> Option<String> {
    let release = fs::read_to_string("/etc/os-release").ok()?;
    let name = release
        .lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))?
        .trim_matches('"')
        .to_owned();
    let kernel = fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    Some(format!("{name}, kernel {}", kernel.trim()))
}

pub fn stack_trace(message: &str) -> String {
    let (_, trace) = message.split_once("\n\n").unwrap_or(("", message));
    trace.lines().map(demangled).collect::<Vec<_>>().join("\n")
}

fn demangled(line: &str) -> String {
    line.split(' ')
        .map(|word| {
            if word.starts_with("_ZN") || word.starts_with("_R") {
                format!("{:#}", rustc_demangle::demangle(word))
            } else {
                word.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
