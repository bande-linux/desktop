use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event as TermEvent, OnResize};
use alacritty_terminal::tty::{self, Options, Pty, Shell};
use bande_l10n::tr;
use bande_xdg::TERMINAL_APP_ID;
use rustix::event::epoll::{self, CreateFlags, EventData, EventFlags};
use rustix::io::Errno;
use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, kill_process, waitid};

use crate::process::{self, Report, Reports};
use bande_terminal_view::{GridSize, Terminal, Theme};

const READ_BUFFER: usize = 65536;
const READS_PER_WAKE: usize = 16;
const UNREAD_INPUT_CAP: usize = 1 << 20;
const WRAPPER: &str = "/bin/sh";
const ENTER: u8 = b'\r';
const LEAVE_PROGRAM: &[u8] =
    b"\x1b[?1049l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?2004l\x1b[?25l\x1b[0m\r\n";
const OUT_OF_MEMORY: u8 = 1;
const TITLE_SEPARATOR: &str = " — ";

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
    readiness: OwnedFd,
    awaiting_room: bool,
    shell: u32,
    output: Vec<u8>,
    reports: Reports,
    reported_directory: Option<PathBuf>,
    program_title: Option<String>,
    foreground: Option<(u32, String)>,
    command: Option<(Instant, String)>,
    notices: Vec<Notice>,
    exited: bool,
    ending: Option<UnixStream>,
    stopped: bool,
}

pub type ShellCommand = (String, Vec<String>);

