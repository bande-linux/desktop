use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::BorrowedFd;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::rc::Rc;
use std::sync::LazyLock;

use accesskit::TreeUpdate;
use bande_app::{Cx, Info, Keys, Kind, Program, View, WindowId};
use bande_inbox::{Inbox, Outbox};
use bande_l10n::Localizer;
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
const WINDOW_SIZE: (u32, u32) = (620, 640);
const FIELD_FONT: f32 = 13.0;
const LIST_HEIGHT: f32 = 240.0;
const BACKUP_TOOL: &str = "bande-backup-restore";

static L10N: LazyLock<Localizer> = LazyLock::new(|| {
    Localizer::new(&[(
        "en-US",
        include_str!("../locales/en-US/bande-recovery-tools.ftl"),
    )])
});

static INFO: Info = Info {
    id: APP_ID,
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || L10N.tr("app-name"),
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
    Working(String),
    Finished,
    Failed(String),
}

enum Reply {
    Step(String),
    Finished,
    Failed(String),
    Versions(Vec<(String, String)>),
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

fn disks() -> Vec<(String, String)> {
    let output = Command::new("lsblk")
        .args(["--json", "--nodeps", "-o", "PATH,SIZE,MODEL,TYPE,RM"])
        .output();
    let devices = output
        .ok()
        .and_then(|output| serde_json::from_slice::<Devices>(&output.stdout).ok())
        .map_or_else(Vec::new, |devices| devices.blockdevices);
    devices
        .into_iter()
        .filter(|device| device.kind == "disk" && !device.rm)
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
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            match line.split_once(' ') {
                Some(("step", step)) => {
                    outbox.send(Reply::Step(step.to_owned()));
                }
                Some(("error", message)) => failure = Some(message.to_owned()),
                Some(("version", version)) => {
                    let mut fields = version.split('\t');
                    let id = fields.next().unwrap_or_default().to_owned();
                    let created = fields.nth(0).unwrap_or_default();
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
        (true, _) => Reply::Finished,
        (false, failure) => Reply::Failed(failure.unwrap_or_else(|| "failed".to_owned())),
    });
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
    secret: Zeroizing<String>,
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
                let mut rows = vec![boxed(Label::heading(&L10N.tr("home")))];
                if Path::new(INSTALLED_ROOT).exists() {
                    rows.push(self.row(
                        &L10N.tr("reinstall"),
                        &L10N.tr("reinstall-detail"),
                        Intent::AskPassword(Purpose::Reinstall),
                    ));
                    rows.push(self.row(
                        &L10N.tr("versions"),
                        &L10N.tr("versions-detail"),
                        Intent::AskPassword(Purpose::Versions),
                    ));
                    rows.push(self.row(
                        &L10N.tr("backup"),
                        &L10N.tr("backup-detail"),
                        Intent::Launch(BACKUP_TOOL),
                    ));
                }
                rows.push(self.row(
                    &L10N.tr("install"),
                    &L10N.tr("install-detail"),
                    Intent::ChooseDisk,
                ));
                rows.push(self.row(
                    &L10N.tr("disks"),
                    &L10N.tr("disks-detail"),
                    Intent::Launch("bande-disks"),
                ));
                rows.push(self.row(
                    &L10N.tr("terminal"),
                    &L10N.tr("terminal-detail"),
                    Intent::Launch("bande-terminal"),
                ));
                rows.push(boxed(self.button(&L10N.tr("restart"), Intent::Restart)));
                rows
            }
            Page::Disks => vec![
                boxed(Label::heading(&L10N.tr("choose-disk"))),
                self.list(&disks(), Intent::Disk),
                boxed(Label::new(&L10N.tr("erase-warning"))),
                boxed(Stack::new(
                    Axis::Horizontal,
                    Spacing::S as f32,
                    0.0,
                    vec![
                        boxed(self.button(&L10N.tr("back"), Intent::Home)),
                        boxed(
                            self.button(&L10N.tr("erase"), Intent::Install)
                                .destructive(),
                        ),
                    ],
                )),
            ],
            Page::Password(_) => {
                let intents = Rc::clone(&self.intents);
                let field = SecureField::new(&L10N.tr("password"), FIELD_FONT, move |_| {
                    intents.borrow_mut().push(Intent::Submit);
                });
                self.password = Some(field.id());
                vec![
                    boxed(Label::heading(&L10N.tr("password-title"))),
                    boxed(field),
                    boxed(Stack::new(
                        Axis::Horizontal,
                        Spacing::S as f32,
                        0.0,
                        vec![
                            boxed(self.button(&L10N.tr("back"), Intent::Home)),
                            boxed(self.button(&L10N.tr("continue"), Intent::Submit).primary()),
                        ],
                    )),
                ]
            }
            Page::Versions => vec![
                boxed(Label::heading(&L10N.tr("versions"))),
                self.list(&self.versions.clone(), Intent::Version),
                boxed(self.button(&L10N.tr("back"), Intent::Home)),
            ],
            Page::Working(step) => vec![boxed(Label::heading(&L10N.tr(&format!("step-{step}"))))],
            Page::Finished => vec![
                boxed(Label::heading(&L10N.tr("step-done"))),
                boxed(self.button(&L10N.tr("restart"), Intent::Restart).primary()),
            ],
            Page::Failed(message) => vec![
                boxed(Label::heading(&message)),
                boxed(self.button(&L10N.tr("back"), Intent::Home)),
            ],
        }
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
                Intent::AskPassword(purpose) => self.page = Page::Password(purpose),
                Intent::Submit => {
                    let secret = self.password.and_then(|id| {
                        self.ui
                            .widget_ref::<SecureField>(id)
                            .map(|field| Zeroizing::new(field.secret().to_owned()))
                    });
                    let Some(secret) = secret else {
                        continue;
                    };
                    self.secret = secret.clone();
                    let command = match self.page {
                        Page::Password(Purpose::Versions) => "versions",
                        _ => "reinstall",
                    };
                    self.engine(vec![command.to_owned()], Some(secret));
                }
                Intent::Version(id) => {
                    self.engine(vec!["restore".to_owned(), id], Some(self.secret.clone()));
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
                Reply::Finished => Page::Finished,
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
        let window = cx.open(
            Kind::Main,
            &L10N.tr("app-name"),
            WINDOW_SIZE,
            |appearance| {
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
                };
                tools.present();
                Pane(tools)
            },
        )?;
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

fn main() -> ExitCode {
    bande_app::run::<Recovery>(&INFO)
}
