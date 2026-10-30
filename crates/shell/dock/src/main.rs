use std::collections::HashMap;
use std::io::Read;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

use accesskit::TreeUpdate;
use unbound_config::{Config, DockEdge, config_path};
use unbound_dock::autostart::{autostart_dir, opens_at_login, set_open_at_login};
use unbound_dock::bus::{self, BusEvent};
use unbound_dock::{
    DockAction, DockApp, DockState, L10N, MenuTarget, Shared, arrange, dock_ui, exclusive_zone,
    scene, surface_size,
};
use unbound_fs::{FolderWatch, Trash};
use unbound_toolkit::ui::{Menu, MenuItem, Ui};
use unbound_toolkit::{
    App, Edges, Image, KeyboardFocus, LayerKind, LayerOptions, MenuPlacement, Point, Rect, Role,
    Scene, TextSystem, Window, WindowError, WindowEvent,
};
use unbound_xdg::{DesktopEntry, Locale, applications, current_desktops, data_dirs, find_icon};

const FILES: &str = "org.unbound.Files";
const TRASH_URI: &str = "trash:///";
const ICON_THEMES: &[&str] = &["Adwaita"];
const MENU_OPEN: &str = "open";
const MENU_KEEP: &str = "keep";
const MENU_LOGIN: &str = "login";
const MENU_SHOW: &str = "show";
const MENU_EMPTY_TRASH: &str = "empty-trash";
const MENU_CONFIRM_EMPTY: &str = "confirm-empty-trash";
const MENU_CANCEL: &str = "cancel";

struct TrashWatch {
    trash: Trash,
    watch: FolderWatch,
}

impl TrashWatch {
    fn start() -> Option<Self> {
        let trash = Trash::for_user()?;
        let watch = FolderWatch::new()
            .inspect_err(|error| tracing::warn!("cannot watch the Trash: {error}"))
            .ok()?;
        let mut trash_watch = Self { trash, watch };
        trash_watch.rewatch();
        Some(trash_watch)
    }

    fn folders(&self) -> Vec<PathBuf> {
        let home = self.trash.home();
        let mut folders = vec![home.root().to_path_buf(), home.files()];
        folders.extend(home.root().parent().map(PathBuf::from));
        folders
    }

    fn rewatch(&mut self) {
        let folders = self.folders();
        self.watch.watch(&folders);
    }
}

struct Dock {
    state: Shared,
    ui: Ui,
    entries: Vec<DesktopEntry>,
    kept: Vec<String>,
    running: Vec<String>,
    icons: HashMap<String, Option<Image>>,
    icon_scale: f64,
    reported: HashMap<String, Rect>,
    input: Option<Rect>,
    zone: i32,
    blur: Option<Vec<Rect>>,
    menu: Option<MenuTarget>,
    confirming: bool,
    keep: Sender<Vec<String>>,
    autostart: Option<PathBuf>,
    bus: Receiver<BusEvent>,
    wake: UnixStream,
    trash: Option<TrashWatch>,
}

impl Dock {
    fn pixels(&self) -> u32 {
        (f64::from(self.state.borrow().icon_size()) * self.icon_scale).ceil() as u32
    }

    fn icon(&mut self, name: Option<&str>) -> Option<Image> {
        let name = name?;
        let pixels = self.pixels();
        self.icons
            .entry(name.to_owned())
            .or_insert_with(|| {
                let path = find_icon(name, pixels, ICON_THEMES, &data_dirs())?;
                Image::load(&path, pixels)
            })
            .clone()
    }

    fn entry(&self, app_id: &str) -> Option<&DesktopEntry> {
        self.entries.iter().find(|entry| entry.app_id() == app_id)
    }

    fn describe(&mut self, app_id: &str, kept: bool) -> Option<DockApp> {
        let running = self.running.iter().any(|running| running == app_id);
        let entry = self.entry(app_id).cloned();
        if kept && entry.is_none() && !running {
            return None;
        }
        let name = entry.as_ref().map_or_else(
            || app_id.rsplit('.').next().unwrap_or(app_id).to_owned(),
            |entry| entry.name.clone(),
        );
        let icon = self.icon(entry.as_ref().and_then(|entry| entry.icon.as_deref()));
        Some(DockApp {
            app_id: app_id.to_owned(),
            name,
            icon,
            kept,
            running,
        })
    }

