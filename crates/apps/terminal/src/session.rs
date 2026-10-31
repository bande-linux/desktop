use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event as TermEvent, OnResize};
use alacritty_terminal::tty::{self, Options, Pty, Shell};
use zbus::blocking::Connection;

use crate::L10N;

use crate::process::{self, Report, Reports};
use crate::scope;
use crate::terminal::{GridSize, Terminal};

const READ_BUFFER: usize = 65536;
const READS_PER_WAKE: usize = 16;
const TITLE_SEPARATOR: &str = " — ";
const CALM_SCREEN: &str = "\x1b[?1049l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[0m\x1b[?25l";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Copy(String),
    Bell,
    Finished {
        command: String,
        elapsed: Duration,
        code: Option<i32>,
    },
    Notify {
        title: String,
        body: String,
    },
}

pub struct Session {
    pub terminal: Terminal,
    pty: Pty,
    shell: u32,
    output: Vec<u8>,
    reports: Reports,
    reported_directory: Option<PathBuf>,
    program_title: Option<String>,
    foreground: Option<(u32, String)>,
    command: Option<(Instant, String)>,
    notices: Vec<Notice>,
    bus: Option<Connection>,
    ending: Option<UnixStream>,
    hung_up: bool,
    out_of_memory: bool,
    exited: bool,
}

impl Session {
    pub fn spawn(
        terminal: Terminal,
        shell: Option<Shell>,
        directory: Option<PathBuf>,
        bus: Option<Connection>,
    ) -> io::Result<Self> {
        let options = Options {
            shell,
            working_directory: directory,
            drain_on_exit: true,
            env: HashMap::from([
                ("TERM".to_owned(), "xterm-256color".to_owned()),
                ("COLORTERM".to_owned(), "truecolor".to_owned()),
                ("TERM_PROGRAM".to_owned(), "unbound-term".to_owned()),
            ]),
        };
        let pty = tty::new(&options, terminal.window_size(), 0)?;
        let shell = pty.child().id();
        let ending = bus.clone().and_then(|bus| {
            scope::follow(bus, shell)
                .inspect_err(|error| tracing::warn!("cannot follow the shell {shell}: {error}"))
                .ok()
        });
        let mut session = Self {
            terminal,
            pty,
            shell,
            output: Vec::new(),
            reports: Reports::new(process::host_name()),
            reported_directory: None,
            program_title: None,
            foreground: None,
            command: None,
            notices: Vec::new(),
            bus,
            ending,
            hung_up: false,
            out_of_memory: false,
            exited: false,
        };
        session.refresh_foreground();
        Ok(session)
    }

    pub fn id(&self) -> u32 {
        self.shell
    }

    pub fn bus(&self) -> Option<Connection> {
        self.bus.clone()
    }

