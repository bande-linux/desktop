use std::cell::RefCell;
use std::io::Write;
use std::os::fd::BorrowedFd;
use std::os::unix::net::UnixStream;
use std::process::{Command, ExitCode, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;
use bande_bus::accounts::{suggested_login, valid_login};
use bande_greeter::{Login, Step as LoginStep, read_message, write_message};
use bande_inbox::{Inbox, Outbox};
use bande_keyboard::Layouts;
use bande_l10n::system::{AvailableLocale, read_supported};
use bande_l10n::{Text, text, tr};
use bande_lock::{BackdropSource, Look, Variant, load_backdrop};
use bande_network::{JoinOutcome, Network, NetworkManager};
use bande_toolkit::ui::{
    Axis, Bounded, Button, Cell, Column, Context, Event, FreeLayout, Key, Label, Row, SecureField,
    SegmentedControl, Stack, Table, TextField, Toggle, Ui, Widget, WidgetId,
};
use bande_toolkit::{
    App, Appearance, Edges, Image, Item, KeyboardFocus, LayerKind, LayerOptions, Rect, Role, Scene,
    Scheme, Shape, Spacing, TextSystem, Window, WindowEvent, run,
};
use zeroize::Zeroizing;

const HELPER: &str = "/usr/libexec/bande-first-run";
const SHELL: &str = "/bin/sh";
const CARD_WIDTH: f32 = 560.0;
const CARD_HEIGHT: f32 = 560.0;
const LIST_HEIGHT: f32 = 300.0;
const FIELD_FONT: f32 = 13.0;
const SCAN_TIME: Duration = Duration::from_secs(6);
const REPORT_SETTINGS: &str = "/usr/share/bande/reports.conf";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Language,
    Keyboard,
    Account,
    RecoveryKey,
    Wifi,
    Look,
    Privacy,
    Analytics,
    Finishing,
}

#[derive(Debug, Clone)]
enum Intent {
    Back,
    Continue,
    Language(String),
    Keyboard(String),
    Network(String),
    Join,
    EncryptDisk(bool),
    Look(usize),
    AutomaticUpdates(bool),
    ShareAnalytics(bool),
}

enum Reply {
    Done(Page),
    RecoveryKey(Option<String>),
    Failed(String),
    Networks(Vec<Network>),
    Joined(JoinOutcome),
    SignedIn,
}

fn helper(arguments: &[&str], input: Option<&str>) -> Result<String, String> {
    let mut child = Command::new("pkexec")
        .env("SHELL", SHELL)
        .arg(HELPER)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    if let Some(mut stdin) = child.stdin.take()
        && let Some(input) = input
    {
        stdin
            .write_all(input.as_bytes())
            .map_err(|error| error.to_string())?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
    }
    Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
}

fn sign_in(login: &str, password: Zeroizing<String>) -> Result<(), String> {
    let socket = std::env::var_os("GREETD_SOCK").ok_or("GREETD_SOCK is not set")?;
    let mut greetd = UnixStream::connect(socket).map_err(|error| error.to_string())?;
    let (mut conversation, request) = Login::begin(login, password);
    write_message(&mut greetd, &request).map_err(|error| error.to_string())?;
    while !conversation.is_idle() {
        let response = read_message(&mut greetd).map_err(|error| error.to_string())?;
        for step in conversation.respond(response) {
            match step {
                LoginStep::Send(request) => {
                    write_message(&mut greetd, &request).map_err(|error| error.to_string())?;
                }
                LoginStep::Show(message) => return Err(message),
                LoginStep::Ask(prompt) => return Err(prompt),
                _ => {}
            }
        }
    }
    Ok(())
}

struct Answers {
    language: String,
    keyboard: String,
    full_name: String,
    login: String,
    password: Zeroizing<String>,
    encrypt: bool,
    network: Option<String>,
    look: usize,
    automatic_updates: bool,
    share_analytics: bool,
}

struct Assistant {
    ui: Ui,
    appearance: Appearance,
    backdrop: Option<Image>,
    backdrop_inbox: Option<Inbox<(Look, Image)>>,
    table: Option<WidgetId>,
    text: TextSystem,
    page: Page,
    answers: Answers,
    locales: Vec<AvailableLocale>,
    layouts: Layouts,
    intents: Rc<RefCell<Vec<Intent>>>,
    inbox: Inbox<Reply>,
    system: Option<zbus::blocking::Connection>,
    networks: Vec<Network>,
    recovery_key: Option<String>,
    message: Option<String>,
    fields: Vec<WidgetId>,
    size: (u32, u32),
    working: bool,
    can_encrypt: bool,
    signed_in: bool,
}