    fn sync(&mut self) {
        let mut apps = Vec::new();
        for app_id in self.kept.clone() {
            apps.extend(self.describe(&app_id, true));
        }
        for app_id in self.running.clone() {
            if !self.kept.contains(&app_id) {
                apps.extend(self.describe(&app_id, false));
            }
        }
        self.state.borrow_mut().set_apps(apps, Instant::now());
    }

    fn load_trash_icons(&mut self) {
        let empty = self.icon(Some("user-trash"));
        let full = self.icon(Some("user-trash-full"));
        let mut state = self.state.borrow_mut();
        state.set_trash_icons(empty, full);
        if let Some(trash) = &self.trash {
            state.set_trash_full(!trash.trash.is_empty());
        }
    }

    fn follow_scale(&mut self, window: &Window) {
        let scale = window.scale();
        if scale == self.icon_scale {
            return;
        }
        self.icon_scale = scale;
        self.icons.clear();
        self.load_trash_icons();
        self.sync();
    }

    fn run_actions(&mut self, window: &mut Window) {
        let actions = self.state.borrow_mut().take_actions();
        for action in actions {
            match action {
                DockAction::Open(app_id) => self.open(window, &app_id),
                DockAction::OpenTrash => self.open_in_files(window, TRASH_URI),
                DockAction::ShowInFiles(app_id) => self.show_in_files(window, &app_id),
                DockAction::Menu { target, at } => self.show_menu(window, target, at),
                DockAction::Keep(shown) => {
                    let kept = arrange(&self.kept, &shown, |app_id| self.hidden(app_id));
                    self.set_kept(kept);
                }
            }
        }
        window.request_redraw();
    }

    fn open(&mut self, window: &mut Window, app_id: &str) {
        if self.running.iter().any(|running| running == app_id) {
            if let Err(error) = window.activate_app(app_id) {
                tracing::warn!("cannot activate {app_id}: {error}");
            }
            return;
        }
        if self.state.borrow().is_launching(app_id) {
            return;
        }
        let Some(entry) = self.entry(app_id).cloned() else {
            return;
        };
        let token = window.activation_token(app_id).ok();
        match unbound_xdg::launch(&entry, token.as_deref()) {
            Ok(_) => self.state.borrow_mut().launching(app_id, Instant::now()),
            Err(error) => tracing::warn!("cannot open {}: {error}", entry.name),
        }
    }

    fn show_in_files(&mut self, window: &mut Window, app_id: &str) {
        let Some(folder) = self
            .entry(app_id)
            .and_then(|entry| entry.path.parent())
            .map(|folder| folder.to_string_lossy().into_owned())
        else {
            return;
        };
        self.open_in_files(window, &folder);
    }

    fn opens_at_login(&self, app_id: &str) -> bool {
        match (&self.autostart, self.entry(app_id)) {
            (Some(dir), Some(entry)) => opens_at_login(dir, &entry.id),
            _ => false,
        }
    }

    fn hidden(&self, app_id: &str) -> bool {
        self.entry(app_id).is_none() && !self.running.iter().any(|running| running == app_id)
    }

    fn set_kept(&mut self, kept: Vec<String>) {
        if kept == self.kept {
            return;
        }
        self.kept = kept.clone();
        self.sync();
        if self.keep.send(kept).is_err() {
            tracing::warn!("the Dock cannot save the apps kept in it");
        }
    }

    fn trash_full(&self) -> bool {
        self.trash
            .as_ref()
            .is_some_and(|trash| !trash.trash.is_empty())
    }

    fn confirm_empty_trash(&mut self, window: &mut Window, at: Point) {
        let count = self
            .trash
            .as_ref()
            .map_or(0, |trash| trash.trash.items().len());
        let items = vec![
            MenuItem::new(
                &L10N.tr_args("empty-trash-question", [("count", (count as i64).into())]),
                MENU_CANCEL,
            )
            .enabled(false),
            MenuItem::new(&L10N.tr("empty-trash-warning"), MENU_CANCEL).enabled(false),
            MenuItem::separator(),
            MenuItem::new(&L10N.tr("menu-cancel"), MENU_CANCEL),
            MenuItem::new(&L10N.tr("menu-empty-trash"), MENU_CONFIRM_EMPTY),
        ];
        match window.show_menu(Menu::new(items), MenuPlacement::AtPoint(at)) {
            Ok(()) => {
                self.menu = Some(MenuTarget::Trash);
                self.confirming = true;
                self.state.borrow_mut().set_menu_open(true, Instant::now());
            }
            Err(error) => tracing::warn!("cannot ask before emptying the Trash: {error}"),
        }
    }