    pub fn fd(&self) -> Option<BorrowedFd<'_>> {
        if self.exited || self.out_of_memory {
            None
        } else if self.hung_up {
            self.ending.as_ref().map(AsFd::as_fd)
        } else {
            Some(self.pty.file().as_fd())
        }
    }

    pub fn exited(&self) -> bool {
        self.exited
    }

    fn hang_up(&mut self) {
        self.hung_up = true;
        self.output.clear();
        self.exited = self.ending.is_none();
    }

    fn learn_ending(&mut self) {
        let mut byte = [0u8];
        let read = self.ending.as_mut().map(|ending| ending.read(&mut byte));
        match read {
            Some(Err(error)) if error.kind() == io::ErrorKind::WouldBlock => {}
            Some(Ok(1)) if byte[0] == 1 => {
                self.out_of_memory = true;
                let message = L10N.tr("tab-out-of-memory");
                self.terminal
                    .feed(format!("{CALM_SCREEN}\r\n\r\n{message}").as_bytes());
            }
            _ => self.exited = true,
        }
    }

    pub fn has_pending_output(&self) -> bool {
        !self.output.is_empty()
    }

    pub fn send(&mut self, bytes: &[u8]) {
        if self.hung_up {
            return;
        }
        self.output.extend_from_slice(bytes);
        self.flush();
    }

    pub fn flush(&mut self) {
        while !self.output.is_empty() {
            let mut master = self.pty.file();
            match master.write(&self.output) {
                Ok(0) => break,
                Ok(written) => {
                    self.output.drain(..written);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
    }

    pub fn read(&mut self) -> Vec<Notice> {
        if self.hung_up {
            self.learn_ending();
            return Vec::new();
        }
        let mut buffer = [0u8; READ_BUFFER];
        for _ in 0..READS_PER_WAKE {
            let mut master = self.pty.file();
            match master.read(&mut buffer) {
                Ok(0) => {
                    self.hang_up();
                    break;
                }
                Ok(read) => {
                    let chunk = &buffer[..read];
                    for report in self.reports.scan(chunk) {
                        self.reported(report);
                    }
                    self.terminal.feed(chunk);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => {
                    self.hang_up();
                    break;
                }
            }
        }
        self.refresh_foreground();
        let notices = self.take_notices();
        self.flush();
        notices
    }

    fn reported(&mut self, report: Report) {
        match report {
            Report::Directory(directory) => self.reported_directory = Some(directory),
            Report::CommandStarted => {
                if self.command.is_none() {
                    let name = self.running_program().unwrap_or_default().to_owned();
                    self.command = Some((Instant::now(), name));
                }
            }
            Report::CommandFinished(code) => self.finish_command(code),
            Report::Notify { title, body } => self.notices.push(Notice::Notify { title, body }),
        }
    }

    fn finish_command(&mut self, code: Option<i32>) {
        if let Some((started, command)) = self.command.take() {
            self.notices.push(Notice::Finished {
                command,
                elapsed: started.elapsed(),
                code,
            });
        }
    }

    pub fn take_notices(&mut self) -> Vec<Notice> {
        let mut notices = std::mem::take(&mut self.notices);
        for event in self.terminal.take_events() {
            match event {
                TermEvent::PtyWrite(text) => self.send(text.as_bytes()),
                TermEvent::TextAreaSizeRequest(reply) => {
                    let text = reply(self.terminal.window_size());
                    self.send(text.as_bytes());
                }
                TermEvent::Title(title) => self.program_title = Some(title),
                TermEvent::ResetTitle => self.program_title = None,
                TermEvent::ClipboardStore(_, text) => notices.push(Notice::Copy(text)),
                TermEvent::Bell => notices.push(Notice::Bell),
                TermEvent::ChildExit(_) | TermEvent::Exit => self.hang_up(),
                _ => {}
            }
        }
        notices
    }

    pub fn resize(&mut self, size: GridSize, cell_size: (u16, u16)) {
        if self.terminal.resize(size, cell_size) {
            self.pty.on_resize(self.terminal.window_size());
        }
    }

    pub fn refresh_foreground(&mut self) -> bool {
        let group = process::foreground_group(self.shell).unwrap_or(self.shell);
        let name = process::name(group);
        let same_group = self
            .foreground
            .as_ref()
            .is_some_and(|(pid, _)| *pid == group);
        if same_group
            && self
                .foreground
                .as_ref()
                .is_some_and(|(_, known)| Some(known) == name.as_ref())
        {
            return false;
        }
        self.foreground = name.map(|name| (group, name));
        match self.running_program().map(str::to_owned) {
            Some(name) => match &mut self.command {
                Some((_, command)) if command.is_empty() || same_group => *command = name,
                Some(_) => {}
                None => self.command = Some((Instant::now(), name)),
            },
            None => self.finish_command(None),
        }
        true
    }

    pub fn running_program(&self) -> Option<&str> {
        self.foreground
            .as_ref()
            .filter(|(pid, _)| *pid != self.shell)
            .map(|(_, name)| name.as_str())
    }

    pub fn working_directory(&self) -> Option<PathBuf> {
        self.reported_directory
            .clone()
            .or_else(|| process::working_directory(self.shell))
    }

    pub fn title(&self, home: Option<&Path>) -> String {
        if let Some(title) = self
            .program_title
            .as_ref()
            .filter(|title| !title.is_empty())
        {
            return title.clone();
        }
        let directory = self
            .working_directory()
            .map(|directory| display_directory(&directory, home));
        let program = self.foreground.as_ref().map(|(_, name)| name.clone());
        [directory, program]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(TITLE_SEPARATOR)
    }
}

pub fn display_directory(directory: &Path, home: Option<&Path>) -> String {
    if home == Some(directory) {
        return "~".to_owned();
    }
    directory.file_name().map_or_else(
        || directory.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}
