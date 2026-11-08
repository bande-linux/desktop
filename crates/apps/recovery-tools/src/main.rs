use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::BorrowedFd;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::rc::Rc;

use accesskit::TreeUpdate;
use bande_app::{Cx, Info, Keys, Kind, Program, View, WindowId};
use bande_inbox::{Inbox, Outbox};
use bande_l10n::tr;
use bande_toolkit::ui::{
    Axis, Bounded, Button, Cell, Column, Context, Label, Row, SecureField, Stack, Table, Ui,
    Widget, WidgetId,
};
use bande_toolkit::{Appearance, Scene, Spacing, TextSystem, Window, WindowEvent};
use bande_xdg::spawn_detached;
use serde::Deserialize;
use zeroize::Zeroizing;

const APP_ID: &str = "org.bande.RecoveryTools";
const ENGINE: &str = "/usr/libexec/bande-install";
const INSTALLED_ROOT: &str = "/dev/disk/by-label/BANDE-ROOT";
const LIVE_DEVICE: &str = "/run/bande-live/device";
const WINDOW_SIZE: (u32, u32) = (620, 640);
const FIELD_FONT: f32 = 13.0;
const LIST_HEIGHT: f32 = 240.0;

static INFO: Info = Info {
    id: APP_ID,
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || tr!("Recovery"),
    keys: Keys::Standard,
    settings: None,
};

#[derive(Debug, Clone)]
enum Intent {
    Home,
    ChooseDisk,
    Disk(String),
    Install,
    AskPassword(Purpose),
    Submit,
    Version(String),
    Launch(&'static str),
    ChooseBackup,
    Backup(String),
    SubmitBackup,
    Restart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Reinstall,
    Versions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Page {
    Home,
    Disks,
    Password(Purpose),
    Versions,
    Backups,
    BackupPassword,
    Working(String),
    Finished(Option<String>),
    Failed(String),
}

enum Reply {
    Step(String),
    Finished(Option<String>),
    Failed(String),
    Versions(Vec<(String, String)>),
}

#[derive(Debug, Clone, Deserialize)]
struct Backup {
    uuid: String,
    filesystem: String,
    machine: String,
    name: String,
}

#[derive(Deserialize)]
struct Devices {
    blockdevices: Vec<Device>,
}

#[derive(Deserialize)]
struct Device {
    path: String,
    size: String,
    model: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    rm: bool,
}

fn running_disk() -> Option<String> {
    let partition = std::fs::read_to_string(LIVE_DEVICE).ok()?;
    let output = Command::new("lsblk")
        .args(["-ndo", "PKNAME", partition.trim()])
        .output()
        .ok()?;
    let parent = String::from_utf8_lossy(&output.stdout);
    Some(format!("/dev/{}", parent.trim()))
}

fn disks() -> Vec<(String, String)> {
    let output = Command::new("lsblk")
        .args(["--json", "--nodeps", "-o", "PATH,SIZE,MODEL,TYPE,RM"])
        .output();
    let devices = output
        .ok()
        .and_then(|output| serde_json::from_slice::<Devices>(&output.stdout).ok())
        .map_or_else(Vec::new, |devices| devices.blockdevices);
    let running = running_disk();
    devices
        .into_iter()
        .filter(|device| device.kind == "disk" && !device.rm && !device.path.contains("zram"))
        .filter(|device| running.as_deref() != Some(device.path.as_str()))
        .map(|device| {
            let name = format!(
                "{} {} ({})",
                device.model.as_deref().unwrap_or("Disk").trim(),
                device.size,
                device.path
            );
            (device.path, name)
        })
        .collect()
}

fn run_engine(arguments: Vec<String>, input: Option<Zeroizing<String>>, outbox: Outbox<Reply>) {
    let mut child = match Command::new("pkexec")
        .env("SHELL", "/bin/bash")
        .arg(ENGINE)
        .args(&arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            outbox.send(Reply::Failed(error.to_string()));
            return;
        }
    };
    if let Some(mut stdin) = child.stdin.take()
        && let Some(input) = input
        && let Err(error) = stdin.write_all(format!("{}\n", input.as_str()).as_bytes())
    {
        outbox.send(Reply::Failed(error.to_string()));
        return;
    }
    let mut versions = Vec::new();
    let mut failure = None;
    let mut warning = None;
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            match line.split_once(' ') {
                Some(("step", step)) => {
                    outbox.send(Reply::Step(step.to_owned()));
                }
                Some(("warning", message)) => warning = Some(message.to_owned()),
                Some(("error", message)) => failure = Some(message.to_owned()),
                Some(("version", version)) => {
                    let mut fields = version.split('\t');
                    let id = fields.next().unwrap_or_default().to_owned();
                    let created = fields.next().unwrap_or_default();
                    let why = fields.next().unwrap_or_default();
                    versions.push((id, format!("{created} {why}")));
                }
                _ => {}
            }
        }
    }
    let succeeded = child.wait().is_ok_and(|status| status.success());
    outbox.send(match (succeeded, failure) {
        (true, _) if !versions.is_empty() => Reply::Versions(versions),
        (true, _) => Reply::Finished(warning),
        (false, failure) => Reply::Failed(failure.unwrap_or_else(|| "failed".to_owned())),
    });
}