    fn empty_trash(&mut self) {
        let Some(trash) = &self.trash else {
            return;
        };
        let trash = trash.trash.clone();
        let emptied = thread::Builder::new()
            .name("empty-trash".to_owned())
            .spawn(move || {
                if let Err(error) = trash.empty() {
                    tracing::warn!("cannot empty the Trash: {error}");
                }
            });
        if let Err(error) = emptied {
            tracing::warn!("cannot start emptying the Trash: {error}");
        }
    }

    fn show_menu(&mut self, window: &mut Window, target: MenuTarget, at: Point) {
        self.confirming = false;
        let items = match &target {
            MenuTarget::Trash => vec![
                MenuItem::new(&L10N.tr("menu-open"), MENU_OPEN),
                MenuItem::separator(),
                MenuItem::new(&L10N.tr("menu-empty-trash-ask"), MENU_EMPTY_TRASH)
                    .enabled(self.trash_full()),
            ],
            MenuTarget::App(app_id) => {
                let installed = self.entry(app_id).is_some();
                let running = self.running.iter().any(|running| running == app_id);
                let mut items = Vec::new();
                if !running {
                    items.push(MenuItem::new(&L10N.tr("menu-open"), MENU_OPEN).enabled(installed));
                    items.push(MenuItem::separator());
                }
                items.push(
                    MenuItem::new(&L10N.tr("menu-keep"), MENU_KEEP)
                        .checked(self.kept.contains(app_id)),
                );
                items.push(
                    MenuItem::new(&L10N.tr("menu-login"), MENU_LOGIN)
                        .checked(self.opens_at_login(app_id))
                        .enabled(installed && self.autostart.is_some()),
                );
                items.push(MenuItem::new(&L10N.tr("menu-show"), MENU_SHOW).enabled(installed));
                items
            }
        };
        match window.show_menu(Menu::new(items), MenuPlacement::AtPoint(at)) {
            Ok(()) => {
                self.menu = Some(target);
                self.state.borrow_mut().set_menu_open(true, Instant::now());
            }
            Err(error) => tracing::warn!("cannot show the Dock menu: {error}"),
        }
    }

    fn menu_action(&mut self, window: &mut Window, action: &str) {
        self.state.borrow_mut().set_menu_open(false, Instant::now());
        let Some(target) = self.menu.take() else {
            return;
        };
        match (target, action) {
            (MenuTarget::Trash, MENU_OPEN) => self.open_in_files(window, TRASH_URI),
            (MenuTarget::Trash, MENU_EMPTY_TRASH) => {
                let at = self.state.borrow().trash_menu_point();
                if let Some(at) = at {
                    self.confirm_empty_trash(window, at);
                }
            }
            (MenuTarget::Trash, MENU_CONFIRM_EMPTY) if self.confirming => self.empty_trash(),
            (MenuTarget::App(app_id), MENU_KEEP) => {
                let mut kept = self.kept.clone();
                match kept.iter().position(|known| *known == app_id) {
                    Some(position) => {
                        kept.remove(position);
                    }
                    None => kept.push(app_id),
                }
                self.set_kept(kept);
            }
            (MenuTarget::App(app_id), MENU_OPEN) => self.open(window, &app_id),
            (MenuTarget::App(app_id), MENU_SHOW) => self.show_in_files(window, &app_id),
            (MenuTarget::App(app_id), MENU_LOGIN) => {
                let open = !self.opens_at_login(&app_id);
                if let (Some(dir), Some(entry)) = (&self.autostart, self.entry(&app_id))
                    && let Err(error) = set_open_at_login(dir, &entry.id, &entry.path, open)
                {
                    tracing::warn!("cannot change whether {app_id} opens at login: {error}");
                }
            }
            _ => {}
        }
    }

    fn open_in_files(&mut self, window: &mut Window, location: &str) {
        let Some(entry) = self.entry(FILES).cloned() else {
            tracing::warn!("cannot open {location}: Files is not installed");
            return;
        };
        let command = entry.command().map(|mut command| {
            command.push(location.to_owned());
            command
        });
        let token = window.activation_token(FILES).ok();
        let started = command
            .map_err(|error| error.to_string())
            .and_then(|command| {
                unbound_xdg::spawn_detached(
                    &command,
                    entry.working_directory.as_deref(),
                    token.as_deref(),
                )
                .map_err(|error| error.to_string())
            });
        if let Err(error) = started {
            tracing::warn!("cannot open {location}: {error}");
        }
    }

