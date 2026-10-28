use std::fmt;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

use crate::entry::DesktopEntry;

pub const TERMINAL: &str = "unbound-term";
pub const ACTIVATION_TOKEN: &str = "XDG_ACTIVATION_TOKEN";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    UnterminatedQuote,
    ReservedCharacter(char),
    Empty,
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnterminatedQuote => write!(f, "the Exec key has an unterminated quote"),
            Self::ReservedCharacter(character) => {
                write!(
                    f,
                    "the Exec key has an unquoted reserved character {character:?}"
                )
            }
            Self::Empty => write!(f, "the Exec key names no program"),
        }
    }
}

impl std::error::Error for ExecError {}

const RESERVED: &[char] = &[
    '\t', '\n', '\'', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(', ')', '`',
];

fn split(exec: &str) -> Result<Vec<(String, bool)>, ExecError> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quoted_argument = false;
    let mut started = false;
    let mut chars = exec.chars();
    while let Some(character) = chars.next() {
        match character {
            ' ' => {
                if started {
                    arguments.push((std::mem::take(&mut current), quoted_argument));
                    started = false;
                    quoted_argument = false;
                }
            }
            '"' => {
                started = true;
                quoted_argument = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(escaped @ ('"' | '`' | '$' | '\\')) => current.push(escaped),
                            Some(other) => {
                                current.push('\\');
                                current.push(other);
                            }
                            None => return Err(ExecError::UnterminatedQuote),
                        },
                        Some(other) => current.push(other),
                        None => return Err(ExecError::UnterminatedQuote),
                    }
                }
            }
            reserved if RESERVED.contains(&reserved) => {
                return Err(ExecError::ReservedCharacter(reserved));
            }
            other => {
                started = true;
                current.push(other);
            }
        }
    }
    if started {
        arguments.push((current, quoted_argument));
    }
    Ok(arguments)
}

impl DesktopEntry {
    pub fn command(&self) -> Result<Vec<String>, ExecError> {
        let mut command = Vec::new();
        for (argument, quoted) in split(&self.exec)? {
            if quoted {
                command.push(argument);
                continue;
            }
            match argument.as_str() {
                "%f" | "%F" | "%u" | "%U" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => {}
                "%i" => {
                    if let Some(icon) = &self.icon {
                        command.push("--icon".to_owned());
                        command.push(icon.clone());
                    }
                }
                _ => {
                    let mut expanded = String::new();
                    let mut chars = argument.chars();
                    while let Some(character) = chars.next() {
                        if character != '%' {
                            expanded.push(character);
                            continue;
                        }
                        match chars.next() {
                            Some('%') => expanded.push('%'),
                            Some('c') => expanded.push_str(&self.name),
                            Some('k') => expanded.push_str(&self.path.to_string_lossy()),
                            _ => {}
                        }
                    }
                    command.push(expanded);
                }
            }
        }
        if command.is_empty() {
            return Err(ExecError::Empty);
        }
        if self.terminal {
            command.splice(0..0, [TERMINAL.to_owned(), "-e".to_owned()]);
        }
        Ok(command)
    }
}

#[derive(Debug)]
pub enum LaunchError {
    Exec(ExecError),
    Spawn(io::Error),
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exec(error) => write!(f, "{error}"),
            Self::Spawn(error) => write!(f, "cannot start the app: {error}"),
        }
    }
}

impl std::error::Error for LaunchError {}

pub fn launch(entry: &DesktopEntry, activation_token: Option<&str>) -> Result<u32, LaunchError> {
    let command = entry.command().map_err(LaunchError::Exec)?;
    spawn_detached(
        &command,
        entry.working_directory.as_deref(),
        activation_token,
    )
    .map_err(LaunchError::Spawn)
}

pub fn spawn_detached(
    command: &[String],
    working_directory: Option<&Path>,
    activation_token: Option<&str>,
) -> io::Result<u32> {
    let (program, arguments) = command
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no program"))?;
    let mut process = Command::new(program);
    process
        .args(arguments)
        .stdin(Stdio::null())
        .env_remove(ACTIVATION_TOKEN);
    if let Some(token) = activation_token {
        process.env(ACTIVATION_TOKEN, token);
    }
    if let Some(directory) = working_directory {
        process.current_dir(directory);
    }
    unsafe {
        process.pre_exec(|| {
            rustix::process::setsid()
                .map(|_| ())
                .map_err(io::Error::from)
        });
    }
    let mut child = process.spawn()?;
    let pid = child.id();
    thread::spawn(move || child.wait());
    Ok(pid)
}
