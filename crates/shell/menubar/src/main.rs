use std::cell::Cell;
use std::collections::HashMap;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

use unbound_datetime::DateFormats;
use unbound_inbox::Inbox;
use unbound_menubar::dbusmenu::{APP_MENU_PATH, Node};
use unbound_menubar::devices::{Control, SoundControl, end_session, run_controls, run_sound};
use unbound_menubar::menus::{
    Command, battery_menu, bluetooth_menu, sound_menu, system_menu, wifi_menu,
};
use unbound_menubar::session::{COUNTDOWN, Ending, facts};
use unbound_menubar::tray::{Errand, TrayIcon, TrayItem, host, run_errands};
use unbound_menubar::{HEIGHT, L10N, MenuBar, Slot, Update, app_name};
use unbound_toolkit::ui::{
    Axis, Button, Event, Key, Label, Menu, MenuItem, PointerButton, Stack, Ui, Widget,
};
use unbound_toolkit::{
    App, Edges, Image, Item, KeyboardFocus, LayerKind, LayerOptions, MenuPlacement, Palette, Point,
    Rect, Role, Scene, TextAlign, TextStyle, TextSystem, Window, WindowEvent,
};
use unbound_xdg::{Locale, applications, current_desktops, data_dirs, find_icon, spawn_detached};
use zbus::blocking::Connection;

const CLOCK_TICK: Duration = Duration::from_secs(1);
const TRAY_PIXELS: u32 = 32;
const SCROLL_STEP: f32 = 1.0 / 16.0;
const SETTINGS: &str = "org.unbound.Settings";
const MONITOR: &str = "org.unbound.ActivityMonitor";
const BRING_TO_FRONT: &str = "bring-to-front";
const BLUR_WIDTH: f32 = 16384.0;
const ABOUT_SIZE: (u32, u32) = (280, 190);
const ABOUT_ICON: f32 = 64.0;
const ABOUT_PADDING: f32 = 28.0;
const ABOUT_GAP: f32 = 14.0;
const ABOUT_NAME_SIZE: f32 = 15.0;
const ABOUT_VERSION_SIZE: f32 = 11.0;

struct Bar {
    bar: MenuBar,
    names: Vec<(String, String)>,
    inbox: Inbox<Update>,
    controls: Option<Sender<Control>>,
    sound: Sender<SoundControl>,
    errands: Sender<Errand>,
    tray: Vec<TrayItem>,
    tray_icons: HashMap<(String, u32), Option<Image>>,
    app_id: String,
    app_root: Option<Node>,
    user: String,
    dates: DateFormats,
    next_clock: Instant,
}

fn report<E: std::fmt::Display>(what: &str, result: Result<(), E>) {
    if let Err(error) = result {
        tracing::warn!("cannot {what}: {error}");
    }
}

fn tray_image(icon: &TrayIcon, pixels: u32) -> Option<Image> {
    match icon {
        TrayIcon::Missing => None,
        TrayIcon::Named { name, theme_path } => {
            let mut directories = data_dirs();
            if let Some(path) = theme_path {
                directories.insert(0, PathBuf::from(path));
            }
            let path = find_icon(name, pixels, &[], &directories)?;
            Image::load(&path, pixels)
        }
        TrayIcon::Pixels {
            width,
            height,
            rgba,
        } => Image::from_rgba(*width, *height, rgba.clone()),
    }
}

impl Bar {
    fn launch(&self, window: &mut Window, app_id: &str, command: &[&str]) {
        let token = window.activation_token(app_id).ok();
        let command: Vec<String> = command.iter().map(|part| (*part).to_owned()).collect();
        if let Err(error) = spawn_detached(&command, None, token.as_deref()) {
            tracing::warn!("cannot start {}: {error}", command.join(" "));
        }
    }

    fn myself(&self) -> String {
        std::env::current_exe()
            .ok()
            .and_then(|path| path.to_str().map(str::to_owned))
            .unwrap_or_else(|| "unbound-menubar".to_owned())
    }

    fn control(&self, control: Control) {
        if let Some(controls) = &self.controls {
            let _ = controls.send(control);
        }
    }

