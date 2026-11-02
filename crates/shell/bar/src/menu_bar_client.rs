use std::cell::Cell;
use std::collections::HashMap;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::LazyLock;
use std::thread;
use std::time::Instant;

use crate::dialogs::DIALOGS;
use crate::installed_apps::InstalledApps;
use crate::menu_bar::{HEIGHT, MenuBar, Slot, StatusView};
use crate::menus::{
    AppMenus, app_menu, battery_menu, bluetooth_menu, cascade, clock_menu, input_source_menu,
    sound_menu, standard_app_menu, system_menu, wifi_menu,
};
use bande_config::{BUS_NAME, BarEdge, MenuBarPlacement, OBJECT_PATH};
use bande_datetime::DateFormats;
use bande_inbox::{Inbox, Outbox};
use bande_keyboard::Layouts;
use bande_status::dbusmenu::{APP_MENU_PATH, Node};
use bande_status::devices::{Control, SoundControl};
use bande_status::gtkmenu;
use bande_status::hub::StatusHub;
use bande_status::session::full_name;
use bande_status::status::{InputSource, InputSources};
use bande_status::tray::{Errand, TrayIcon, TrayItem};
use bande_status::unread::{ClockMark, set_do_not_disturb};
use bande_status::{Command, Update};
use bande_toolkit::ui::{Event, Menu, MenuItem, PointerButton};
use bande_toolkit::{
    App, Edges, Image, KeyboardFocus, LayerKind, LayerOptions, MenuPlacement, Panel, Rect, Role,
    Scene, SessionEnd, TextSystem, Window, WindowEvent,
};
use bande_xdg::{
    DesktopEntry, app_name, data_dirs, find_app, find_icon, launch_action, spawn_detached,
};
use zbus::blocking::Connection;
use zbus::blocking::fdo::PropertiesProxy;

use crate::menu_search_client::SharedHandoff;
use crate::surfaces::Display;

const TRAY_PIXELS: u32 = 32;
const SCROLL_STEP: f32 = 1.0 / 16.0;
const SETTINGS: &str = "org.bande.Settings";
const TRAY_ITEM: &str = "tray-item:";
const MENU_BAR_NAMESPACE: &str = "menubar";
const CONFIG_INTERFACE: &str = "org.bande.Config";

pub type Restart = Rc<Cell<bool>>;

static LAYOUTS: LazyLock<Layouts> = LazyLock::new(Layouts::load);

pub struct MenuBarClient<V> {
    bar: V,
    display: Display,
    handoff: SharedHandoff,
    layout: (MenuBarPlacement, BarEdge),
    restart: Restart,
    workspaces: (usize, usize),
    clock_mark: ClockMark,
    anchors: Vec<(Panel, Rect)>,
    apps: InstalledApps,
    inbox: Inbox<Update>,
    status: StatusHub,
    tray: Vec<TrayItem>,
    tray_icons: TrayIcons,
    open_anchor: Option<Rect>,
    app_id: String,
    menu_key: String,
    app_root: Option<Node>,
    menu_loaded: bool,
    waiting_menus: Option<Rect>,
    user: String,
    dates: DateFormats,
}

fn report<E: std::fmt::Display>(what: &str, result: Result<(), E>) {
    if let Err(error) = result {
        tracing::warn!("cannot {what}: {error}");
    }
}

