mod ipp;
mod nearby;

use std::path::{Path, PathBuf};
use std::process::Command;

pub use ipp::{Error, Result};
pub use nearby::{NearbyPrinter, browse};

use ipp::{Group, KEYWORD, PRINTER, Request, URI};

const SOCKET: &str = "/run/cups/cups.sock";
const LPADMIN: &str = "/usr/sbin/lpadmin";
const SERVER: &str = "CUPS_SERVER";
const LOCAL: &str = "ipp://localhost";
const GET_JOBS: u16 = 0x000A;
const CANCEL_JOB: u16 = 0x0008;
const PAUSE_PRINTER: u16 = 0x0010;
const RESUME_PRINTER: u16 = 0x0011;
const GET_DEFAULT: u16 = 0x4001;
const GET_PRINTERS: u16 = 0x4002;
const DELETE_PRINTER: u16 = 0x4004;
const SET_DEFAULT: u16 = 0x400A;
const STATE_PRINTING: i32 = 4;
const STATE_STOPPED: i32 = 5;
const JOB_HELD: i32 = 4;
const JOB_PROCESSING: i32 = 5;
const JOB_STOPPED: i32 = 6;
const NAME_LIMIT: usize = 127;
const PRINTER_ATTRIBUTES: [&str; 10] = [
    "printer-name",
    "printer-info",
    "printer-location",
    "printer-make-and-model",
    "device-uri",
    "printer-state",
    "printer-state-reasons",
    "printer-is-accepting-jobs",
    "queued-job-count",
    "printer-type",
];
const JOB_ATTRIBUTES: [&str; 5] = [
    "job-id",
    "job-name",
    "job-state",
    "job-originating-user-name",
    "job-printer-uri",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrinterState {
    Idle,
    Printing,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Printer {
    pub name: String,
    pub info: String,
    pub location: String,
    pub model: String,
    pub uri: String,
    pub state: PrinterState,
    pub reasons: Vec<String>,
    pub accepting: bool,
    pub jobs: u32,
    pub default: bool,
}

impl Printer {
    pub fn title(&self) -> &str {
        if self.info.is_empty() {
            &self.name
        } else {
            &self.info
        }
    }

    fn of(group: &Group, default: Option<&str>) -> Option<Self> {
        let name = group.text("printer-name")?;
        Some(Self {
            default: default == Some(name.as_str()),
            info: group.text("printer-info").unwrap_or_default(),
            location: group.text("printer-location").unwrap_or_default(),
            model: group.text("printer-make-and-model").unwrap_or_default(),
            uri: group.text("device-uri").unwrap_or_default(),
            state: match group.integer("printer-state") {
                Some(STATE_PRINTING) => PrinterState::Printing,
                Some(STATE_STOPPED) => PrinterState::Stopped,
                _ => PrinterState::Idle,
            },
            reasons: group
                .texts("printer-state-reasons")
                .into_iter()
                .filter(|reason| reason != "none")
                .collect(),
            accepting: group.boolean("printer-is-accepting-jobs").unwrap_or(true),
            jobs: group
                .integer("queued-job-count")
                .and_then(|count| u32::try_from(count).ok())
                .unwrap_or(0),
            name,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Waiting,
    Held,
    Printing,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub id: i32,
    pub name: String,
    pub user: String,
    pub printer: String,
    pub state: JobState,
}

fn user() -> String {
    let uid = rustix::process::getuid().as_raw().to_string();
    std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|passwd| {
            passwd.lines().find_map(|line| {
                let mut fields = line.split(':');
                let login = fields.next()?;
                (fields.nth(1)? == uid).then(|| login.to_owned())
            })
        })
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_default()
}

pub fn queue_name(title: &str) -> String {
    let mut name: String = title
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '_'
            }
        })
        .take(NAME_LIMIT)
        .collect();
    while name.contains("__") {
        name = name.replace("__", "_");
    }
    let name = name.trim_matches('_').to_owned();
    if name.is_empty() {
        "Printer".to_owned()
    } else {
        name
    }
}

pub struct Cups {
    socket: PathBuf,
    user: String,
}

impl Default for Cups {
    fn default() -> Self {
        Self::new()
    }
}

impl Cups {
    pub fn new() -> Self {
        let socket = std::env::var_os(SERVER)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| PathBuf::from(SOCKET));
        Self {
            socket,
            user: user(),
        }
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    pub fn installed(&self) -> bool {
        self.socket.exists()
    }

    fn request(&self, operation: u16) -> Request {
        Request::new(operation, &self.user)
    }

    fn printer_request(&self, operation: u16, name: &str) -> Request {
        let mut request = self.request(operation);
        request.string(URI, "printer-uri", &format!("{LOCAL}/printers/{name}"));
        request
    }

    fn send(&self, request: &mut Request, resource: &str) -> Result<Vec<Group>> {
        request.send(&self.socket, resource, &self.user)
    }

    pub fn default_printer(&self) -> Result<Option<String>> {
        let mut request = self.request(GET_DEFAULT);
        request.strings(KEYWORD, "requested-attributes", &["printer-name"]);
        match self.send(&mut request, "/") {
            Ok(groups) => Ok(groups
                .iter()
                .filter(|group| group.tag == PRINTER)
                .find_map(|group| group.text("printer-name"))),
            Err(Error::Refused(_, _)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub fn printers(&self) -> Result<Vec<Printer>> {
        let default = self.default_printer()?;
        let mut request = self.request(GET_PRINTERS);
        request.strings(KEYWORD, "requested-attributes", &PRINTER_ATTRIBUTES);
        let groups = match self.send(&mut request, "/") {
            Err(Error::Refused(_, _)) => Vec::new(),
            other => other?,
        };
        let mut printers: Vec<Printer> = groups
            .iter()
            .filter(|group| group.tag == PRINTER)
            .filter_map(|group| Printer::of(group, default.as_deref()))
            .collect();
        printers.sort_by_key(|printer| printer.title().to_lowercase());
        Ok(printers)
    }

    pub fn jobs(&self) -> Result<Vec<Job>> {
        let mut request = self.request(GET_JOBS);
        request
            .string(URI, "printer-uri", &format!("{LOCAL}/"))
            .string(KEYWORD, "which-jobs", "not-completed")
            .strings(KEYWORD, "requested-attributes", &JOB_ATTRIBUTES);
        let groups = match self.send(&mut request, "/") {
            Err(Error::Refused(_, _)) => Vec::new(),
            other => other?,
        };
        Ok(groups
            .iter()
            .filter(|group| group.tag == ipp::JOB)
            .filter_map(|group| {
                Some(Job {
                    id: group.integer("job-id")?,
                    name: group.text("job-name").unwrap_or_default(),
                    user: group.text("job-originating-user-name").unwrap_or_default(),
                    printer: group
                        .text("job-printer-uri")?
                        .rsplit('/')
                        .next()?
                        .to_owned(),
                    state: match group.integer("job-state") {
                        Some(JOB_HELD) => JobState::Held,
                        Some(JOB_PROCESSING) => JobState::Printing,
                        Some(JOB_STOPPED) => JobState::Stopped,
                        _ => JobState::Waiting,
                    },
                })
            })
            .collect())
    }

    pub fn set_default(&self, name: &str) -> Result<()> {
        self.send(&mut self.printer_request(SET_DEFAULT, name), "/admin/")
            .map(drop)
    }

    pub fn remove(&self, name: &str) -> Result<()> {
        self.send(&mut self.printer_request(DELETE_PRINTER, name), "/admin/")
            .map(drop)
    }

    pub fn pause(&self, name: &str, paused: bool) -> Result<()> {
        let operation = if paused {
            PAUSE_PRINTER
        } else {
            RESUME_PRINTER
        };
        self.send(&mut self.printer_request(operation, name), "/admin/")
            .map(drop)
    }

    pub fn cancel(&self, job: i32) -> Result<()> {
        let mut request = self.request(CANCEL_JOB);
        request
            .string(URI, "printer-uri", &format!("{LOCAL}/"))
            .integer(ipp::INTEGER, "job-id", job);
        self.send(&mut request, "/jobs/").map(drop)
    }

    pub fn add(&self, printer: &NearbyPrinter) -> std::io::Result<String> {
        let name = queue_name(&printer.name);
        let program = if Path::new(LPADMIN).exists() {
            LPADMIN
        } else {
            "lpadmin"
        };
        let output = Command::new(program)
            .env(SERVER, &self.socket)
            .args(["-p", &name, "-E", "-v", &printer.uri, "-m", "everywhere"])
            .args(["-D", &printer.name])
            .args(["-L", &printer.location])
            .output()?;
        if output.status.success() {
            return Ok(name);
        }
        Err(std::io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

pub fn spooler_signal(message: &zbus::Message) -> bool {
    message
        .header()
        .interface()
        .is_some_and(|interface| interface.as_str() == SPOOLER)
}

pub const SPOOLER: &str = "com.redhat.PrinterSpooler";