    fn run(&mut self, window: &mut Window, command: Command) {
        let myself = self.myself();
        match command {
            Command::About => self.launch(window, "", &[&myself, "--about"]),
            Command::Settings(pane) => match pane {
                Some(pane) => self.launch(window, SETTINGS, &["unbound-settings", &pane]),
                None => self.launch(window, SETTINGS, &["unbound-settings"]),
            },
            Command::ForceQuit => {
                self.launch(window, MONITOR, &["unbound-monitor", "--force-quit"]);
            }
            Command::Restart => self.launch(window, "", &[&myself, "--confirm", "restart"]),
            Command::ShutDown => self.launch(window, "", &[&myself, "--confirm", "shut-down"]),
            Command::LogOut => self.launch(window, "", &[&myself, "--confirm", "log-out"]),
            Command::SetMuted(muted) => {
                let _ = self.sound.send(SoundControl::SetMuted(muted));
            }
            Command::UseOutput(name) => {
                let _ = self.sound.send(SoundControl::UseOutput(name));
            }
            Command::Item { item, id } => {
                let _ = self.errands.send(Errand::Clicked { menu: item, id });
            }
            Command::AboutApp(version) => {
                let app = self.app_id.clone();
                self.launch(window, "", &[&myself, "--about-app", &app, &version]);
            }
            Command::HideApp => report("hide the app", window.hide_app(&self.app_id)),
            Command::HideOthers => {
                for app in window.running_apps() {
                    if app != self.app_id {
                        report("hide an app", window.hide_app(&app));
                    }
                }
            }
            Command::ShowAll => {
                for app in window.running_apps() {
                    report("show an app", window.show_app(&app));
                }
            }
            Command::Join(ssid) => {
                let needs_password = self
                    .bar
                    .status()
                    .wifi
                    .as_ref()
                    .and_then(|wifi| wifi.networks.iter().find(|network| network.ssid == ssid))
                    .is_some_and(|network| network.security.needs_password() && !network.known);
                if needs_password {
                    self.launch(window, SETTINGS, &["unbound-settings", "wifi"]);
                } else {
                    self.control(Control::Run(Command::Join(ssid)));
                }
            }
            command => self.control(Control::Run(command)),
        }
    }