fn confirm_session_end(window: &mut Window, ending: SessionEnd) {
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

type TrayIcons = HashMap<String, (TrayIcon, u32, Option<Image>)>;

fn tray_images(
    items: &[TrayItem],
    known: &mut TrayIcons,
    pixels: u32,
) -> Vec<(String, Option<Image>)> {
    let mut previous = std::mem::take(known);
    items
        .iter()
        .map(|item| {
            let image = match previous.remove(&item.id) {
                Some((icon, size, image)) if icon == item.icon && size == pixels => image,
                _ => tray_image(&item.icon, pixels.max(16)),
            };
            known.insert(item.id.clone(), (item.icon.clone(), pixels, image.clone()));
            (item.title.clone(), image)
        })
        .collect()
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

    fn frontmost_name(&self) -> Option<String> {
        (!self.app_id.is_empty()).then(|| app_name(self.apps.entries(), &self.app_id))
    }

    fn myself(&self) -> String {
        std::env::current_exe()
            .ok()
            .and_then(|path| path.to_str().map(str::to_owned))
            .unwrap_or_else(|| "bande-shell".to_owned())
    }

    fn run(&mut self, window: &mut Window, command: Command) {
        let myself = self.myself();
        match command {
            Command::About => self.launch(window, DIALOGS, &[&myself, "--about"]),
            Command::Settings(pane) => match pane {
                Some(pane) => self.launch(window, SETTINGS, &["bande-settings", &pane]),
                None => self.launch(window, SETTINGS, &["bande-settings"]),
            },
            Command::ForceQuit => report("open Force Quit", window.show_force_quit()),
            Command::ForceQuitApp => {
                report("force quit the app", window.force_quit_app(&self.app_id));
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
            Command::SetDoNotDisturb(on) => {
                thread::spawn(move || {
                    let written =
                        Connection::session().and_then(|session| set_do_not_disturb(&session, on));
                    if let Err(error) = written {
                        tracing::warn!("cannot switch Do Not Disturb: {error}");
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
                    self.launch(window, SETTINGS, &["bande-settings", "wifi"]);
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
            Slot::AppMenu => root.children.first(),
            Slot::AppMenuTitle(index) => visible().skip(1).nth(index),
            _ => None,
        }
    }

    fn entry(&self) -> Option<&DesktopEntry> {
        find_app(self.apps.entries(), &self.app_id)
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
        cascade(&AppMenus {
            name: self.bar.app(),
            root: self.app_root.as_ref(),
            item: &self.menu_key,
            actions: &actions,
            windows: &windows,
        })
    }

    fn follow_menus(&mut self, menu: String) {
        if menu == self.menu_key {
            return;
        }
        self.menu_key = menu;
        self.app_root = None;
        self.menu_loaded = false;
        self.bar.set_app_menus(Vec::new());
        if !self.menu_key.is_empty() {
            self.status.errand(Errand::AppMenu(self.menu_key.clone()));
        }
    }

    fn show_app_menu(&mut self, window: &mut Window, anchor: Rect) {
        let menu = self.cascade(window);
        report(
            "open the app's menus",
            window.show_menu(menu, MenuPlacement::Below(anchor)),
        );
    }

    fn open(&mut self, window: &mut Window, slot: Slot) {
        if slot == Slot::Search {
            report("open the launcher", window.show_launcher());
            return;
        }
        let Some(anchor) = self.bar.slot_rect(slot) else {
            return;
        };
        let status = self.bar.status().clone();
        let local = match slot {
            Slot::BandeMenu => Some(system_menu(&self.user, self.frontmost_name().as_deref())),
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
            Slot::MoreStatusItems => Some(self.more_menu(&status.input_sources)),
            Slot::AppMenu
                if self
                    .app_menu_node(slot)
                    .is_none_or(|node| node.children.is_empty()) =>
            {
                Some(standard_app_menu(self.bar.app()))
            }
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
                report(
                    "open Control Center",
                    window.toggle_panel(Panel::ControlCenter),
                );
            }
            Slot::Clock => report(
                "open the Notification Center",
                window.toggle_panel(Panel::NotificationCenter),
            ),
            Slot::AppMenu | Slot::AppMenuTitle(_) => {
                if let Some(node) = self.app_menu_node(slot) {
                    let menu = self.menu_key.clone();
                    let parent = node.id;
                    self.bar.set_open(Some(slot));
                    window.request_redraw();
                    self.open_anchor = Some(anchor);
                    self.status.errand(Errand::ShowMenu { menu, parent });
                }
            }
            Slot::StatusItem(index) => self.open_tray(window, index, anchor, false),
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
                Slot::StatusItem(index) => items.extend(
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
                self.bar.set_open(Some(Slot::StatusItem(index)));
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
        let icons = tray_images(&self.tray, &mut self.tray_icons, pixels);
        self.bar.set_tray(icons);
    }

    fn pointer_down(
        &mut self,
        window: &mut Window,
        position: bande_toolkit::Point,
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
        if let (Slot::Clock, PointerButton::Secondary) = (slot, button)
            && let Some(anchor) = self.bar.slot_rect(slot)
        {
            self.bar.set_open(Some(slot));
            report(
                "open the clock menu",
                window.show_menu(
                    clock_menu(self.clock_mark == ClockMark::Quiet),
                    self.bar.menu_placement(anchor),
                ),
            );
            window.request_redraw();
            return;
        }
        if let (Slot::StatusItem(index), PointerButton::Secondary) = (slot, button)
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
                if self.bar.set_app(&app_name(self.apps.entries(), &app_id)) {
                    window.request_redraw();
                }
                if app_id != self.app_id {
                    self.app_id = app_id;
                    window.request_redraw();
                }
                self.publish_menu(window);
            }
            WindowEvent::ActiveMenus(menu) => {
                self.follow_menus(menu_key(&self.app_id, menu));
                self.publish_menu(window);
            }
            WindowEvent::AppsChanged => self.publish_menu(window),
            WindowEvent::ShowAppMenu {
                app_id,
                menu,
                anchor,
            } => {
                self.follow_menus(menu_key(&app_id, menu));
                match self.menu_loaded {
                    true => self.show_app_menu(window, anchor),
                    false => self.waiting_menus = Some(anchor),
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
                    && let Some(anchor) = self.bar.slot_rect(Slot::MoreStatusItems)
                {
                    self.open_tray(window, index, anchor, false);
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
                        name: LAYOUTS.name(code),
                        short: LAYOUTS.short(code),
                    })
                    .collect();
                self.bar.status_mut().input_sources = InputSources { sources, active };
                window.request_redraw();
            }
            _ => {}
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        let mut sources = vec![self.inbox.fd()];
        sources.extend(self.apps.fd());
        sources
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        if index > 0 {
            self.apps.read();
            return;
        }
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
                Update::ClockMark(mark) => {
                    self.clock_mark = mark;
                    self.bar.set_clock_mark(mark);
                }
                Update::BatteryPercentage(shown) => {
                    self.bar.set_battery_percentage(shown);
                }
                Update::ClockFormat(format) => {
                    self.dates = DateFormats::from_env().with_clock_format(&format);
                    self.bar.set_time(&bande_datetime::now(), &self.dates);
                }
                Update::Minute => {
                    self.bar.set_time(&bande_datetime::now(), &self.dates);
                }
                Update::AppMenuChanged => {
                    if !self.menu_key.is_empty() {
                        self.status.errand(Errand::AppMenu(self.menu_key.clone()));
                    }
                }
                Update::GtkMenuChanged(sender) => {
                    if gtkmenu::changed_by(&self.menu_key, &sender) {
                        self.status.errand(Errand::AppMenu(self.menu_key.clone()));
                    }
                }
                Update::AppMenu { menu, root } => {
                    if menu == self.menu_key {
                        self.app_root = root;
                        self.menu_loaded = true;
                        let titles = self.app_menu_titles();
                        self.bar.set_app_menus(titles);
                        self.publish_menu(window);
                        if let Some(anchor) = self.waiting_menus.take() {
                            self.show_app_menu(window, anchor);
                        }
                    }
                }
            }
        }
        window.request_redraw();
    }

    fn deadline(&self) -> Option<Instant> {
        let now =
            (!self.handoff.borrow().chosen.is_empty() || self.restart.get()).then(Instant::now);
        now.into_iter().chain(self.apps.deadline()).min()
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.bar.accessibility())
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        if self.apps.reload_if_changed(now) {
            self.bar
                .set_app(&app_name(self.apps.entries(), &self.app_id));
            window.request_redraw();
        }
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
        display: Display,
        status: &StatusHub,
        layout: (MenuBarPlacement, BarEdge),
    ) -> std::io::Result<Self> {
        let bar = MenuBar::new(window.appearance().palette(), text, String::new());
        MenuBarClient::with_view(bar, display, status, layout)
    }
}

impl<V: StatusView> MenuBarClient<V> {
    pub fn with_view(
        mut view: V,
        display: Display,
        status: &StatusHub,
        layout: (MenuBarPlacement, BarEdge),
    ) -> std::io::Result<Self> {
        let inbox = Inbox::new()?;
        status.subscribe(inbox.outbox());
        let dates = DateFormats::from_env();
        view.set_time(&bande_datetime::now(), &dates);
        Ok(Self {
            bar: view,
            display,
            handoff: SharedHandoff::default(),
            layout,
            restart: Restart::default(),
            workspaces: (1, 0),
            clock_mark: ClockMark::Clear,
            anchors: Vec::new(),
            apps: InstalledApps::load(),
            inbox,
            status: status.clone(),
            tray: Vec::new(),
            tray_icons: HashMap::new(),
            open_anchor: None,
            app_id: String::new(),
            menu_key: String::new(),
            app_root: None,
            menu_loaded: false,
            waiting_menus: None,
            user: full_name(),
            dates,
        })
    }
}

fn menu_key(app_id: &str, menu: String) -> String {
    match (menu.is_empty(), app_id.is_empty()) {
        (false, _) => menu,
        (true, true) => String::new(),
        (true, false) => format!("{app_id}{APP_MENU_PATH}"),
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