const LOOKS: [(Text, &str); 3] = [
    (text!("Light"), "light"),
    (text!("Dark"), "dark"),
    (text!("Automatic"), "automatic"),
];

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

fn switch(title: &str, on: bool, on_change: impl FnMut(bool) + 'static) -> Box<dyn Widget> {
    boxed(Stack::new(
        Axis::Horizontal,
        Spacing::S as f32,
        0.0,
        vec![
            boxed(Toggle::new(title, on, on_change)),
            boxed(Label::new(title)),
        ],
    ))
}

impl Assistant {
    fn intent(&self, intent: Intent) -> impl FnMut() + 'static {
        let intents = Rc::clone(&self.intents);
        move || intents.borrow_mut().push(intent.clone())
    }

    fn list(
        &mut self,
        rows: Vec<(String, String)>,
        selected: &str,
        make: fn(String) -> Intent,
    ) -> Box<dyn Widget> {
        let intents = Rc::clone(&self.intents);
        let mut table = Table::new(vec![Column::new("", CARD_WIDTH).flexible()])
            .without_header()
            .on_selection(move |keys: &[String]| {
                if let Some(key) = keys.first() {
                    intents.borrow_mut().push(make(key.clone()));
                }
            });
        table.set_rows(
            rows.into_iter()
                .map(|(key, name)| Row::new(key, vec![Cell::new(name)]))
                .collect(),
        );
        table.select(&[selected.to_owned()]);
        self.table = Some(table.id());
        boxed(Bounded::new(table, LIST_HEIGHT))
    }

    fn content(&mut self) -> Vec<Box<dyn Widget>> {
        self.fields.clear();
        match self.page {
            Page::Language => {
                let rows = self
                    .locales
                    .iter()
                    .map(|locale| (locale.code.clone(), locale.name.clone()))
                    .collect();
                let selected = self.answers.language.clone();
                vec![self.list(rows, &selected, Intent::Language)]
            }
            Page::Keyboard => {
                let rows = self
                    .layouts
                    .all()
                    .iter()
                    .map(|layout| (layout.code.clone(), layout.name.clone()))
                    .collect();
                let selected = self.answers.keyboard.clone();
                vec![self.list(rows, &selected, Intent::Keyboard)]
            }
            Page::Account => {
                let full_name =
                    TextField::new(&tr!("Full name"), &self.answers.full_name, |_| {})
                        .without_text_checking();
                let login = TextField::new(&tr!("Account name"), &self.answers.login, |_| {})
                    .without_text_checking();
                let password = SecureField::new(&tr!("Password"), FIELD_FONT, |_| {});
                let intents = Rc::clone(&self.intents);
                let verify = SecureField::new(&tr!("Verify password"), FIELD_FONT, move |_| {
                    intents.borrow_mut().push(Intent::Continue);
                });
                self.fields = vec![full_name.id(), login.id(), password.id(), verify.id()];
                let mut rows: Vec<Box<dyn Widget>> = vec![
                    boxed(full_name),
                    boxed(login),
                    boxed(password),
                    boxed(verify),
                ];
                if self.can_encrypt {
                    let intents = Rc::clone(&self.intents);
                    rows.push(switch(
                        &tr!("Encrypt this disk with your password"),
                        self.answers.encrypt,
                        move |on| intents.borrow_mut().push(Intent::EncryptDisk(on)),
                    ));
                    rows.push(boxed(
                        Label::new(&tr!("Without the password or the recovery key, no one can read this disk, and neither can you. You type the password each time the computer starts, on a built-in or USB keyboard: Bluetooth keyboards do not work that early.")).secondary(),
                    ));
                }
                rows
            }
            Page::RecoveryKey => vec![
                boxed(Label::new(self.recovery_key.as_deref().unwrap_or_default())),
                boxed(Label::new(&tr!("Write this key down and keep it safe. It is shown only once, and it is the only way in if you forget your password.")).secondary()),
            ],
            Page::Wifi => {
                let rows = self
                    .networks
                    .iter()
                    .map(|network| (network.name(), network.name()))
                    .collect();
                let selected = self.answers.network.clone().unwrap_or_default();
                let list = self.list(rows, &selected, Intent::Network);
                let intents = Rc::clone(&self.intents);
                let password = SecureField::new(&tr!("Wi-Fi password"), FIELD_FONT, move |_| {
                    intents.borrow_mut().push(Intent::Join);
                });
                self.fields = vec![password.id()];
                let join = Button::new(&tr!("Join"), self.intent(Intent::Join));
                vec![list, boxed(password), boxed(join)]
            }
            Page::Look => {
                let intents = Rc::clone(&self.intents);
                let titles = LOOKS.map(|(id, _)| id.tr());
                vec![boxed(
                    SegmentedControl::new(
                        &tr!("Appearance"),
                        &titles,
                        Some(self.answers.look),
                        move |index| {
                            intents.borrow_mut().push(Intent::Look(index));
                        },
                    )
                    .fill_equally(),
                )]
            }
            Page::Privacy => {
                let intents = Rc::clone(&self.intents);
                vec![
                    boxed(Label::new(&tr!("Your files, account and settings stay on this computer. Bande contacts the internet only to check the clock, to check for a working connection and to get software updates."))),
                    switch(
                        &tr!("Install updates automatically"),
                        self.answers.automatic_updates,
                        move |on| intents.borrow_mut().push(Intent::AutomaticUpdates(on)),
                    ),
                ]
            }
            Page::Analytics => {
                let intents = Rc::clone(&self.intents);
                let mut rows = vec![
                    boxed(Label::new(&tr!("When something goes wrong, such as a program closing unexpectedly or the computer freezing, Bande can send a report so it gets fixed. Once a day it can also send how long the computer was used."))),
                    boxed(Label::new(&tr!("A report says which program failed and where, its version, the kind of computer and recent system messages. Names, addresses, file names and anything you typed are removed first. Your files are never sent, and nothing in a report says who you are.")).secondary()),
                    boxed(Label::new(&tr!("You can read every report, turn this off or delete what was sent at any time in Settings, under Privacy & Security, Diagnostics.")).secondary()),
                ];
                if let Some(privacy) = privacy_address() {
                    rows.push(boxed(
                        Label::new(&tr!(
                            "What the server keeps, and for how long: {address}",
                            address = privacy
                        ))
                        .secondary(),
                    ));
                }
                rows.push(switch(
                    &tr!("Share Bande Analytics"),
                    self.answers.share_analytics,
                    move |on| intents.borrow_mut().push(Intent::ShareAnalytics(on)),
                ));
                rows
            }
            Page::Finishing => vec![boxed(Label::new(&tr!("Setting up your desktop.")).secondary())],
        }
    }

    fn present(&mut self) {
        let id = match self.page {
            Page::Language => text!("Language"),
            Page::Keyboard => text!("Keyboard"),
            Page::Account => text!("Your Account"),
            Page::RecoveryKey => text!("Recovery Key"),
            Page::Wifi => text!("Wi-Fi"),
            Page::Look => text!("Appearance"),
            Page::Privacy => text!("Privacy"),
            Page::Analytics => text!("Bande Analytics"),
            Page::Finishing => text!("Getting Ready"),
        };
        let mut column: Vec<Box<dyn Widget>> = vec![boxed(Label::heading(&id.tr()))];
        column.extend(self.content());
        if let Some(message) = &self.message {
            column.push(boxed(Label::new(message)));
        }
        let mut focus = self.fields.first().copied();
        if self.page != Page::Finishing {
            let mut buttons: Vec<Box<dyn Widget>> = Vec::new();
            if !matches!(self.page, Page::Language | Page::RecoveryKey) {
                buttons.push(boxed(Button::new(&tr!("Back"), self.intent(Intent::Back))));
            }
            let forward = match self.page {
                Page::Analytics => text!("Finish"),
                Page::Wifi if self.answers.network.is_none() => text!("Skip"),
                _ => text!("Continue"),
            };
            let mut next = Button::new(&forward.tr(), self.intent(Intent::Continue)).primary();
            next.set_enabled(!self.working);
            focus = focus.or(Some(next.id()));
            buttons.push(boxed(next));
            column.push(boxed(
                Stack::new(Axis::Horizontal, Spacing::S as f32, 0.0, buttons).trailing(),
            ));
        }
        let stack = Stack::new(Axis::Vertical, Spacing::M as f32, 0.0, column);
        let (width, height) = (self.size.0 as f32, self.size.1 as f32);
        let mut root = FreeLayout::new();
        root.insert(
            stack,
            Rect::new(
                ((width - CARD_WIDTH) / 2.0).max(0.0),
                ((height - CARD_HEIGHT) / 2.0).max(0.0),
                CARD_WIDTH.min(width),
                CARD_HEIGHT.min(height),
            ),
        );
        let mut ui = Ui::new(
            Box::new(root),
            Context::new(self.text.sharing_fonts(), self.appearance.palette()),
        );
        ui.set_appearance(self.appearance);
        if let Some(focus) = focus {
            ui.set_focus(focus);
        }
        self.ui = ui;
    }

    fn request_backdrop(&mut self) {
        self.backdrop_inbox = load_backdrop(
            BackdropSource::Picture(None),
            Variant::Dark,
            self.size,
            vec![Look::Blurred],
        )
        .inspect_err(|error| tracing::warn!("cannot load the wallpaper: {error}"))
        .ok();
    }

    fn secret(&self, index: usize) -> Zeroizing<String> {
        Zeroizing::new(
            self.fields
                .get(index)
                .and_then(|id| self.ui.widget_ref::<SecureField>(*id))
                .map(|field| field.secret().to_owned())
                .unwrap_or_default(),
        )
    }

    fn text(&self, index: usize) -> String {
        self.fields
            .get(index)
            .and_then(|id| self.ui.widget_ref::<TextField>(*id))
            .map(TextField::text)
            .unwrap_or_default()
    }

    fn work(&mut self, job: impl FnOnce() -> Reply + Send + 'static) {
        self.working = true;
        let outbox: Outbox<Reply> = self.inbox.outbox();
        std::thread::spawn(move || {
            outbox.send(job());
        });
    }

    fn apply(&mut self, arguments: Vec<String>, next: Page) {
        self.work(move || {
            let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
            match helper(&arguments, None) {
                Ok(_) => Reply::Done(next),
                Err(error) => Reply::Failed(error),
            }
        });
    }

    fn take_account(&mut self) -> bool {
        let full_name = self.text(0);
        let mut login = self.text(1);
        if login.is_empty() {
            login = suggested_login(&full_name);
        }
        let (password, verify) = (self.secret(2), self.secret(3));
        self.answers.full_name = full_name;
        self.answers.login = login;
        let problem = if !valid_login(&self.answers.login) || self.answers.full_name.is_empty() {
            Some(text!(
                "Enter a name, and an account name of lowercase letters and digits."
            ))
        } else if password.is_empty() {
            Some(text!("Enter a password."))
        } else if *password != *verify {
            Some(text!("The passwords do not match."))
        } else {
            None
        };
        self.message = problem.map(|id| id.tr());
        self.answers.password = password;
        problem.is_none()
    }

    fn create_account(&mut self) {
        let password = self.answers.password.clone();
        let login = self.answers.login.clone();
        let full_name = self.answers.full_name.clone();
        let encrypt = self.answers.encrypt && self.can_encrypt;
        self.work(move || {
            let mut key = None;
            if encrypt {
                match helper(&["encrypt"], Some(&format!("{}\n", password.as_str()))) {
                    Ok(printed) => key = printed.lines().last().map(str::to_owned),
                    Err(error) => return Reply::Failed(error),
                }
            }
            match helper(
                &["account", &login, &full_name],
                Some(&format!("{}\n", password.as_str())),
            ) {
                Ok(_) => Reply::RecoveryKey(key),
                Err(error) => Reply::Failed(error),
            }
        });
    }

    fn scan(&mut self) {
        let Some(system) = self.system.clone() else {
            return;
        };
        let outbox: Outbox<Reply> = self.inbox.outbox();
        std::thread::spawn(move || {
            let manager = NetworkManager::new(&system);
            if let Err(error) = manager.scan() {
                tracing::info!("cannot scan for Wi-Fi: {error}");
            }
            outbox.send(Reply::Networks(manager.networks().unwrap_or_default()));
            std::thread::sleep(SCAN_TIME);
            outbox.send(Reply::Networks(manager.networks().unwrap_or_default()));
        });
    }

    fn show_wifi(&mut self) {
        let wanted = self.system.as_ref().is_some_and(|system| {
            let manager = NetworkManager::new(system);
            manager.has_wifi().unwrap_or(false)
        });
        if wanted {
            self.page = Page::Wifi;
            self.scan();
        } else {
            self.page = Page::Look;
        }
    }

    fn finish(&mut self) {
        self.page = Page::Finishing;
        let login = self.answers.login.clone();
        let password = self.answers.password.clone();
        let look = LOOKS[self.answers.look].1;
        let updates = self.answers.automatic_updates;
        let analytics = self.answers.share_analytics;
        self.work(move || {
            let updates = if updates { "on" } else { "off" };
            let analytics = if analytics { "on" } else { "off" };
            if let Err(error) = helper(&["finish", &login, look, updates, analytics], None) {
                return Reply::Failed(error);
            }
            match sign_in(&login, password) {
                Ok(()) => Reply::SignedIn,
                Err(error) => Reply::Failed(error),
            }
        });
    }

    fn forward(&mut self) {
        self.message = None;
        match self.page {
            Page::Language => {
                let language = self.answers.language.clone();
                self.apply(vec!["locale".to_owned(), language], Page::Keyboard);
            }
            Page::Keyboard => {
                let keyboard = self.answers.keyboard.clone();
                self.apply(vec!["keyboard".to_owned(), keyboard], Page::Account);
            }
            Page::Account => {
                if self.take_account() {
                    self.create_account();
                }
            }
            Page::RecoveryKey => self.show_wifi(),
            Page::Wifi => self.page = Page::Look,
            Page::Look => self.page = Page::Privacy,
            Page::Privacy => self.page = Page::Analytics,
            Page::Analytics => self.finish(),
            Page::Finishing => {}
        }
    }

    fn back(&mut self) {
        self.message = None;
        self.page = match self.page {
            Page::Keyboard => Page::Language,
            Page::Account => Page::Keyboard,
            Page::Look | Page::Wifi => Page::Account,
            Page::Privacy => Page::Look,
            Page::Analytics => Page::Privacy,
            page => page,
        };
    }

    fn join(&mut self) {
        let Some(system) = self.system.clone() else {
            return;
        };
        let Some(network) = self
            .networks
            .iter()
            .find(|network| Some(network.name()) == self.answers.network)
            .cloned()
        else {
            return;
        };
        let password = self.secret(0);
        self.work(move || {
            let password = Some(password.as_str()).filter(|password| !password.is_empty());
            match NetworkManager::new(&system).join(&network, password) {
                Ok(outcome) => Reply::Joined(outcome),
                Err(error) => Reply::Failed(error.to_string()),
            }
        });
    }

    fn settle(&mut self) -> bool {
        let intents: Vec<Intent> = self.intents.borrow_mut().drain(..).collect();
        let mut changed = false;
        for intent in intents {
            let page = self.page;
            match intent {
                Intent::Continue if !self.working => self.forward(),
                Intent::Back if !self.working => self.back(),
                Intent::Language(code) => self.answers.language = code,
                Intent::Keyboard(code) => self.answers.keyboard = code,
                Intent::Network(name) => {
                    self.answers.network = Some(name);
                }
                Intent::Join if !self.working => self.join(),
                Intent::EncryptDisk(on) => self.answers.encrypt = on,
                Intent::Look(index) => {
                    self.answers.look = index;
                    self.appearance.scheme = match LOOKS[index].1 {
                        "light" => Scheme::Light,
                        _ => Scheme::Dark,
                    };
                    self.ui.set_appearance(self.appearance);
                    changed = true;
                }
                Intent::AutomaticUpdates(on) => self.answers.automatic_updates = on,
                Intent::ShareAnalytics(on) => self.answers.share_analytics = on,
                _ => {}
            }
            if self.page != page || self.working {
                self.present();
                changed = true;
            }
        }
        changed
    }

    fn receive(&mut self, reply: Reply) {
        if !matches!(reply, Reply::Networks(_)) {
            self.working = false;
        }
        match reply {
            Reply::Done(page) => self.page = page,
            Reply::RecoveryKey(key) => {
                self.recovery_key = key;
                if self.recovery_key.is_some() {
                    self.page = Page::RecoveryKey;
                } else {
                    self.show_wifi();
                }
            }
            Reply::Failed(error) => {
                tracing::warn!("setup step failed: {error}");
                self.message = Some(tr!(
                    "That step did not work. Try again, or restart this computer."
                ));
                if self.page == Page::Finishing {
                    self.page = Page::Analytics;
                }
            }
            Reply::Networks(networks) => {
                let rows = networks
                    .iter()
                    .map(|network| Row::new(network.name(), vec![Cell::new(network.name())]))
                    .collect();
                self.networks = networks;
                if self.page == Page::Wifi
                    && let Some(id) = self.table
                    && let Some(table) = self.ui.widget_as::<Table>(id)
                {
                    table.set_rows(rows);
                }
                return;
            }
            Reply::Joined(JoinOutcome::Joined) => self.page = Page::Look,
            Reply::Joined(outcome) => self.message = Some(outcome.to_string()),
            Reply::SignedIn => self.signed_in = true,
        }
        self.present();
    }
}