fn needs_password() -> bool {
    Command::new("pkexec")
        .env("SHELL", "/bin/bash")
        .args([ENGINE, "needs-password"])
        .output()
        .is_ok_and(|output| output.stdout.trim_ascii() != b"none")
}

fn list_backups() -> Vec<Backup> {
    Command::new("pkexec")
        .env("SHELL", "/bin/bash")
        .args([ENGINE, "backups"])
        .output()
        .ok()
        .and_then(|output| serde_json::from_slice(&output.stdout).ok())
        .unwrap_or_default()
}

fn command_of(purpose: Purpose) -> &'static str {
    match purpose {
        Purpose::Reinstall => "reinstall",
        Purpose::Versions => "versions",
    }
}

struct Tools {
    ui: Ui,
    intents: Rc<RefCell<Vec<Intent>>>,
    inbox: Inbox<Reply>,
    page: Page,
    appearance: Appearance,
    disk: Option<String>,
    versions: Vec<(String, String)>,
    password: Option<WidgetId>,
    disk_password: Option<WidgetId>,
    secret: Zeroizing<String>,
    backups: Vec<Backup>,
    backup: Option<Backup>,
}

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

fn new_ui(root: Box<dyn Widget>, appearance: Appearance) -> Ui {
    let mut ui = Ui::new(root, Context::new(TextSystem::new(), appearance.palette()));
    ui.set_appearance(appearance);
    ui
}

impl Tools {
    fn button(&self, title: &str, intent: Intent) -> Button {
        let intents = Rc::clone(&self.intents);
        Button::new(title, move || intents.borrow_mut().push(intent.clone()))
    }

    fn row(&self, title: &str, detail: &str, intent: Intent) -> Box<dyn Widget> {
        boxed(Stack::new(
            Axis::Vertical,
            Spacing::XS as f32,
            0.0,
            vec![
                boxed(self.button(title, intent)),
                boxed(Label::new(detail).secondary()),
            ],
        ))
    }

    fn list(&self, rows: &[(String, String)], make: fn(String) -> Intent) -> Box<dyn Widget> {
        let intents = Rc::clone(&self.intents);
        let mut table = Table::new(vec![Column::new("", 560.0).flexible()])
            .without_header()
            .on_selection(move |keys: &[String]| {
                if let Some(key) = keys.first() {
                    intents.borrow_mut().push(make(key.clone()));
                }
            });
        table.set_rows(
            rows.iter()
                .map(|(key, name)| Row::new(key.clone(), vec![Cell::new(name.as_str())]))
                .collect(),
        );
        boxed(Bounded::new(table, LIST_HEIGHT))
    }