    fn app_menu_titles(&self) -> Vec<String> {
        self.app_root
            .as_ref()
            .map(|root| {
                root.children
                    .iter()
                    .skip(1)
                    .filter(|child| child.visible && !child.separator)
                    .map(|child| child.label.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn app_menu_node(&self, slot: Slot) -> Option<&Node> {
        let root = self.app_root.as_ref()?;
        let visible = || {
            root.children
                .iter()
                .filter(|child| child.visible && !child.separator)
        };
        match slot {
            Slot::App => root.children.first(),
            Slot::AppMenu(index) => visible().skip(1).nth(index),
            _ => None,
        }
    }

    fn open(&mut self, window: &mut Window, slot: Slot) {
        let Some(anchor) = self.bar.slot_rect(slot) else {
            return;
        };
        let status = self.bar.status().clone();
        let local = match slot {
            Slot::System => Some(system_menu(&self.user)),
            Slot::Battery => status.battery.as_ref().map(battery_menu),
            Slot::Wifi => {
                self.control(Control::Scan);
                status.wifi.as_ref().map(wifi_menu)
            }
            Slot::Sound => status.sound.as_ref().map(sound_menu),
            Slot::Bluetooth => status.bluetooth.as_ref().map(bluetooth_menu),
            Slot::App if self.app_root.is_none() => Some(Menu::new(vec![MenuItem::new(
                self.bar.app(),
                BRING_TO_FRONT,
            )])),
            _ => None,
        };
        if let Some(menu) = local {
            self.bar.set_open(Some(slot));
            report(
                "open the menu",
                window.show_menu(menu, MenuPlacement::Below(anchor)),
            );
            window.request_redraw();
            return;
        }
        match slot {
            Slot::ControlCenter => {
                report("open Control Center", window.toggle_control_center());
            }
            Slot::Clock => {
                let _ = self.errands.send(Errand::ToggleNotificationCenter);
            }
            Slot::App | Slot::AppMenu(_) => {
                if let Some(node) = self.app_menu_node(slot) {
                    let menu = format!("{}{APP_MENU_PATH}", self.app_id);
                    let parent = node.id;
                    self.bar.set_open(Some(slot));
                    window.request_redraw();
                    let _ = self.errands.send(Errand::ShowMenu {
                        menu,
                        parent,
                        anchor,
                    });
                }
            }
            Slot::Tray(index) => self.open_tray(window, index, anchor, false),
            _ => {}
        }
    }

    fn open_tray(&mut self, window: &mut Window, index: usize, anchor: Rect, secondary: bool) {
        let Some(item) = self.tray.get(index) else {
            return;
        };
        let menu_first = item.item_is_menu || !secondary;
        match (&item.menu, menu_first) {
            (Some(menu), true) => {
                let _ = self.errands.send(Errand::ShowMenu {
                    menu: menu.clone(),
                    parent: 0,
                    anchor,
                });
                self.bar.set_open(Some(Slot::Tray(index)));
                window.request_redraw();
            }
            _ => {
                let _ = self.errands.send(Errand::Activate {
                    item: item.id.clone(),
                    x: (anchor.x + anchor.width / 2.0) as i32,
                    y: (anchor.y + anchor.height) as i32,
                });
            }
        }
    }

    fn refresh_tray_icons(&mut self, window: &Window) {
        let pixels = ((f64::from(TRAY_PIXELS) / 2.0) * window.scale()).round() as u32;
        let icons = self
            .tray
            .iter()
            .map(|item| {
                let icon = self
                    .tray_icons
                    .entry((format!("{}{:?}", item.id, item.icon), pixels))
                    .or_insert_with(|| tray_image(&item.icon, pixels.max(16)))
                    .clone();
                (item.title.clone(), icon)
            })
            .collect();
        self.bar.set_tray(icons);
    }

    fn pointer_down(
        &mut self,
        window: &mut Window,
        position: unbound_toolkit::Point,
        button: PointerButton,
    ) {
        let Some(slot) = self.bar.slot_at(position) else {
            return;
        };
        if self.bar.open() == Some(slot) {
            window.close_menu();
            self.bar.set_open(None);
            window.request_redraw();
            return;
        }
        if let (Slot::Tray(index), PointerButton::Secondary) = (slot, button)
            && let Some(anchor) = self.bar.slot_rect(slot)
        {
            self.open_tray(window, index, anchor, true);
            return;
        }
        self.open(window, slot);
    }
}

impl App for Bar {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let scale = window.scale();
        self.bar.scene(width, height, scale)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::ActiveApp(app_id) => {
                if self.bar.set_app(app_name(&app_id, &self.names)) {
                    window.request_redraw();
                }
                if app_id != self.app_id {
                    self.app_root = None;
                    self.bar.set_app_menus(Vec::new());
                    let _ = self.errands.send(Errand::AppMenu(app_id.clone()));
                    self.app_id = app_id;
                }
            }
            WindowEvent::Input(Event::PointerDown(pointer)) => {
                self.pointer_down(window, pointer.position, pointer.button);
            }
            WindowEvent::Input(Event::PointerMove(pointer)) => {
                if let (Some(open), Some(slot)) =
                    (self.bar.open(), self.bar.slot_at(pointer.position))
                    && open != slot
                    && !matches!(slot, Slot::ControlCenter | Slot::Clock)
                {
                    window.close_menu();
                    self.bar.set_open(None);
                    self.open(window, slot);
                }
            }
            WindowEvent::Input(Event::Scroll(scroll)) => {
                if self.bar.slot_at(scroll.position) == Some(Slot::Sound) && scroll.delta.y != 0.0 {
                    let delta = -scroll.delta.y.signum() * SCROLL_STEP;
                    let _ = self.sound.send(SoundControl::Nudge(delta));
                }
            }
            WindowEvent::MenuAction(action) => {
                self.bar.set_open(None);
                window.request_redraw();
                if action == BRING_TO_FRONT {
                    let app = self.app_id.clone();
                    report("bring the app forward", window.activate_app(&app));
                } else if let Some(command) = Command::parse(&action) {
                    self.run(window, command);
                }
            }
            WindowEvent::Accessibility(request) => {
                if request.action == accesskit::Action::Click
                    && let Some(slot) = self.bar.slot_of_node(request.target_node)
                {
                    self.open(window, slot);
                }
            }
            WindowEvent::MenuClosed => {
                if self.bar.set_open(None) {
                    window.request_redraw();
                }
            }
            WindowEvent::AppearanceChanged(appearance) => {
                self.bar.set_palette(appearance.palette());
                window.request_redraw();
            }
            _ => {}
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.inbox.fd()]
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) {
        for update in self.inbox.drain() {
            match update {
                Update::Devices((battery, wifi, bluetooth)) => {
                    let status = self.bar.status_mut();
                    status.battery = battery;
                    status.wifi = wifi;
                    status.bluetooth = bluetooth;
                }
                Update::Sound(sound) => self.bar.status_mut().sound = sound,
                Update::Tray(items) => {
                    self.tray = items;
                    self.refresh_tray_icons(window);
                }
                Update::Menu { menu, anchor } => {
                    if self.bar.open().is_some() {
                        report(
                            "open the menu",
                            window.show_menu(menu, MenuPlacement::Below(anchor)),
                        );
                    }
                }
                Update::AppMenuChanged => {
                    if !self.app_id.is_empty() {
                        let _ = self.errands.send(Errand::AppMenu(self.app_id.clone()));
                    }
                }
                Update::AppMenu { app, root } => {
                    if app == self.app_id {
                        self.app_root = root;
                        let titles = self.app_menu_titles();
                        self.bar.set_app_menus(titles);
                    }
                }
            }
        }
        window.request_redraw();
    }

    fn deadline(&self) -> Option<Instant> {
        Some(self.next_clock)
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.bar.accessibility())
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        self.next_clock = now + CLOCK_TICK;
        if self.bar.set_clock(self.dates.clock(&jiff::Zoned::now())) {
            window.request_redraw();
        }
    }
}

fn run_bar() -> Result<(), Box<dyn std::error::Error>> {
    let names = applications(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    )
    .into_iter()
    .map(|entry| (entry.app_id().to_owned(), entry.name))
    .collect();
    let inbox = Inbox::new()?;
    let controls = match Connection::system() {
        Ok(system) => {
            let (controls, received) = mpsc::channel();
            let outbox = inbox.outbox();
            thread::spawn(move || run_controls(&system, &received, &outbox));
            Some(controls)
        }
        Err(error) => {
            tracing::warn!("no system bus: {error}");
            None
        }
    };
    let (sound, sound_controls) = mpsc::channel();
    let outbox = inbox.outbox();
    thread::spawn(move || run_sound(&sound_controls, &outbox));
    let (errands, errand_queue) = mpsc::channel();
    match Connection::session() {
        Ok(session) => {
            let (hosting, outbox) = (session.clone(), inbox.outbox());
            thread::spawn(move || {
                if let Err(error) = host(&hosting, &outbox) {
                    tracing::warn!("cannot host status items: {error}");
                }
            });
            let outbox = inbox.outbox();
            thread::spawn(move || run_errands(&session, &errand_queue, &outbox));
        }
        Err(error) => tracing::warn!("no session bus: {error}"),
    }
    let mut window = Window::open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Top,
            anchor: Edges {
                top: true,
                left: true,
                right: true,
                bottom: false,
            },
            exclusive_zone: HEIGHT as i32,
            keyboard: KeyboardFocus::None,
            namespace: "menubar".to_owned(),
        }),
        (0, HEIGHT),
    )?;
    let blurred = window
        .set_background_blur(&[Rect::new(0.0, 0.0, BLUR_WIDTH, HEIGHT as f32)])
        .inspect_err(|error| tracing::info!("the menu bar is not blurred: {error}"))
        .is_ok();
    let dates = DateFormats::from_env();
    let mut bar = Bar {
        bar: MenuBar::new(
            window.appearance().palette(),
            TextSystem::new(),
            dates.clock(&jiff::Zoned::now()),
        ),
        names,
        inbox,
        controls,
        sound,
        errands,
        tray: Vec::new(),
        tray_icons: HashMap::new(),
        app_id: String::new(),
        app_root: None,
        user: user_name(),
        dates,
        next_clock: Instant::now() + CLOCK_TICK,
    };
    bar.bar.set_translucent(blurred);
    unbound_toolkit::run(&mut window, &mut bar)?;
    Ok(())
}