    fn report(&mut self, window: &mut Window) {
        let blur = self.state.borrow().blur_rects();
        if self.blur.as_ref().is_some_and(|known| *known != blur) {
            let blurred = match window.set_background_blur(&blur) {
                Ok(()) => true,
                Err(error) => {
                    tracing::info!("the Dock is not blurred: {error}");
                    false
                }
            };
            self.blur = blurred.then_some(blur);
            self.state.borrow_mut().set_blurred(blurred);
        }
        let state = self.state.borrow();
        let region = state.input_region();
        if self.input != Some(region) {
            self.input = Some(region);
            window.set_input_region(Some(&[region]));
        }
        let zone = state.exclusive_zone();
        if self.zone != zone {
            self.zone = zone;
            window.set_exclusive_zone(zone);
        }
        let rects = state.icon_rects();
        drop(state);
        let forgotten = self
            .reported
            .keys()
            .filter(|app_id| !rects.iter().any(|(known, _)| known == *app_id))
            .cloned()
            .collect::<Vec<_>>();
        for app_id in forgotten {
            self.reported.remove(&app_id);
            let _ = window.set_icon_rect(&app_id, Rect::default());
        }
        for (app_id, rect) in rects {
            let rounded = Rect::new(
                rect.x.round(),
                rect.y.round(),
                rect.width.round(),
                rect.height.round(),
            );
            if self.reported.get(&app_id) == Some(&rounded) {
                continue;
            }
            match window.set_icon_rect(&app_id, rounded) {
                Ok(()) => {
                    self.reported.insert(app_id, rounded);
                }
                Err(error) => {
                    tracing::debug!("cannot report the icon of {app_id}: {error}");
                    return;
                }
            }
        }
    }

    fn move_to(&mut self, window: &mut Window, edge: DockEdge, now: Instant) {
        if self.state.borrow().edge() == edge {
            return;
        }
        self.state.borrow_mut().set_edge(edge, now);
        self.reported.clear();
        self.input = None;
        window.set_layer_anchor(anchor(edge));
        let icon_size = self.state.borrow().icon_size();
        if let Err(error) = window.set_layer_size(surface_size(edge, icon_size, (0, 0))) {
            tracing::warn!("cannot move the Dock: {error}");
        }
    }

    fn resize(&mut self, window: &mut Window, size: f32) {
        if !self.state.borrow_mut().set_icon_size(size) {
            return;
        }
        let (edge, icon_size) = {
            let state = self.state.borrow();
            (state.edge(), state.icon_size())
        };
        self.icons.clear();
        self.load_trash_icons();
        self.sync();
        if let Err(error) = window.set_layer_size(surface_size(edge, icon_size, (0, 0))) {
            tracing::warn!("cannot resize the Dock: {error}");
        }
    }

    fn receive(&mut self, window: &mut Window) {
        let mut drained = [0; 64];
        let _ = self.wake.read(&mut drained);
        let now = Instant::now();
        for event in self.bus.try_iter().collect::<Vec<_>>() {
            match event {
                BusEvent::IconSize(size) => self.resize(window, size as f32),
                BusEvent::AutoHide(auto_hide) => {
                    self.state.borrow_mut().set_auto_hide(auto_hide, now);
                }
                BusEvent::DockApps(apps) => {
                    if apps != self.kept && !self.state.borrow().is_dragging() {
                        self.kept = apps;
                        self.sync();
                    }
                }
                BusEvent::Edge(edge) => self.move_to(window, edge, now),
                BusEvent::Launcher(app_id, update) => {
                    self.state
                        .borrow_mut()
                        .update_launcher(&app_id, update, now);
                }
            }
        }
        window.request_redraw();
    }
}