const END_PATIENCE: Duration = Duration::from_secs(5);
const END_POLL: Duration = Duration::from_millis(5);
const RESULT_PATIENCE: Duration = Duration::from_secs(2);
const SCOPE_PATIENCE: Duration = Duration::from_secs(2);
const STOPPED_UNTIL_PLACED: &str = r#"kill -STOP $$
not_started=$1
shift
trap 'printf "%s\n" "$not_started"; read -r _' EXIT
[ $# -gt 0 ] || set -- "${SHELL:-$(getent passwd "$(id -u)" | cut -d: -f7)}"
exec "$@""#;

fn stopped_until_placed(shell: Option<ShellCommand>) -> ShellCommand {
    let mut wrapped = ["-c", STOPPED_UNTIL_PLACED, "sh"]
        .map(str::to_owned)
        .to_vec();
    wrapped.push(tr!(
        "The program could not be started. Press Return to close this tab."
    ));
    if let Some((program, arguments)) = shell {
        wrapped.push(program);
        wrapped.extend(arguments);
    }
    (WRAPPER.to_owned(), wrapped)
}

fn place_and_start(shell: u32) {
    let Some(pid) = i32::try_from(shell).ok().and_then(Pid::from_raw) else {
        return;
    };
    let stopped = loop {
        match waitid(
            WaitId::Pid(pid),
            WaitIdOptions::STOPPED | WaitIdOptions::EXITED | WaitIdOptions::NOWAIT,
        ) {
            Err(Errno::INTR) => {}
            waited => break waited,
        }
    };
    match stopped {
        Ok(Some(status)) if status.stopped() => {}
        Ok(_) | Err(Errno::CHILD) => return,
        Err(error) => return tracing::warn!("cannot tell whether the shell started: {error}"),
    }
    match bande_xdg::place_tab_in_scope(TERMINAL_APP_ID, shell, SCOPE_PATIENCE) {
        Ok(true) => {}
        Ok(false) => tracing::warn!("the tab's scope was not ready in time"),
        Err(error) => tracing::debug!("the tab shares the app's scope: {error}"),
    }
    match kill_process(pid, Signal::CONT) {
        Ok(()) | Err(Errno::SRCH) => {}
        Err(error) => tracing::warn!("cannot start the shell: {error}"),
    }
}

fn kill_stopped(shell: u32) {
    let Some(pid) = i32::try_from(shell).ok().and_then(Pid::from_raw) else {
        return;
    };
    if let Err(error) = kill_process(pid, Signal::KILL) {
        tracing::warn!("cannot end the stopped shell: {error}");
    }
}

fn ran_out_of_memory(shell: u32) -> bool {
    let Some(pid) = i32::try_from(shell).ok().and_then(Pid::from_raw) else {
        return false;
    };
    let deadline = Instant::now() + END_PATIENCE;
    loop {
        let ended = waitid(
            WaitId::Pid(pid),
            WaitIdOptions::EXITED | WaitIdOptions::NOWAIT | WaitIdOptions::NOHANG,
        );
        match ended {
            Ok(Some(status)) if status.terminating_signal() == Some(Signal::KILL.as_raw()) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(END_POLL),
            _ => return false,
        }
    }
    bande_xdg::tab_ran_out_of_memory(TERMINAL_APP_ID, shell, RESULT_PATIENCE)
        .inspect_err(|error| tracing::debug!("cannot tell why the shell was killed: {error}"))
        .unwrap_or_default()
}

fn enter_server_folder_in_the_shell(
    shell: Option<ShellCommand>,
    directory: Option<PathBuf>,
) -> (Option<ShellCommand>, Option<PathBuf>) {
    let Some(folder) = directory
        .as_ref()
        .filter(|folder| bande_volumes::on_connected_volume(folder))
        .cloned()
    else {
        return (shell, directory);
    };
    let (program, arguments) = shell.unwrap_or_default();
    let start = match program.is_empty() {
        true => r#"cd -- "$1"; exec "${SHELL:-/bin/sh}" -l"#,
        false => r#"cd -- "$1"; shift; exec "$@""#,
    };
    let mut wrapped = vec![
        "-c".to_owned(),
        start.to_owned(),
        "sh".to_owned(),
        folder.to_string_lossy().into_owned(),
    ];
    if !program.is_empty() {
        wrapped.push(program);
        wrapped.extend(arguments);
    }
    (Some(("/bin/sh".to_owned(), wrapped)), None)
}

impl Session {
    pub fn spawn(
        terminal: Terminal,
        shell: Option<ShellCommand>,
        directory: Option<PathBuf>,
    ) -> io::Result<Self> {
        let (shell, directory) = enter_server_folder_in_the_shell(shell, directory);
        let (program, arguments) = stopped_until_placed(shell);
        let options = Options {
            shell: Some(Shell::new(program, arguments)),
            working_directory: directory,
            drain_on_exit: true,
            env: HashMap::from([
                ("TERM".to_owned(), "xterm-256color".to_owned()),
                ("COLORTERM".to_owned(), "truecolor".to_owned()),
                ("TERM_PROGRAM".to_owned(), "bande-terminal".to_owned()),
            ]),
        };
        let pty = tty::new(&options, terminal.window_size(), 0)?;
        let shell = pty.child().id();
        std::thread::Builder::new()
            .name("tab-scope".to_owned())
            .spawn(move || place_and_start(shell))
            .inspect_err(|_| kill_stopped(shell))?;
        let readiness = epoll::create(CreateFlags::CLOEXEC)?;
        epoll::add(
            &readiness,
            pty.file(),
            EventData::new_u64(0),
            EventFlags::IN,
        )?;
        let mut session = Self {
            terminal,
            pty,
            readiness,
            awaiting_room: false,
            shell,
            output: Vec::new(),
            reports: Reports::new(process::host_name()),
            reported_directory: None,
            program_title: None,
            foreground: None,
            command: None,
            notices: Vec::new(),
            exited: false,
            ending: None,
            stopped: false,
        };
        session.refresh_foreground();
        Ok(session)
    }

    pub fn id(&self) -> u32 {
        self.shell
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.readiness.as_fd()
    }

    fn ended(&self) -> bool {
        self.stopped || self.ending.is_some()
    }

    pub fn exited(&self) -> bool {
        self.exited
    }

    pub fn has_pending_output(&self) -> bool {
        !self.output.is_empty()
    }

    pub fn send(&mut self, bytes: &[u8]) -> bool {
        if self.ended() {
            self.exited |= self.stopped && bytes.contains(&ENTER);
            return true;
        }
        if self.output.len() >= UNREAD_INPUT_CAP {
            return false;
        }
        self.output.extend_from_slice(bytes);
        self.flush();
        true
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
        self.await_room(!self.output.is_empty());
    }

    fn await_room(&mut self, wanted: bool) {
        if self.ended() || wanted == self.awaiting_room {
            return;
        }
        let events = match wanted {
            true => EventFlags::IN | EventFlags::OUT,
            false => EventFlags::IN,
        };
        match epoll::modify(
            &self.readiness,
            self.pty.file(),
            EventData::new_u64(0),
            events,
        ) {
            Ok(()) => self.awaiting_room = wanted,
            Err(error) => tracing::warn!("cannot wait to type into the terminal: {error}"),
        }
    }

    pub fn read(&mut self, theme: &Theme) -> Vec<Notice> {
        if self.ended() {
            self.learn_why_the_shell_ended();
            return Vec::new();
        }
        let mut buffer = [0u8; READ_BUFFER];
        for _ in 0..READS_PER_WAKE {
            let mut master = self.pty.file();
            match master.read(&mut buffer) {
                Ok(0) => {
                    self.shell_ended();
                    break;
                }
                Ok(read) => {
                    let chunk = &buffer[..read];
                    let mut fed = 0;
                    for (end, report) in self.reports.scan(chunk) {
                        self.terminal.feed(&chunk[fed..end]);
                        fed = end;
                        self.reported(report);
                    }
                    self.terminal.feed(&chunk[fed..]);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => {
                    self.shell_ended();
                    break;
                }
            }
        }
        if !self.ended() {
            self.refresh_foreground();
        }
        let notices = self.take_notices(theme);
        self.flush();
        notices
    }

    fn shell_ended(&mut self) {
        if let Err(error) = self.ask_why_the_shell_ended() {
            tracing::warn!("cannot ask why the shell ended: {error}");
            self.exited = true;
        }
    }

    fn ask_why_the_shell_ended(&mut self) -> io::Result<()> {
        epoll::delete(&self.readiness, self.pty.file())?;
        let (asked, answering) = UnixStream::pair()?;
        asked.set_nonblocking(true)?;
        epoll::add(
            &self.readiness,
            &asked,
            EventData::new_u64(0),
            EventFlags::IN,
        )?;
        let shell = self.shell;
        std::thread::Builder::new()
            .name("tab-end".to_owned())
            .spawn(move || {
                if ran_out_of_memory(shell) {
                    let _ = (&answering).write_all(&[OUT_OF_MEMORY]);
                }
            })?;
        self.ending = Some(asked);
        Ok(())
    }

    fn learn_why_the_shell_ended(&mut self) {
        let Some(asked) = &mut self.ending else {
            return;
        };
        match asked.read(&mut [0]) {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
            Ok(1) => self.stopped = true,
            _ => self.exited = true,
        }
        if let Some(asked) = self.ending.take()
            && let Err(error) = epoll::delete(&self.readiness, &asked)
        {
            tracing::warn!("cannot leave the ended shell: {error}");
        }
        if !self.stopped {
            return;
        }
        self.output.clear();
        self.foreground = None;
        self.finish_command(None);
        let notice = tr!(
            "Everything in this tab was stopped because memory ran low. Press Return to close it."
        );
        self.terminal.feed(LEAVE_PROGRAM);
        self.terminal
            .feed(format!("\x1b[1m{notice}\x1b[0m").as_bytes());
    }

    fn reported(&mut self, report: Report) {
        match report {
            Report::Directory(directory) => self.reported_directory = Some(directory),
            Report::PromptStarted => self.terminal.mark_output_end(),
            Report::CommandStarted => self.start_command(),
            Report::OutputStarted => {
                self.terminal.mark_output_start();
                self.start_command();
            }
            Report::CommandFinished(code) => {
                self.terminal.mark_output_end();
                self.finish_command(code);
            }
            Report::Notify { title, body } => self.notices.push(Notice::Notify { title, body }),
        }
    }

    fn start_command(&mut self) {
        if self.command.is_none() {
            let name = self.running_program().unwrap_or_default().to_owned();
            self.command = Some((Instant::now(), name));
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

    pub fn take_notices(&mut self, theme: &Theme) -> Vec<Notice> {
        let mut notices = std::mem::take(&mut self.notices);
        for event in self.terminal.take_events() {
            match event {
                TermEvent::PtyWrite(text) => {
                    self.send(text.as_bytes());
                }
                TermEvent::ColorRequest(index, reply) => {
                    if let Some(color) = self.terminal.reported_color(index, theme) {
                        self.send(reply(color).as_bytes());
                    }
                }
                TermEvent::TextAreaSizeRequest(reply) => {
                    let text = reply(self.terminal.window_size());
                    self.send(text.as_bytes());
                }
                TermEvent::Title(title) => self.program_title = Some(title),
                TermEvent::ResetTitle => self.program_title = None,
                TermEvent::ClipboardStore(_, text) => notices.push(Notice::Copy(text)),
                TermEvent::Bell => notices.push(Notice::Bell),
                TermEvent::ChildExit(_) | TermEvent::Exit => self.exited = true,
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