fn user_name() -> String {
    let login = std::env::var("USER").unwrap_or_default();
    std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|passwd| {
            passwd.lines().find_map(|line| {
                let fields: Vec<&str> = line.split(':').collect();
                (fields.first() == Some(&login.as_str()))
                    .then(|| fields.get(4)?.split(',').next().map(str::to_owned))
                    .flatten()
                    .filter(|name| !name.is_empty())
            })
        })
        .unwrap_or(login)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Choice {
    Undecided,
    Cancel,
    Confirm,
}

struct Dialog {
    ui: Ui,
    choice: Rc<Cell<Choice>>,
    ending: Option<(Ending, Instant)>,
    next_second: Option<Instant>,
}

fn dialog_ui(lines: Vec<Box<dyn Widget>>, buttons: Vec<Box<dyn Widget>>, window: &Window) -> Ui {
    let mut rows = lines;
    if !buttons.is_empty() {
        rows.push(Box::new(Stack::new(Axis::Horizontal, 12.0, 0.0, buttons)));
    }
    Ui::new(
        Box::new(Stack::new(Axis::Vertical, 10.0, 20.0, rows)),
        unbound_toolkit::ui::Context::new(TextSystem::new(), window.appearance().palette()),
    )
}

fn confirmation(ending: Ending, left: Duration, choice: &Rc<Cell<Choice>>, window: &Window) -> Ui {
    let cancel = Rc::clone(choice);
    let confirm = Rc::clone(choice);
    dialog_ui(
        vec![
            Box::new(Label::new(&ending.question())),
            Box::new(Label::new(&ending.countdown(left))),
        ],
        vec![
            Box::new(Button::new(&L10N.tr("button-cancel"), move || {
                cancel.set(Choice::Cancel);
            })),
            Box::new(Button::new(&ending.button(), move || {
                confirm.set(Choice::Confirm);
            })),
        ],
        window,
    )
}