    fn content(&mut self) -> Vec<Box<dyn Widget>> {
        self.password = None;
        match self.page.clone() {
            Page::Home => {
                let mut rows = vec![boxed(Label::heading(&tr!("What do you want to do?")))];
                if Path::new(INSTALLED_ROOT).exists() {
                    rows.push(self.row(
                        &tr!("Reinstall Bande"),
                        &tr!("Installs a fresh system. Your files and accounts stay."),
                        Intent::AskPassword(Purpose::Reinstall),
                    ));
                    rows.push(self.row(
                        &tr!("Restore a Previous Version"),
                        &tr!(
                            "Go back to the system as it was on an earlier date. Your files stay."
                        ),
                        Intent::AskPassword(Purpose::Versions),
                    ));
                    rows.push(self.row(
                        &tr!("Restore from Backup"),
                        &tr!("Bring back your files from a backup disk."),
                        Intent::ChooseBackup,
                    ));
                }
                rows.push(self.row(
                    &tr!("Install Bande"),
                    &tr!("Erases a disk and installs Bande on it."),
                    Intent::ChooseDisk,
                ));
                rows.push(self.row(
                    &tr!("Disks"),
                    &tr!("Check, format and partition disks."),
                    Intent::Launch("bande-disks"),
                ));
                rows.push(self.row(
                    &tr!("Terminal"),
                    &tr!("Work at the command line."),
                    Intent::Launch("bande-terminal"),
                ));
                rows.push(boxed(self.button(&tr!("Restart"), Intent::Restart)));
                rows
            }
            Page::Disks => vec![
                boxed(Label::heading(&tr!("Choose a Disk"))),
                self.list(&disks(), Intent::Disk),
                boxed(Label::new(&tr!(
                    "This erases everything on the selected disk."
                ))),
                boxed(Stack::new(
                    Axis::Horizontal,
                    Spacing::S as f32,
                    0.0,
                    vec![
                        boxed(self.button(&tr!("Back"), Intent::Home)),
                        boxed(
                            self.button(&tr!("Erase and Install"), Intent::Install)
                                .destructive(),
                        ),
                    ],
                )),
            ],
            Page::Password(_) => {
                let intents = Rc::clone(&self.intents);
                let field = SecureField::new(&tr!("Disk password"), FIELD_FONT, move |_| {
                    intents.borrow_mut().push(Intent::Submit);
                });
                self.password = Some(field.id());
                vec![
                    boxed(Label::heading(&tr!("Enter the Disk Password"))),
                    boxed(field),
                    boxed(Stack::new(
                        Axis::Horizontal,
                        Spacing::S as f32,
                        0.0,
                        vec![
                            boxed(self.button(&tr!("Back"), Intent::Home)),
                            boxed(self.button(&tr!("Continue"), Intent::Submit).primary()),
                        ],
                    )),
                ]
            }
            Page::Versions => vec![
                boxed(Label::heading(&tr!("Restore a Previous Version"))),
                self.list(&self.versions.clone(), Intent::Version),
                boxed(self.button(&tr!("Back"), Intent::Home)),
            ],
            Page::Backups => {
                let rows: Vec<(String, String)> = self
                    .backups
                    .iter()
                    .map(|backup| (backup.uuid.clone(), backup.name.clone()))
                    .collect();
                vec![
                    boxed(Label::heading(&tr!("Restore from Backup"))),
                    self.list(&rows, Intent::Backup),
                    boxed(self.button(&tr!("Back"), Intent::Home)),
                ]
            }
            Page::BackupPassword => {
                let intents = Rc::clone(&self.intents);
                let field = SecureField::new(&tr!("Backup password"), FIELD_FONT, move |_| {
                    intents.borrow_mut().push(Intent::SubmitBackup);
                });
                self.password = Some(field.id());
                let mut rows = vec![
                    boxed(Label::heading(&tr!("Enter the Backup's Password"))),
                    boxed(field),
                ];
                if needs_password() {
                    let disk = SecureField::new(&tr!("Disk password"), FIELD_FONT, |_| {});
                    self.disk_password = Some(disk.id());
                    rows.push(boxed(disk));
                } else {
                    self.disk_password = None;
                }
                rows.push(boxed(Stack::new(
                    Axis::Horizontal,
                    Spacing::S as f32,
                    0.0,
                    vec![
                        boxed(self.button(&tr!("Back"), Intent::Home)),
                        boxed(
                            self.button(&tr!("Continue"), Intent::SubmitBackup)
                                .primary(),
                        ),
                    ],
                )));
                rows
            }
            Page::Working(step) => vec![boxed(Label::heading(&step_title(step.as_str())))],
            Page::Finished(warning) => vec![
                boxed(Label::heading(&tr!(
                    "Finished. Restart to use your computer."
                ))),
                boxed(Label::new(warning.as_deref().unwrap_or_default())),
                boxed(self.button(&tr!("Restart"), Intent::Restart).primary()),
            ],
            Page::Failed(message) => vec![
                boxed(Label::heading(&message)),
                boxed(self.button(&tr!("Back"), Intent::Home)),
            ],
        }
    }