impl App for Assistant {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        if (width, height) != self.size {
            self.size = (width, height);
            self.request_backdrop();
            self.present();
        }
        let mut scene = self.ui.scene(width, height);
        if let Some(image) = &self.backdrop {
            let frame = Rect::new(0.0, 0.0, width as f32, height as f32);
            scene.items.splice(
                0..0,
                [
                    Item::Image {
                        rect: frame,
                        image: image.clone(),
                    },
                    Item::Fill {
                        shape: Shape::rect(frame),
                        paint: self.appearance.palette().scrim.into(),
                    },
                ],
            );
        }
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let changed = match event {
            WindowEvent::Input(input) => {
                let handled = self.ui.handle(&input);
                if !handled
                    && matches!(
                        input,
                        Event::Key {
                            key: Key::Enter,
                            ..
                        }
                    )
                {
                    self.intents.borrow_mut().push(Intent::Continue);
                }
                handled
            }
            WindowEvent::Accessibility(request) => self.ui.perform(&request),
            _ => false,
        };
        if self.settle() || changed {
            window.request_redraw();
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        std::iter::once(self.inbox.fd())
            .chain(self.backdrop_inbox.as_ref().map(Inbox::fd))
            .collect()
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        if index == 0 {
            for reply in self.inbox.drain() {
                self.receive(reply);
            }
        } else if let Some(inbox) = &mut self.backdrop_inbox {
            for (_, image) in inbox.drain() {
                self.backdrop = Some(image);
            }
        }
        window.request_redraw();
    }

