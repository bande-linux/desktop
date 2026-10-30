use std::collections::HashMap;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

use unbound_bar::menubar::{HEIGHT, MenuBar, Slot, StatusView, app_name};
use unbound_bar::menus::{
    app_menu, battery_menu, bluetooth_menu, input_source_menu, sound_menu, system_menu, wifi_menu,
};
use unbound_datetime::DateFormats;
use unbound_inbox::Inbox;
use unbound_keyboard::Layouts;
use unbound_status::dbusmenu::{APP_MENU_PATH, Node};
use unbound_status::devices::{Control, SoundControl, run_controls, run_sound};
use unbound_status::status::{InputSource, InputSources};
use unbound_status::tray::{Errand, TrayIcon, TrayItem, host, run_errands};
use unbound_status::unread::follow_unread;
use unbound_status::{Command, Update};
use unbound_toolkit::ui::{Event, Menu, MenuItem, PointerButton};
use unbound_toolkit::{
    App, Edges, Image, KeyboardFocus, LayerKind, LayerOptions, Rect, Role, Scene, TextSystem,
    Window, WindowEvent,
};
use unbound_xdg::{Locale, applications, current_desktops, data_dirs, find_icon, spawn_detached};
use zbus::blocking::Connection;

const CLOCK_TICK: Duration = Duration::from_secs(1);
const TRAY_PIXELS: u32 = 32;
const SCROLL_STEP: f32 = 1.0 / 16.0;
const SETTINGS: &str = "org.unbound.Settings";
const MONITOR: &str = "org.unbound.ActivityMonitor";
const BRING_TO_FRONT: &str = "bring-to-front";
const TRAY_ITEM: &str = "tray-item:";
const BLUR_WIDTH: f32 = 16384.0;