    fn secret_of(&self, field: Option<WidgetId>) -> Option<Zeroizing<String>> {
        field.and_then(|id| {
            self.ui
                .widget_ref::<SecureField>(id)
                .map(|field| Zeroizing::new(field.secret().to_owned()))
        })
    }

    fn present(&mut self) {
        let content = self.content();
        let root = Stack::new(
            Axis::Vertical,
            Spacing::M as f32,
            Spacing::L as f32,
            content,
        );
        self.ui = new_ui(Box::new(root), self.appearance);
        if let Some(field) = self.password {
            self.ui.set_focus(field);
        }
    }

    fn engine(&mut self, arguments: Vec<String>, input: Option<Zeroizing<String>>) {
        let outbox = self.inbox.outbox();
        std::thread::spawn(move || run_engine(arguments, input, outbox));
        self.page = Page::Working("partition".to_owned());
    }

    fn settle(&mut self) -> bool {
        let intents: Vec<Intent> = self.intents.borrow_mut().drain(..).collect();
        let mut changed = !intents.is_empty();
        for intent in intents {
            match intent {
                Intent::Home => self.page = Page::Home,
                Intent::ChooseDisk => self.page = Page::Disks,
                Intent::Disk(disk) => {
                    self.disk = Some(disk);
                    continue;
                }
                Intent::Install => {
                    let Some(disk) = self.disk.clone() else {
                        continue;
                    };
                    self.engine(vec!["install".to_owned(), disk], None);
                }
                Intent::AskPassword(purpose) => {
                    if needs_password() {
                        self.page = Page::Password(purpose);
                    } else {
                        self.engine(vec![command_of(purpose).to_owned()], None);
                    }
                }
                Intent::Submit => {
                    let Some(secret) = self.secret_of(self.password) else {
                        continue;
                    };
                    self.secret = secret.clone();
                    if let Page::Password(purpose) = self.page {
                        self.engine(vec![command_of(purpose).to_owned()], Some(secret));
                    }
                }
                Intent::Version(id) => {
                    self.engine(vec!["restore".to_owned(), id], Some(self.secret.clone()));
                }
                Intent::ChooseBackup => {
                    self.backups = list_backups();
                    self.page = Page::Backups;
                }
                Intent::Backup(uuid) => {
                    self.backup = self
                        .backups
                        .iter()
                        .find(|backup| backup.uuid == uuid)
                        .cloned();
                    self.page = Page::BackupPassword;
                }
                Intent::SubmitBackup => {
                    let (Some(backup), Some(secret)) =
                        (self.backup.clone(), self.secret_of(self.password))
                    else {
                        continue;
                    };
                    let disk = self.secret_of(self.disk_password).unwrap_or_default();
                    let input = Zeroizing::new(format!("{}\n{}", disk.as_str(), secret.as_str()));
                    self.engine(
                        vec![
                            "restore-backup".to_owned(),
                            backup.uuid,
                            backup.filesystem,
                            backup.machine,
                        ],
                        Some(input),
                    );
                }
                Intent::Launch(program) => {
                    if let Err(error) = spawn_detached("", &[program.to_owned()], None, None) {
                        tracing::warn!("cannot start {program}: {error}");
                    }
                    continue;
                }
                Intent::Restart => {
                    if let Err(error) = Command::new("systemctl").arg("reboot").spawn() {
                        tracing::warn!("cannot restart: {error}");
                    }
                    continue;
                }
            }
            self.present();
            changed = true;
        }
        changed
    }