impl App for Dock {
    fn scene(&mut self, window: &mut Window) -> Scene {
        self.follow_scale(window);
        let scene = scene(&mut self.ui, window.size());
        self.report(window);
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::Input(input) => {
                if self.ui.handle(&input) {
                    window.request_redraw();
                }
                window.set_cursor(self.ui.cursor());
                self.run_actions(window);
            }
            WindowEvent::Accessibility(request) => {
                if self.ui.perform(&request) {
                    window.request_redraw();
                }
                self.run_actions(window);
            }
            WindowEvent::AppsChanged => {
                self.running = window.running_apps();
                self.sync();
                window.request_redraw();
            }
            WindowEvent::MenuAction(action) => {
                self.menu_action(window, &action);
                window.request_redraw();
            }
            WindowEvent::MenuClosed => {
                self.menu = None;
                self.confirming = false;
                self.state.borrow_mut().set_menu_open(false, Instant::now());
                window.request_redraw();
            }
            WindowEvent::ActiveApp(app_id) => {
                self.state.borrow_mut().activated(&app_id);
                window.request_redraw();
            }
            WindowEvent::AppearanceChanged(appearance) => {
                self.ui.set_appearance(appearance);
                self.state.borrow_mut().set_appearance(appearance);
                window.request_redraw();
            }
            _ => {}
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        let mut sources = vec![self.wake.as_fd()];
        sources.extend(self.trash.as_ref().map(|trash| trash.watch.fd()));
        sources
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        match index {
            0 => self.receive(window),
            _ => {
                if let Some(trash) = &mut self.trash {
                    trash.watch.read();
                }
            }
        }
    }

    fn deadline(&self) -> Option<Instant> {
        let watch = self.trash.as_ref().and_then(|trash| trash.watch.deadline());
        [self.state.borrow().deadline(), watch]
            .into_iter()
            .flatten()
            .min()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        self.state.borrow_mut().tick(now);
        if let Some(trash) = &mut self.trash
            && !trash.watch.take_changed(now).is_empty()
        {
            trash.rewatch();
            let full = !trash.trash.is_empty();
            self.state.borrow_mut().set_trash_full(full);
        }
        window.request_redraw();
    }

    fn animating(&self) -> bool {
        self.ui.is_animating() || self.state.borrow().pulsing(Instant::now())
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }
}

fn anchor(edge: DockEdge) -> Edges {
    Edges {
        left: edge != DockEdge::Right,
        right: edge != DockEdge::Left,
        top: edge != DockEdge::Bottom,
        bottom: edge != DockEdge::Top,
    }
}

fn read_config(config_home: Option<&PathBuf>, home: Option<&PathBuf>) -> Config {
    config_path(
        config_home.map(PathBuf::as_path),
        home.map(PathBuf::as_path),
    )
    .and_then(|path| std::fs::read_to_string(path).ok())
    .and_then(|source| Config::parse(&source).ok())
    .unwrap_or_default()
}

fn run() -> Result<(), WindowError> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config_home = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let config = read_config(config_home.as_ref(), home.as_ref());
    let edge = config.dock.edge;
    let icon_size = config.dock.icon_size as f32;
    let mut window = Window::open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Top,
            anchor: anchor(edge),
            exclusive_zone: exclusive_zone(icon_size).round() as i32,
            keyboard: KeyboardFocus::None,
            namespace: "dock".to_owned(),
        }),
        surface_size(edge, icon_size, (1, 1)),
    )?;
    let (events, bus) = mpsc::channel();
    let (wake, wake_sender) = UnixStream::pair()?;
    wake.set_nonblocking(true)?;
    bus::follow(events, wake_sender);
    let appearance = window.appearance();
    let state = DockState::new(edge, icon_size);
    state.borrow_mut().set_appearance(appearance);
    state
        .borrow_mut()
        .set_auto_hide(config.dock.auto_hide, Instant::now());
    let mut ui = dock_ui(state.clone(), appearance.palette(), TextSystem::new());
    ui.set_appearance(appearance);
    let locale = Locale::from_env();
    let mut dock = Dock {
        state,
        ui,
        entries: applications(&data_dirs(), locale.as_ref(), &current_desktops()),
        kept: config.dock.apps,
        running: window.running_apps(),
        icons: HashMap::new(),
        icon_scale: window.scale(),
        reported: HashMap::new(),
        input: None,
        zone: exclusive_zone(icon_size).round() as i32,
        blur: Some(Vec::new()),
        menu: None,
        confirming: false,
        keep: bus::keep_writer(),
        autostart: autostart_dir(config_home.as_deref(), home.as_deref()),
        bus,
        wake,
        trash: TrashWatch::start(),
    };
    dock.load_trash_icons();
    dock.sync();
    unbound_toolkit::run(&mut window, &mut dock)
}

fn main() -> ExitCode {
    unbound_log::init("unbound-dock");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
