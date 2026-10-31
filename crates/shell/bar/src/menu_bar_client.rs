use std::cell::Cell;
use std::collections::HashMap;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::rc::Rc;
use std::thread;
use std::time::Instant;

use crate::dialogs::DIALOGS;
use crate::menu_bar::{HEIGHT, MenuBar, Slot, StatusView};
use crate::menus::{
    AppMenus, app_menu, battery_menu, bluetooth_menu, cascade, input_source_menu, sound_menu,
    system_menu, wifi_menu,
};
use unbound_config::{BUS_NAME, BarEdge, MenuBarPlacement, OBJECT_PATH};
use unbound_datetime::DateFormats;
use unbound_inbox::{Inbox, Outbox};
use unbound_keyboard::Layouts;
use unbound_status::dbusmenu::{APP_MENU_PATH, Node};
use unbound_status::devices::{Control, SoundControl};
use unbound_status::hub::StatusHub;
use unbound_status::session::full_name;
use unbound_status::status::{InputSource, InputSources};
use unbound_status::tray::{Errand, TrayIcon, TrayItem};
use unbound_status::{Command, Update};
use unbound_toolkit::ui::{Event, Menu, MenuItem, PointerButton};
use unbound_toolkit::{
    App, Edges, Image, KeyboardFocus, LayerKind, LayerOptions, Panel, Rect, Role, Scene,
    SessionEnd, TextSystem, Window, WindowEvent,
};
use unbound_xdg::{
    DesktopEntry, ICON_THEMES, Locale, app_name, applications, current_desktops, data_dirs,
    find_app, find_icon, launch_action, remember_running_apps, spawn_detached,
};
use zbus::blocking::Connection;
use zbus::blocking::fdo::PropertiesProxy;

use crate::menu_search_client::SharedHandoff;
use crate::surfaces::{Display, NotificationCenter};

const TRAY_PIXELS: u32 = 32;
const SCROLL_STEP: f32 = 1.0 / 16.0;
const SETTINGS: &str = "org.unbound.Settings";
const MONITOR: &str = "org.unbound.ActivityMonitor";
const BRING_TO_FRONT: &str = "bring-to-front";
const TRAY_ITEM: &str = "tray-item:";
const MENU_BAR_NAMESPACE: &str = "menubar";
const HEADER_ICON: f32 = 24.0;
const CONFIG_INTERFACE: &str = "org.unbound.Config";

pub type Restart = Rc<Cell<bool>>;

pub struct MenuBarClient<V> {
    bar: V,
    display: Display,
    notification_center: NotificationCenter,
    handoff: SharedHandoff,
    layout: (MenuBarPlacement, BarEdge),
    restart: Restart,
    workspaces: (usize, usize),
    anchors: Vec<(Panel, Rect)>,
    entries: Vec<DesktopEntry>,
    inbox: Inbox<Update>,
    status: StatusHub,
    tray: Vec<TrayItem>,
    tray_icons: HashMap<(String, u32), Option<Image>>,
    open_anchor: Option<Rect>,
    app_id: String,
    app_root: Option<Node>,
    user: String,
    layouts: Layouts,
    dates: DateFormats,
}

fn report<E: std::fmt::Display>(what: &str, result: Result<(), E>) {
    if let Err(error) = result {
        tracing::warn!("cannot {what}: {error}");
    }
}