pub struct MenuBarClient<V> {
    bar: V,
    names: Vec<(String, String)>,
    inbox: Inbox<Update>,
    controls: Option<Sender<Control>>,
    sound: Sender<SoundControl>,
    errands: Sender<Errand>,
    tray: Vec<TrayItem>,
    tray_icons: HashMap<(String, u32), Option<Image>>,
    open_anchor: Option<Rect>,
    app_id: String,
    app_root: Option<Node>,
    user: String,
    layouts: Layouts,
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

impl<V: StatusView> MenuBarClient<V> {
    pub fn view_mut(&mut self) -> &mut V {
        &mut self.bar
    }

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
            .unwrap_or_else(|| "unbound-bar".to_owned())
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
            Command::SelectInputSource(index) => {
                report("switch the input source", window.select_input_source(index));
            }
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
            Slot::InputSource => Some(input_source_menu(&status.input_sources)),
            Slot::TrayMore => Some(Menu::new(
                self.tray
                    .iter()
                    .enumerate()
                    .skip(self.bar.shown_tray_items())
                    .map(|(index, item)| MenuItem::new(&item.title, &format!("{TRAY_ITEM}{index}")))
                    .collect(),
            )),
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
                window.show_menu(menu, self.bar.menu_placement(anchor)),
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
                    self.open_anchor = Some(anchor);
                    let _ = self.errands.send(Errand::ShowMenu { menu, parent });
                }
            }
            Slot::Tray(index) => self.open_tray(window, index, anchor, false),
            Slot::Launcher => report("open the launcher", window.show_launcher()),
            Slot::Workspace(_) => report("show the windows", window.show_windows("")),
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
                self.open_anchor = Some(anchor);
                let _ = self.errands.send(Errand::ShowMenu {
                    menu: menu.clone(),
                    parent: 0,
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
        let Some(slot) = self.bar.slot_at(position, button) else {
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

impl<V: StatusView> App for MenuBarClient<V> {
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
            WindowEvent::Input(Event::PointerLeave) => {
                if self.bar.set_hovered(None) {
                    window.request_redraw();
                }
            }
            WindowEvent::Input(Event::PointerMove(pointer)) => {
                let slot = self.bar.slot_at(pointer.position, PointerButton::Primary);
                if self.bar.set_hovered(slot) {
                    window.request_redraw();
                }
                if let (Some(open), Some(slot)) = (self.bar.open(), slot)
                    && self.bar.slides_between_menus()
                    && open != slot
                    && !matches!(slot, Slot::ControlCenter | Slot::Clock)
                {
                    window.close_menu();
                    self.bar.set_open(None);
                    self.open(window, slot);
                }
            }
            WindowEvent::Input(Event::Scroll(scroll)) => {
                let over_sound = matches!(
                    self.bar.slot_at(scroll.position, PointerButton::Secondary),
                    Some(Slot::Sound | Slot::ControlCenter)
                );
                if over_sound && scroll.delta.y != 0.0 {
                    let delta = -scroll.delta.y.signum() * SCROLL_STEP;
                    let _ = self.sound.send(SoundControl::Nudge(delta));
                }
            }
            WindowEvent::MenuAction(action) => {
                self.bar.set_open(None);
                window.request_redraw();
                if let Some(index) = action
                    .strip_prefix(TRAY_ITEM)
                    .and_then(|index| index.parse::<usize>().ok())
                    && let Some(anchor) = self.bar.slot_rect(Slot::TrayMore)
                {
                    self.open_tray(window, index, anchor, false);
                } else if action == BRING_TO_FRONT {
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
            WindowEvent::Workspaces { count, active } => {
                if self.bar.set_workspaces(count, active) {
                    window.request_redraw();
                }
            }
            WindowEvent::InputSources { sources, active } => {
                let sources = sources
                    .iter()
                    .map(|code| InputSource {
                        name: self.layouts.name(code),
                        short: self.layouts.short(code),
                    })
                    .collect();
                self.bar.status_mut().input_sources = InputSources { sources, active };
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
                Update::Menu { menu, root } => {
                    if self.bar.open().is_some()
                        && let Some(anchor) = self.open_anchor
                    {
                        report(
                            "open the menu",
                            window
                                .show_menu(app_menu(&root, &menu), self.bar.menu_placement(anchor)),
                        );
                    }
                }
                Update::Unread(unread) => {
                    self.bar.set_unread(unread);
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
        if self.bar.set_time(&jiff::Zoned::now(), &self.dates) {
            window.request_redraw();
        }
    }
}

pub fn menu_bar_role() -> Role {
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
    })
}

pub const MENU_BAR_SIZE: (u32, u32) = (0, HEIGHT);

impl MenuBarClient<MenuBar> {
    pub fn new(
        window: &mut Window,
        text: TextSystem,
        session: Option<&Connection>,
        system: Option<&Connection>,
    ) -> std::io::Result<Self> {
        let blurred = window
            .set_background_blur(&[Rect::new(0.0, 0.0, BLUR_WIDTH, HEIGHT as f32)])
            .inspect_err(|error| tracing::info!("the menu bar is not blurred: {error}"))
            .is_ok();
        let mut bar = MenuBar::new(window.appearance().palette(), text, String::new());
        bar.set_translucent(blurred);
        MenuBarClient::with_view(bar, session, system)
    }
}

impl<V: StatusView> MenuBarClient<V> {
    pub fn with_view(
        mut view: V,
        session: Option<&Connection>,
        system: Option<&Connection>,
    ) -> std::io::Result<Self> {
        let names = applications(
            &data_dirs(),
            Locale::from_env().as_ref(),
            &current_desktops(),
        )
        .into_iter()
        .map(|entry| (entry.app_id().to_owned(), entry.name))
        .collect();
        let inbox = Inbox::new()?;
        let controls = system.map(|system| {
            let (controls, received) = mpsc::channel();
            let (system, outbox) = (system.clone(), inbox.outbox());
            thread::spawn(move || run_controls(&system, &received, &outbox));
            controls
        });
        let (sound, sound_controls) = mpsc::channel();
        let outbox = inbox.outbox();
        thread::spawn(move || run_sound(&sound_controls, &outbox));
        let (errands, errand_queue) = mpsc::channel();
        if let Some(session) = session {
            let (hosting, outbox) = (session.clone(), inbox.outbox());
            thread::spawn(move || {
                if let Err(error) = host(&hosting, &outbox) {
                    tracing::warn!("cannot host status items: {error}");
                }
            });
            let (following, outbox) = (session.clone(), inbox.outbox());
            thread::spawn(move || {
                if let Err(error) = follow_unread(&following, &outbox) {
                    tracing::info!("cannot follow unread notifications: {error}");
                }
            });
            let (session, outbox) = (session.clone(), inbox.outbox());
            thread::spawn(move || run_errands(&session, &errand_queue, &outbox));
        }
        let dates = DateFormats::from_env();
        view.set_time(&jiff::Zoned::now(), &dates);
        Ok(Self {
            bar: view,
            names,
            inbox,
            controls,
            sound,
            errands,
            tray: Vec::new(),
            tray_icons: HashMap::new(),
            open_anchor: None,
            app_id: String::new(),
            app_root: None,
            user: user_name(),
            layouts: Layouts::load(),
            dates,
            next_clock: Instant::now() + CLOCK_TICK,
        })
    }
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