    fn deadline(&self) -> Option<Instant> {
        self.ui.deadline()
    }

    fn tick(&mut self, window: &mut Window, _now: Instant) {
        window.request_redraw();
    }

    fn finished(&self) -> bool {
        self.signed_in
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }
}

fn privacy_address() -> Option<String> {
    std::fs::read_to_string(REPORT_SETTINGS)
        .ok()?
        .lines()
        .filter_map(|line| line.split_once('='))
        .find(|(name, _)| name.trim() == "endpoint")
        .map(|(_, endpoint)| format!("{}/privacy", endpoint.trim().trim_end_matches('/')))
}

fn run_assistant() -> Result<(), Box<dyn std::error::Error>> {
    let mut window = Window::open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Overlay,
            anchor: Edges::ALL,
            exclusive_zone: -1,
            keyboard: KeyboardFocus::Exclusive,
            namespace: "greeter".to_owned(),
        }),
        (0, 0),
    )?;
    let text = TextSystem::new();
    let appearance = Appearance {
        scheme: Scheme::Dark,
        ..Appearance::default()
    };
    let mut ui = Ui::new(
        Box::new(FreeLayout::new()),
        Context::new(text.sharing_fonts(), appearance.palette()),
    );
    ui.set_appearance(appearance);
    let can_encrypt = helper(&["encryption-status"], None).is_ok_and(|status| status == "off");
    let mut assistant = Assistant {
        ui,
        appearance,
        backdrop: None,
        backdrop_inbox: None,
        table: None,
        text,
        page: Page::Language,
        answers: Answers {
            language: "en_US.UTF-8".to_owned(),
            keyboard: "us".to_owned(),
            full_name: String::new(),
            login: String::new(),
            password: Zeroizing::new(String::new()),
            encrypt: true,
            network: None,
            look: 2,
            automatic_updates: true,
            share_analytics: false,
        },
        locales: read_supported(),
        layouts: Layouts::load(),
        intents: Rc::default(),
        inbox: Inbox::new()?,
        system: bande_bus::system()
            .inspect_err(|error| tracing::warn!("no system bus: {error}"))
            .ok(),
        networks: Vec::new(),
        recovery_key: None,
        message: None,
        fields: Vec::new(),
        size: (0, 0),
        working: false,
        can_encrypt,
        signed_in: false,
    };
    assistant.present();
    run(&mut window, &mut assistant)?;
    Ok(())
}

fn main() -> ExitCode {
    bande_log::init("bande-setup");
    match run_assistant() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