fn confirm_session_end(window: &mut Window, ending: SessionEnd) {
    remember_running_apps(&window.running_apps());
    report("ask to end the session", window.confirm_session_end(ending));
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

    pub fn handoff(&self) -> SharedHandoff {
        self.handoff.clone()
    }

    pub fn restart(&self) -> Restart {
        self.restart.clone()
    }

    fn report_anchors(&mut self, window: &mut Window) {
        if !self.display.main {
            return;
        }
        let edge = self.bar.edge();
        for (panel, slot) in [
            (Panel::ControlCenter, Slot::ControlCenter),
            (Panel::NotificationCenter, Slot::Clock),
        ] {
            let Some(rect) = self.bar.slot_rect(slot) else {
                continue;
            };
            if self.anchors.contains(&(panel, rect)) {
                continue;
            }
            match window.set_panel_anchor(panel, edge, rect) {
                Ok(()) => {
                    self.anchors.retain(|(known, _)| *known != panel);
                    self.anchors.push((panel, rect));
                }
                Err(error) => {
                    tracing::debug!("cannot place the panels beside the bar: {error}");
                    return;
                }
            }
        }
    }

    fn publish_menu(&mut self, window: &Window) {
        let menu = self.cascade(window);
        self.handoff.borrow_mut().menu = menu;
    }

    fn launch(&self, window: &mut Window, app_id: &str, command: &[&str]) {
        let token = window.activation_token(app_id).ok();
        let command: Vec<String> = command.iter().map(|part| (*part).to_owned()).collect();
        if let Err(error) = spawn_detached(app_id, &command, None, token.as_deref()) {
            tracing::warn!("cannot start {}: {error}", command.join(" "));
        }
    }

    fn myself(&self) -> String {
        std::env::current_exe()
            .ok()
            .and_then(|path| path.to_str().map(str::to_owned))
            .unwrap_or_else(|| "unbound-shell".to_owned())
    }

    fn run(&mut self, window: &mut Window, command: Command) {
        let myself = self.myself();
        match command {
            Command::About => self.launch(window, DIALOGS, &[&myself, "--about"]),
            Command::Settings(pane) => match pane {
                Some(pane) => self.launch(window, SETTINGS, &["unbound-settings", &pane]),
                None => self.launch(window, SETTINGS, &["unbound-settings"]),
            },
            Command::ForceQuit => {
                self.launch(window, MONITOR, &["unbound-monitor", "--force-quit"]);
            }
            Command::Restart => confirm_session_end(window, SessionEnd::Restart),
            Command::ShutDown => confirm_session_end(window, SessionEnd::ShutDown),
            Command::LogOut => confirm_session_end(window, SessionEnd::LogOut),
            Command::SetMuted(muted) => {
                self.status.sound(SoundControl::SetMuted(muted));
            }
            Command::UseOutput(name) => {
                self.status.sound(SoundControl::UseOutput(name));
            }
            Command::Item { item, id } => {
                self.status.errand(Errand::Clicked { menu: item, id });
            }
            Command::AboutApp(version) => {
                let app = self.app_id.clone();
                self.launch(window, DIALOGS, &[&myself, "--about-app", &app, &version]);
            }
            Command::HideApp => report("hide the app", window.hide_app(&self.app_id)),
            Command::QuitApp => report("quit the app", window.quit_app(&self.app_id)),
            Command::ShowWindow(identifier) => {
                report(
                    "bring the window forward",
                    window.activate_window(&identifier),
                );
            }
            Command::AppAction(id) => self.run_app_action(window, &id),
            Command::ShowBatteryPercentage(shown) => {
                self.bar.set_battery_percentage(shown);
                thread::spawn(move || {
                    let written = Connection::session().and_then(|session| {
                        config(&session)?.set_property("ShowBatteryPercentage", shown)?;
                        Ok(())
                    });
                    if let Err(error) = written {
                        tracing::warn!("cannot save the battery percentage setting: {error}");
                    }
                });
            }
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
                    self.status.control(Control::Run(Command::Join(ssid)));
                }
            }
            command => self.status.control(Control::Run(command)),
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

    fn entry(&self) -> Option<&DesktopEntry> {
        find_app(&self.entries, &self.app_id)
    }

    fn run_app_action(&mut self, window: &mut Window, id: &str) {
        let Some(entry) = self.entry().cloned() else {
            return;
        };
        let Some(action) = entry.actions.iter().find(|action| action.id == id) else {
            return;
        };
        let token = window.activation_token(&self.app_id).ok();
        if let Err(error) = launch_action(&entry, action, token.as_deref()) {
            tracing::warn!("cannot {} in {}: {error}", action.name, entry.name);
        }
    }

    fn cascade(&self, window: &Window) -> Menu {
        let actions: Vec<(String, String)> = self
            .entry()
            .map(|entry| {
                entry
                    .actions
                    .iter()
                    .map(|action| (action.id.clone(), action.name.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let windows: Vec<(String, String)> = window
            .app_windows(&self.app_id)
            .into_iter()
            .map(|shown| {
                let title = if shown.title.is_empty() {
                    self.bar.app().to_owned()
                } else {
                    shown.title
                };
                (shown.identifier, title)
            })
            .collect();
        let item = format!("{}{APP_MENU_PATH}", self.app_id);
        cascade(&AppMenus {
            name: self.bar.app(),
            root: self.app_root.as_ref(),
            item: &item,
            actions: &actions,
            windows: &windows,
        })
    }

    fn follow_app_icon(&mut self, window: &Window) {
        let pixels = (f64::from(HEADER_ICON) * window.scale()).round() as u32;
        let icon = self
            .entry()
            .and_then(|entry| entry.icon.clone())
            .and_then(|name| find_icon(&name, pixels, &ICON_THEMES, &data_dirs()))
            .and_then(|path| Image::load(&path, pixels));
        self.bar.set_app_icon(icon);
    }

    fn open(&mut self, window: &mut Window, slot: Slot) {
        if slot == Slot::Launcher {
            report("open the launcher", window.show_launcher());
            return;
        }
        let Some(anchor) = self.bar.slot_rect(slot) else {
            return;
        };
        if slot == Slot::App && self.bar.cascades() {
            let menu = self.cascade(window);
            self.bar.set_open(Some(slot));
            report(
                "open the app's menus",
                window.show_menu(menu, self.bar.menu_placement(anchor)),
            );
            window.request_redraw();
            return;
        }
        let status = self.bar.status().clone();
        let local = match slot {
            Slot::System => Some(system_menu(&self.user)),
            Slot::Battery => status
                .battery
                .as_ref()
                .map(|battery| battery_menu(battery, self.bar.battery_percentage())),
            Slot::Wifi => {
                self.status.control(Control::Scan);
                status.wifi.as_ref().map(wifi_menu)
            }
            Slot::Sound => status
                .sound
                .as_ref()
                .map(|sound| sound_menu(sound, &status.capturing)),
            Slot::Bluetooth => status.bluetooth.as_ref().map(bluetooth_menu),
            Slot::InputSource => Some(input_source_menu(&status.input_sources)),
            Slot::TrayMore => Some(self.more_menu(&status.input_sources)),
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
            Slot::ControlCenter | Slot::Wifi | Slot::Sound | Slot::Bluetooth | Slot::Battery => {
                report("open Control Center", window.toggle_control_center());
            }
            Slot::Clock => (self.notification_center)(),
            Slot::App | Slot::AppMenu(_) => {
                if let Some(node) = self.app_menu_node(slot) {
                    let menu = format!("{}{APP_MENU_PATH}", self.app_id);
                    let parent = node.id;
                    self.bar.set_open(Some(slot));
                    window.request_redraw();
                    self.open_anchor = Some(anchor);
                    self.status.errand(Errand::ShowMenu { menu, parent });
                }
            }
            Slot::Tray(index) => self.open_tray(window, index, anchor, false),
            Slot::Workspace(index) if index == self.workspaces.1 => {
                report("show the windows", window.show_windows(""));
            }
            Slot::Workspace(index) => {
                report("switch the workspace", window.switch_workspace(index));
            }
            _ => {}
        }
    }

    fn more_menu(&self, sources: &InputSources) -> Menu {
        let mut items = Vec::new();
        for slot in self.bar.hidden_tray() {
            match slot {
                Slot::Tray(index) => items.extend(
                    self.tray
                        .get(index)
                        .map(|item| MenuItem::new(&item.title, &format!("{TRAY_ITEM}{index}"))),
                ),
                Slot::InputSource => {
                    if !items.is_empty() {
                        items.push(MenuItem::separator());
                    }
                    items.extend(input_source_menu(sources).items);
                }
                _ => {}
            }
        }
        Menu::new(items)
    }

    fn open_tray(&mut self, window: &mut Window, index: usize, anchor: Rect, secondary: bool) {
        let Some(item) = self.tray.get(index) else {
            return;
        };
        let menu_first = item.item_is_menu || !secondary;
        match (&item.menu, menu_first) {
            (Some(menu), true) => {
                self.open_anchor = Some(anchor);
                self.status.errand(Errand::ShowMenu {
                    menu: menu.clone(),
                    parent: 0,
                });
                self.bar.set_open(Some(Slot::Tray(index)));
                window.request_redraw();
            }
            _ => {
                self.status.errand(Errand::Activate {
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
        let scene = self.bar.scene(width, height, scale);
        self.report_anchors(window);
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::ActiveApp(app_id) => {
                if self.bar.set_app(&app_name(&self.entries, &app_id)) {
                    window.request_redraw();
                }
                if app_id != self.app_id {
                    self.app_root = None;
                    self.bar.set_app_menus(Vec::new());
                    self.status.errand(Errand::AppMenu(app_id.clone()));
                    self.app_id = app_id;
                    self.follow_app_icon(window);
                    window.request_redraw();
                }
                self.publish_menu(window);
            }
            WindowEvent::AppsChanged => self.publish_menu(window),
            WindowEvent::FocusShell(namespace) if namespace == MENU_BAR_NAMESPACE => {
                if self.bar.cascades() && self.bar.open() != Some(Slot::App) {
                    self.open(window, Slot::App);
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
                    self.status.sound(SoundControl::Nudge(delta));
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
                if matches!(
                    request.action,
                    accesskit::Action::Click | accesskit::Action::ShowContextMenu
                ) && let Some(slot) = self.bar.slot_of_node(request.target_node, request.action)
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
            WindowEvent::ActiveOutput(output) => {
                let key = output.is_empty() || output == self.display.output;
                if self.bar.set_key_display(key) {
                    window.request_redraw();
                }
            }
            WindowEvent::Workspaces { count, active } => {
                self.workspaces = (count, active);
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
                Update::Capturing(capturing) => self.bar.status_mut().capturing = capturing,
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
                Update::BarLayout { menu_bar, edge } => {
                    let wanted = (
                        MenuBarPlacement::from_keyword(&menu_bar).unwrap_or(self.layout.0),
                        BarEdge::from_keyword(&edge).unwrap_or(self.layout.1),
                    );
                    if wanted.0.combines(wanted.1) != self.layout.0.combines(self.layout.1) {
                        self.restart.set(true);
                    }
                }
                Update::Unread(unread) => {
                    self.bar.set_unread(unread);
                }
                Update::BatteryPercentage(shown) => {
                    self.bar.set_battery_percentage(shown);
                }
                Update::ClockFormat(format) => {
                    self.dates = DateFormats::from_env().with_clock_format(&format);
                    self.bar.set_time(&jiff::Zoned::now(), &self.dates);
                }
                Update::Minute => {
                    self.bar.set_time(&jiff::Zoned::now(), &self.dates);
                }
                Update::AppMenuChanged => {
                    if !self.app_id.is_empty() {
                        self.status.errand(Errand::AppMenu(self.app_id.clone()));
                    }
                }
                Update::AppMenu { app, root } => {
                    if app == self.app_id {
                        self.app_root = root;
                        let titles = self.app_menu_titles();
                        self.bar.set_app_menus(titles);
                        self.publish_menu(window);
                    }
                }
            }
        }
        window.request_redraw();
    }

    fn deadline(&self) -> Option<Instant> {
        (!self.handoff.borrow().chosen.is_empty() || self.restart.get()).then(Instant::now)
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.bar.accessibility())
    }

    fn tick(&mut self, window: &mut Window, _now: Instant) {
        let chosen = std::mem::take(&mut self.handoff.borrow_mut().chosen);
        for action in chosen {
            if let Some(command) = Command::parse(&action) {
                self.run(window, command);
            }
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
        namespace: MENU_BAR_NAMESPACE.to_owned(),
    })
}

pub const MENU_BAR_SIZE: (u32, u32) = (0, HEIGHT);

impl MenuBarClient<MenuBar> {
    pub fn new(
        window: &mut Window,
        text: TextSystem,
        place: (Display, NotificationCenter),
        status: &StatusHub,
        layout: (MenuBarPlacement, BarEdge),
    ) -> std::io::Result<Self> {
        let bar = MenuBar::new(window.appearance().palette(), text, String::new());
        MenuBarClient::with_view(bar, place, status, layout)
    }
}

impl<V: StatusView> MenuBarClient<V> {
    pub fn with_view(
        mut view: V,
        (display, notification_center): (Display, NotificationCenter),
        status: &StatusHub,
        layout: (MenuBarPlacement, BarEdge),
    ) -> std::io::Result<Self> {
        let entries = applications(
            &data_dirs(),
            Locale::from_env().as_ref(),
            &current_desktops(),
        );
        let inbox = Inbox::new()?;
        status.subscribe(inbox.outbox());
        let dates = DateFormats::from_env();
        view.set_time(&jiff::Zoned::now(), &dates);
        Ok(Self {
            bar: view,
            display,
            notification_center,
            handoff: SharedHandoff::default(),
            layout,
            restart: Restart::default(),
            workspaces: (1, 0),
            anchors: Vec::new(),
            entries,
            inbox,
            status: status.clone(),
            tray: Vec::new(),
            tray_icons: HashMap::new(),
            open_anchor: None,
            app_id: String::new(),
            app_root: None,
            user: full_name(),
            layouts: Layouts::load(),
            dates,
        })
    }
}

pub fn status_hub(
    session: Option<&Connection>,
    system: Option<&Connection>,
) -> std::io::Result<StatusHub> {
    let hub = StatusHub::start(session, system)?;
    if let Some(session) = session {
        let (config, outbox) = (session.clone(), hub.publisher());
        thread::spawn(move || {
            if let Err(error) = follow_config(&config, &outbox) {
                tracing::warn!("the bar does not follow its settings: {error}");
            }
        });
    }
    Ok(hub)
}

fn config(session: &Connection) -> zbus::Result<zbus::blocking::Proxy<'static>> {
    zbus::blocking::proxy::Builder::new(session)
        .destination(BUS_NAME)?
        .path(OBJECT_PATH)?
        .interface(CONFIG_INTERFACE)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
}

fn follow_config(session: &Connection, outbox: &Outbox<Update>) -> zbus::Result<()> {
    let config = config(session)?;
    let changes = PropertiesProxy::builder(session)
        .destination(BUS_NAME)?
        .path(OBJECT_PATH)?
        .build()?
        .receive_properties_changed()?;
    let publish = || -> zbus::Result<bool> {
        Ok(outbox.send(Update::BatteryPercentage(
            config.get_property("ShowBatteryPercentage")?,
        )) && outbox.send(Update::ClockFormat(config.get_property("ClockFormat")?))
            && outbox.send(Update::BarLayout {
                menu_bar: config.get_property("BarMenuBar")?,
                edge: config.get_property("BarEdge")?,
            }))
    };
    if !publish()? {
        return Ok(());
    }
    for _ in changes {
        if !publish()? {
            break;
        }
    }
    Ok(())
}