impl App for Dialog {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.ui.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match &event {
            WindowEvent::Input(Event::Key {
                key: Key::Escape, ..
            }) => self.choice.set(Choice::Cancel),
            WindowEvent::Input(Event::Key {
                key: Key::Enter, ..
            }) if self.ending.is_some() => self.choice.set(Choice::Confirm),
            WindowEvent::Input(input) => {
                if self.ui.handle(input) {
                    window.request_redraw();
                }
            }
            WindowEvent::Accessibility(request) => {
                if self.ui.perform(request) {
                    window.request_redraw();
                }
            }
            WindowEvent::AppearanceChanged(appearance) => {
                self.ui.set_appearance(*appearance);
                window.request_redraw();
            }
            _ => {}
        }
        self.decide(window);
    }

    fn deadline(&self) -> Option<Instant> {
        self.next_second
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        let Some((ending, until)) = self.ending else {
            return;
        };
        let left = until.saturating_duration_since(now);
        if left.is_zero() {
            self.choice.set(Choice::Confirm);
        } else {
            self.ui = confirmation(
                ending,
                left + Duration::from_millis(999),
                &self.choice,
                window,
            );
            self.next_second = Some(now + Duration::from_secs(1));
            window.request_redraw();
        }
        self.decide(window);
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.ui.accessibility())
    }
}

impl Dialog {
    fn decide(&mut self, window: &mut Window) {
        match self.choice.get() {
            Choice::Undecided => {}
            Choice::Cancel => window.close(),
            Choice::Confirm => {
                if let Some((ending, _)) = self.ending {
                    let result =
                        Connection::system().and_then(|system| end_session(&system, ending));
                    report("end the session", result);
                }
                window.close();
            }
        }
    }
}

fn run_confirmation(ending: Ending) -> Result<(), Box<dyn std::error::Error>> {
    let mut window = Window::open(
        Role::toplevel(&ending.button(), "org.unbound.MenuBar"),
        (420, 130),
    )?;
    let choice = Rc::new(Cell::new(Choice::Undecided));
    let now = Instant::now();
    let ui = confirmation(ending, COUNTDOWN, &choice, &window);
    let mut dialog = Dialog {
        ui,
        choice,
        ending: Some((ending, now + COUNTDOWN)),
        next_second: Some(now + Duration::from_secs(1)),
    };
    unbound_toolkit::run(&mut window, &mut dialog)?;
    Ok(())
}