    fn receive(&mut self) -> bool {
        let replies = self.inbox.drain();
        let changed = !replies.is_empty();
        for reply in replies {
            self.page = match reply {
                Reply::Step(step) if step == "done" => continue,
                Reply::Step(step) => Page::Working(step),
                Reply::Finished(warning) => Page::Finished(warning),
                Reply::Failed(message) => Page::Failed(message),
                Reply::Versions(versions) => {
                    self.versions = versions;
                    Page::Versions
                }
            };
            self.present();
        }
        changed
    }
}

struct Pane(Tools);

impl View for Pane {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.0.ui.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        let driven = window
            .drive(&mut self.0.ui, event)
            .inspect_err(|error| tracing::warn!("{error}"))
            .is_ok_and(|unhandled| unhandled.is_none());
        self.0.settle() || driven
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.0.appearance = appearance;
        self.0.ui.set_appearance(appearance);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.0.inbox.fd()]
    }

    fn source_ready(&mut self, _window: &mut Window, _index: usize) -> bool {
        self.0.receive()
    }

    fn animating(&self) -> bool {
        self.0.ui.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.0.ui.accessibility())
    }
}

struct Recovery {
    window: Option<WindowId>,
}

impl Recovery {
    fn open(&mut self, cx: &mut Cx<Self>) -> Result<(), bande_app::Error> {
        let inbox = Inbox::new().map_err(|error| error.to_string())?;
        let window = cx.open(Kind::Main, &tr!("Recovery"), WINDOW_SIZE, |appearance| {
            let mut tools = Tools {
                ui: new_ui(
                    Box::new(Stack::new(Axis::Vertical, 0.0, 0.0, Vec::new())),
                    appearance,
                ),
                intents: Rc::default(),
                inbox,
                page: Page::Home,
                appearance,
                disk: None,
                versions: Vec::new(),
                password: None,
                secret: Zeroizing::new(String::new()),
                disk_password: None,
                backups: Vec::new(),
                backup: None,
            };
            tools.present();
            Pane(tools)
        })?;
        self.window = Some(window);
        Ok(())
    }
}

impl Program for Recovery {
    type View = Pane;

    fn start(cx: &mut Cx<Self>, _arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let mut recovery = Self { window: None };
        recovery.open(cx)?;
        Ok(recovery)
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        if self.window.is_none()
            && let Err(error) = self.open(cx)
        {
            tracing::warn!("cannot open the window: {error}");
        }
    }

    fn window_closed(&mut self, _cx: &mut Cx<Self>, _window: WindowId, _view: Pane) {
        self.window = None;
    }
}

fn step_title(step: &str) -> String {
    match step {
        "partition" => tr!("Preparing the disk"),
        "recovery" => tr!("Copying the recovery system"),
        "format" => tr!("Formatting"),
        "copy" => tr!("Copying the system"),
        "boot" => tr!("Setting up startup"),
        "done" => tr!("Finished. Restart to use your computer."),
        "restore" => tr!("Restoring your backup"),
        other => other.to_owned(),
    }
}

fn main() -> ExitCode {
    bande_app::run::<Recovery>(&INFO)
}