fn run_about() -> Result<(), Box<dyn std::error::Error>> {
    let read = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
    let model = std::fs::read("/proc/device-tree/model")
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    let facts = facts(
        &read("/proc/sys/kernel/hostname"),
        &read("/etc/os-release"),
        model.as_deref(),
        &read("/proc/cpuinfo"),
        &read("/proc/meminfo"),
    );
    let mut window = Window::open(
        Role::toplevel(&L10N.tr("about-title"), "org.unbound.MenuBar"),
        (360, 210),
    )?;
    let row = |label: &str, value: &str| -> Box<dyn Widget> {
        Box::new(Label::new(&format!("{label}    {value}")))
    };
    let mut lines: Vec<Box<dyn Widget>> = vec![Box::new(Label::heading(&facts.name))];
    lines.push(row(&L10N.tr("about-system"), &facts.system));
    if !facts.chip.is_empty() {
        lines.push(row(&L10N.tr("about-chip"), &facts.chip));
    }
    lines.push(row(&L10N.tr("about-memory"), &facts.memory));
    lines.push(row(
        &L10N.tr("about-desktop"),
        &format!("Unbound {}", env!("CARGO_PKG_VERSION")),
    ));
    let ui = dialog_ui(lines, Vec::new(), &window);
    let mut dialog = Dialog {
        ui,
        choice: Rc::new(Cell::new(Choice::Undecided)),
        ending: None,
        next_second: None,
    };
    unbound_toolkit::run(&mut window, &mut dialog)?;
    Ok(())
}

struct AppAbout {
    name: String,
    version: String,
    icon: Option<Image>,
    palette: Palette,
    text: TextSystem,
}

impl App for AppAbout {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let width_points = width as f32;
        let centred = |text: &mut TextSystem, content: &str, style: TextStyle| {
            text.paragraph(content, style, Some(width_points), TextAlign::Center)
        };
        let name = centred(
            &mut self.text,
            &self.name,
            TextStyle::new(ABOUT_NAME_SIZE, self.palette.label_primary).weight(TextStyle::SEMIBOLD),
        );
        let version = centred(
            &mut self.text,
            &L10N.tr_args(
                "about-app-version",
                [("version", self.version.clone().into())],
            ),
            TextStyle::new(ABOUT_VERSION_SIZE, self.palette.label_muted),
        );
        let mut items = Vec::new();
        if let Some(icon) = &self.icon {
            items.push(Item::Image {
                rect: Rect::new(
                    ((width_points - ABOUT_ICON) / 2.0).round(),
                    ABOUT_PADDING,
                    ABOUT_ICON,
                    ABOUT_ICON,
                ),
                image: icon.clone(),
            });
        }
        let name_top = ABOUT_PADDING + ABOUT_ICON + ABOUT_GAP;
        let version_top = name_top + name.height() + 4.0;
        items.push(Item::Text {
            origin: Point::new(0.0, name_top),
            paragraph: name,
        });
        items.push(Item::Text {
            origin: Point::new(0.0, version_top),
            paragraph: version,
        });
        Scene {
            width,
            height,
            background: self.palette.surface_background,
            items,
        }
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::Input(Event::Key {
                key: Key::Escape, ..
            }) => window.close(),
            WindowEvent::AppearanceChanged(appearance) => {
                self.palette = appearance.palette();
                window.request_redraw();
            }
            _ => {}
        }
    }
}

fn run_about_app(app_id: &str, version: &str) -> Result<(), Box<dyn std::error::Error>> {
    let directories = data_dirs();
    let entry = applications(
        &directories,
        Locale::from_env().as_ref(),
        &current_desktops(),
    )
    .into_iter()
    .find(|entry| entry.app_id() == app_id);
    let name = entry
        .as_ref()
        .map_or_else(|| app_id.to_owned(), |entry| entry.name.clone());
    let mut window = Window::open(
        Role::toplevel(
            &L10N.tr_args("about-app-title", [("app", name.clone().into())]),
            "org.unbound.MenuBar",
        ),
        ABOUT_SIZE,
    )?;
    let pixels = (f64::from(ABOUT_ICON) * window.scale().max(2.0)).round() as u32;
    let icon = entry
        .and_then(|entry| entry.icon)
        .and_then(|icon| find_icon(&icon, pixels, &[], &directories))
        .and_then(|path| Image::load(&path, pixels));
    let mut about = AppAbout {
        name,
        version: version.to_owned(),
        icon,
        palette: window.appearance().palette(),
        text: TextSystem::new(),
    };
    unbound_toolkit::run(&mut window, &mut about)?;
    Ok(())
}

fn main() -> ExitCode {
    unbound_log::init("unbound-menubar");
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let result = match arguments
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--about"] => run_about(),
        ["--about-app", app_id, version] => run_about_app(app_id, version),
        ["--confirm", keyword] => match Ending::from_keyword(keyword) {
            Some(ending) => run_confirmation(ending),
            None => Err(format!("unknown session ending {keyword}").into()),
        },
        _ => run_bar(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
