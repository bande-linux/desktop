use std::collections::HashMap;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use crate::L10N;
use crate::dialogs::DIALOGS;
use crate::installed_apps::InstalledApps;
use crate::rail::surface_size as rail_surface_size;
use crate::taskbar::{
    MenuTarget, Rail, Shared, TaskbarAction, TaskbarApp, TaskbarState, exclusive_zone, scene,
    surface_size, taskbar_ui,
};
use accesskit::TreeUpdate;
use bande_config::{Bar as BarConfig, BarEdge};
use bande_fs::{FolderWatch, Trash};
use bande_inbox::Inbox;
use bande_taskbar_model::arrange;
use bande_taskbar_model::bus::{self, BusEvent};
use bande_toolkit::ui::{Event, Menu, MenuItem, Ui};
use bande_toolkit::{
    App, DragAction, DragEvent, Edges, Image, KeyboardFocus, LayerKind, LayerOptions,
    MenuPlacement, Point, Rect, Role, Scene, TextSystem, Window, WindowEvent,
};
use bande_xdg::{
    DesktopEntry, ICON_THEMES, app_name, autostart_dir, data_dirs, data_home, find_app,
    find_app_for_process, find_icon, launch_with_arguments, opens_at_login, path_from_file_uri,
    set_open_at_login, write_launcher,
};
use zbus::blocking::Connection;

const FILES: &str = "org.bande.Files";
const HIDDEN: [&str; 2] = [DIALOGS, "org.bande.FileChooser"];
const PROC: &str = "/proc";
const TRASH_URI: &str = "trash:///";
const MENU_OPEN: &str = "open";
const MENU_KEEP: &str = "keep";
const MENU_LOGIN: &str = "login";
const MENU_SHOW: &str = "show";
const MENU_EMPTY_TRASH: &str = "empty-trash";
const MENU_CONFIRM_EMPTY: &str = "confirm-empty-trash";
const MENU_CANCEL: &str = "cancel";
const TASKBAR_NAMESPACE: &str = "taskbar";
const KEYBOARD_GRACE: Duration = Duration::from_millis(60);

const MENU_HIDE: &str = "hide";
const MENU_QUIT: &str = "quit";
const MENU_FORCE_QUIT: &str = "force-quit";
const MENU_WINDOW: &str = "window:";
const MENU_ACTION: &str = "action:";
const MENU_OPEN_APP: &str = "open-app:";

struct TrashWatch {
    trash: Trash,
    watch: FolderWatch,
    checked: Inbox<bool>,
    full: bool,
}

impl TrashWatch {
    fn start() -> Option<Self> {
        let trash = Trash::for_user()?;
        let watch = FolderWatch::new()
            .inspect_err(|error| tracing::warn!("cannot watch the Trash: {error}"))
            .ok()?;
        let checked = Inbox::new()
            .inspect_err(|error| tracing::warn!("cannot watch the Trash: {error}"))
            .ok()?;
        let mut trash_watch = Self {
            trash,
            watch,
            checked,
            full: false,
        };
        trash_watch.rewatch();
        trash_watch.check();
        Some(trash_watch)
    }

    fn check(&self) {
        let (trash, outbox) = (self.trash.clone(), self.checked.outbox());
        let checking = thread::Builder::new()
            .name("trash-check".to_owned())
            .spawn(move || outbox.send(!trash.is_empty()));
        if let Err(error) = checking {
            tracing::warn!("cannot look into the Trash: {error}");
        }
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

pub struct Taskbar {
    state: Shared,
    ui: Ui,
    apps: InstalledApps,
    kept: Vec<String>,
    windows: Vec<(String, String)>,
    running: Vec<String>,
    icons: HashMap<String, Option<Image>>,
    icon_scale: f64,
    reported: HashMap<String, Rect>,
    input: Option<Rect>,
    zone: i32,
    menu: Option<MenuTarget>,
    confirming: bool,
    swallow_release: bool,
    active: Option<String>,
    release_at: Option<Instant>,
    unresponsive: Vec<String>,
    keep: Option<Sender<Vec<String>>>,
    autostart: Option<PathBuf>,
    bus: Inbox<BusEvent>,
    trash: Option<TrashWatch>,
    session: Option<Connection>,
    output: String,
}

impl Taskbar {
    fn pixels(&self) -> u32 {
        (f64::from(self.state.borrow().icon_size()) * self.icon_scale).ceil() as u32
    }

    fn icon(&mut self, name: &str) -> Option<Image> {
        let pixels = self.pixels();
        self.icons
            .entry(name.to_owned())
            .or_insert_with(|| {
                let path = find_icon(name, pixels, &ICON_THEMES, &data_dirs())?;
                Image::load(&path, pixels)
            })
            .clone()
    }

    fn entry(&self, app_id: &str) -> Option<&DesktopEntry> {
        find_app(self.apps.entries(), app_id)
    }

    fn identity(&self, window_app: &str) -> String {
        self.windows
            .iter()
            .find(|(known, _)| known == window_app)
            .map(|(_, identity)| identity.as_str())
            .or_else(|| self.entry(window_app).map(DesktopEntry::app_id))
            .unwrap_or(window_app)
            .to_owned()
    }

    fn window_app(&self, app_id: &str) -> String {
        self.windows
            .iter()
            .find(|(window_app, _)| window_app == app_id)
            .or_else(|| self.windows.iter().find(|(_, identity)| identity == app_id))
            .map_or(app_id, |(window_app, _)| window_app)
            .to_owned()
    }

    fn follow_windows(&mut self, window: &Window) {
        let entries = self.apps.entries();
        let windows: Vec<(String, String)> = window
            .running_apps()
            .into_iter()
            .filter(|window_app| !HIDDEN.contains(&window_app.as_str()))
            .map(|window_app| {
                let identity = find_app(entries, &window_app)
                    .or_else(|| {
                        let pid = window.app_pid(&window_app)?;
                        find_app_for_process(entries, Path::new(PROC), pid)
                    })
                    .map_or_else(|| window_app.clone(), |entry| entry.app_id().to_owned());
                (window_app, identity)
            })
            .collect();
        let mut running = Vec::new();
        for (_, app_id) in &windows {
            if !running.contains(app_id) {
                running.push(app_id.clone());
            }
        }
        self.windows = windows;
        self.running = running;
    }

    fn describe(&mut self, app_id: &str, kept: bool) -> Option<TaskbarApp> {
        let running = self.running.iter().any(|running| running == app_id);
        let entry = self.entry(app_id).cloned();
        if kept && entry.is_none() && !running {
            return None;
        }
        let name = app_name(self.apps.entries(), app_id);
        let icon = self.icon(
            entry
                .as_ref()
                .and_then(|entry| entry.icon.as_deref())
                .unwrap_or(app_id),
        );
        Some(TaskbarApp {
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
        let empty = self.icon("user-trash");
        let full = self.icon("user-trash-full");
        let mut state = self.state.borrow_mut();
        state.set_trash_icons(empty, full);
        if let Some(trash) = &self.trash {
            state.set_trash_full(trash.full);
        }
    }

    fn follow_installed_apps(&mut self, window: &mut Window, now: Instant) {
        if !self.apps.reload_if_changed(now) {
            return;
        }
        self.icons.clear();
        self.load_trash_icons();
        self.follow_windows(window);
        self.sync();
        window.request_redraw();
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
                TaskbarAction::Open(app_id) => self.open(window, &app_id),
                TaskbarAction::OpenTrash => self.open_in_files(window, TRASH_URI),
                TaskbarAction::ShowInFiles(app_id) => self.show_in_files(window, &app_id),
                TaskbarAction::Menu { target, at } => self.show_menu(window, target, at),
                TaskbarAction::ReleaseKeyboard => self.release_keyboard(window),
                TaskbarAction::Keep(shown) => {
                    let shown: Vec<String> = shown
                        .into_iter()
                        .filter(|app_id| {
                            self.kept.contains(app_id) || self.keep_launcher(window, app_id)
                        })
                        .collect();
                    let kept = arrange(&self.kept, &shown, |app_id| self.hidden(app_id));
                    self.set_kept(kept);
                }
            }
        }
        window.request_redraw();
    }

    fn open(&mut self, window: &mut Window, app_id: &str) {
        if self.running.iter().any(|running| running == app_id) {
            if let Err(error) = window.reopen_app(&self.window_app(app_id)) {
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
        match bande_xdg::launch(&entry, token.as_deref()) {
            Ok(_) => self.state.borrow_mut().launching(app_id, Instant::now()),
            Err(error) => tracing::warn!("cannot open {}: {error}", entry.name),
        }
    }

    fn run_action(&mut self, window: &mut Window, app_id: &str, id: &str) {
        let Some(entry) = self.entry(app_id).cloned() else {
            return;
        };
        let Some(action) = entry.actions.iter().find(|action| action.id == id) else {
            return;
        };
        let token = window.activation_token(app_id).ok();
        match bande_xdg::launch_action(&entry, action, token.as_deref()) {
            Ok(_) if !self.running.iter().any(|running| running == app_id) => {
                self.state.borrow_mut().launching(app_id, Instant::now());
            }
            Ok(_) => {}
            Err(error) => tracing::warn!("cannot {} in {}: {error}", action.name, entry.name),
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

    fn drop_action(&self, target: &MenuTarget) -> Option<DragAction> {
        match target {
            MenuTarget::Trash => self.trash.is_some().then_some(DragAction::Move),
            MenuTarget::More(_) => None,
            MenuTarget::App(app_id) => self
                .entry(app_id)
                .filter(|entry| !entry.mime_types.is_empty())
                .map(|_| DragAction::Copy),
        }
    }

    fn drag(&mut self, window: &mut Window, drag: DragEvent) {
        let now = Instant::now();
        let drag = match drag {
            DragEvent::Entered { position, uris, .. } if !uris.is_empty() => {
                DragEvent::Moved(position)
            }
            other => other,
        };
        match drag {
            DragEvent::Moved(position) => {
                let target = self.state.borrow().drop_target(position);
                let action = target.as_ref().and_then(|target| self.drop_action(target));
                window.accept_drag(action);
                let hovered = action.and(target);
                let mut state = self.state.borrow_mut();
                state.drag_inside(true, now);
                state.hover_drop(hovered.as_ref(), now);
            }
            DragEvent::Entered { .. } => window.accept_drag(None),
            DragEvent::Left | DragEvent::Finished(_) => {
                let mut state = self.state.borrow_mut();
                state.hover_drop(None, now);
                state.drag_inside(false, now);
            }
            DragEvent::Dropped {
                position, transfer, ..
            } => {
                let mut state = self.state.borrow_mut();
                state.hover_drop(None, now);
                state.drag_inside(false, now);
                drop(state);
                let files: Vec<PathBuf> = transfer
                    .uris
                    .iter()
                    .filter_map(|uri| path_from_file_uri(uri))
                    .collect();
                let target = self.state.borrow().drop_target(position);
                match target {
                    Some(MenuTarget::Trash) => self.move_to_trash(files),
                    Some(MenuTarget::App(app_id)) => self.open_with(window, &app_id, &files),
                    Some(MenuTarget::More(_)) | None => {}
                }
            }
        }
    }

    fn move_to_trash(&self, files: Vec<PathBuf>) {
        let Some(trash) = &self.trash else {
            return;
        };
        let trash = trash.trash.clone();
        let moved = thread::Builder::new()
            .name("move-to-trash".to_owned())
            .spawn(move || {
                for file in files {
                    if let Err(error) = trash.put(&file) {
                        tracing::warn!("cannot move {} to the Trash: {error}", file.display());
                    }
                }
            });
        if let Err(error) = moved {
            tracing::warn!("cannot start moving to the Trash: {error}");
        }
    }

    fn open_with(&mut self, window: &mut Window, app_id: &str, files: &[PathBuf]) {
        let Some(entry) = self.entry(app_id).cloned() else {
            return;
        };
        if files.is_empty() {
            return;
        }
        let token = window.activation_token(app_id).ok();
        match bande_xdg::launch_with(&entry, files, token.as_deref()) {
            Ok(()) => {
                if !self.running.iter().any(|running| running == app_id) {
                    self.state.borrow_mut().launching(app_id, Instant::now());
                }
            }
            Err(error) => tracing::warn!("cannot open the files with {}: {error}", entry.name),
        }
    }

    fn release_keyboard(&mut self, window: &mut Window) {
        self.release_at = None;
        self.state.borrow_mut().end_navigation(Instant::now());
        let Some(active) = self.active.clone() else {
            return;
        };
        if let Err(error) = window.activate_app(&active) {
            tracing::debug!("cannot give the keyboard back to {active}: {error}");
        }
    }

    fn app_name(&self, app_id: &str) -> String {
        app_name(self.apps.entries(), app_id)
    }

    fn opens_at_login(&self, app_id: &str) -> bool {
        match (&self.autostart, self.entry(app_id)) {
            (Some(dir), Some(entry)) => opens_at_login(dir, &entry.id),
            _ => false,
        }
    }

    fn can_keep(&self, window: &Window, app_id: &str) -> bool {
        self.entry(app_id).is_some()
            || self.kept.iter().any(|kept| kept == app_id)
            || window.app_pid(&self.window_app(app_id)).is_some()
    }

    fn keep_launcher(&mut self, window: &Window, app_id: &str) -> bool {
        if self.entry(app_id).is_some() {
            return true;
        }
        let Some(pid) = window.app_pid(&self.window_app(app_id)) else {
            return false;
        };
        let Some(home) = data_home() else {
            return false;
        };
        match write_launcher(&home, app_id, Path::new(PROC), pid) {
            Ok(_) => {
                self.apps.reload();
                self.follow_windows(window);
                self.entry(app_id).is_some()
            }
            Err(error) => {
                tracing::warn!("cannot write a launcher to keep {app_id}: {error}");
                false
            }
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
        if self
            .keep
            .as_ref()
            .is_none_or(|keep| keep.send(kept).is_err())
        {
            tracing::warn!("the Taskbar cannot save the apps kept in it");
        }
    }

    fn trash_full(&self) -> bool {
        self.trash.as_ref().is_some_and(|trash| trash.full)
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
        let session = self.session.clone();
        let emptied = thread::Builder::new()
            .name("empty-trash".to_owned())
            .spawn(move || {
                if let Some(session) = session {
                    bande_sound::play(&session, "trash-empty");
                }
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
            MenuTarget::More(hidden) => hidden
                .iter()
                .map(|app_id| {
                    MenuItem::new(&self.app_name(app_id), &format!("{MENU_OPEN_APP}{app_id}"))
                })
                .collect(),
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
                if running {
                    let windows = window.app_windows(&self.window_app(app_id));
                    for shown in &windows {
                        let title = if shown.title.is_empty() {
                            self.app_name(app_id)
                        } else {
                            shown.title.clone()
                        };
                        let title = if shown.minimized {
                            L10N.tr_args("menu-minimized-window", [("title", title.into())])
                        } else {
                            title
                        };
                        items.push(MenuItem::new(
                            &title,
                            &format!("{MENU_WINDOW}{}", shown.identifier),
                        ));
                    }
                    if !windows.is_empty() {
                        items.push(MenuItem::separator());
                    }
                } else {
                    items.push(MenuItem::new(&L10N.tr("menu-open"), MENU_OPEN).enabled(installed));
                    items.push(MenuItem::separator());
                }
                let actions = self
                    .entry(app_id)
                    .map(|entry| entry.actions.as_slice())
                    .unwrap_or_default();
                for action in actions {
                    items.push(MenuItem::new(
                        &action.name,
                        &format!("{MENU_ACTION}{}", action.id),
                    ));
                }
                if !actions.is_empty() {
                    items.push(MenuItem::separator());
                }
                items.push(
                    MenuItem::new(&L10N.tr("menu-keep"), MENU_KEEP)
                        .checked(self.kept.contains(app_id))
                        .enabled(self.can_keep(window, app_id)),
                );
                items.push(
                    MenuItem::new(&L10N.tr("menu-login"), MENU_LOGIN)
                        .checked(self.opens_at_login(app_id))
                        .enabled(installed && self.autostart.is_some()),
                );
                items.push(MenuItem::new(&L10N.tr("menu-show"), MENU_SHOW).enabled(installed));
                if running {
                    items.push(MenuItem::separator());
                    items.push(MenuItem::new(&L10N.tr("menu-hide"), MENU_HIDE));
                    let force_quit = MenuItem::new(&L10N.tr("menu-force-quit"), MENU_FORCE_QUIT);
                    items.push(if self.unresponsive.contains(app_id) {
                        force_quit
                    } else {
                        MenuItem::new(&L10N.tr("menu-quit"), MENU_QUIT).alternate(force_quit)
                    });
                }
                items
            }
        };
        match window.show_menu(Menu::new(items), MenuPlacement::AtPoint(at)) {
            Ok(()) => {
                self.menu = Some(target);
                self.state.borrow_mut().set_menu_open(true, Instant::now());
            }
            Err(error) => tracing::warn!("cannot show the Taskbar menu: {error}"),
        }
    }

    fn menu_action(&mut self, window: &mut Window, action: &str) {
        self.set_menu_open(false);
        let Some(target) = self.menu.take() else {
            return;
        };
        match (target, action) {
            (MenuTarget::More(_), action) if action.starts_with(MENU_OPEN_APP) => {
                self.open(window, &action[MENU_OPEN_APP.len()..]);
            }
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
                    None if self.keep_launcher(window, &app_id) => kept.push(app_id),
                    None => return,
                }
                self.set_kept(kept);
            }
            (MenuTarget::App(app_id), MENU_OPEN) => self.open(window, &app_id),
            (MenuTarget::App(app_id), MENU_SHOW) => self.show_in_files(window, &app_id),
            (MenuTarget::App(app_id), MENU_HIDE) => {
                if let Err(error) = window.hide_app(&self.window_app(&app_id)) {
                    tracing::warn!("cannot hide {app_id}: {error}");
                }
            }
            (MenuTarget::App(app_id), MENU_QUIT) => {
                if let Err(error) = window.quit_app(&self.window_app(&app_id)) {
                    tracing::warn!("cannot quit {app_id}: {error}");
                }
            }
            (MenuTarget::App(app_id), MENU_FORCE_QUIT) => {
                if let Err(error) = window.force_quit_app(&self.window_app(&app_id)) {
                    tracing::warn!("cannot force {app_id} to quit: {error}");
                }
            }
            (MenuTarget::App(app_id), action) if action.starts_with(MENU_ACTION) => {
                self.run_action(window, &app_id, &action[MENU_ACTION.len()..]);
            }
            (MenuTarget::App(_), action) if action.starts_with(MENU_WINDOW) => {
                let identifier = &action[MENU_WINDOW.len()..];
                if let Err(error) = window.activate_window(identifier) {
                    tracing::warn!("cannot bring the window forward: {error}");
                }
            }
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
        let token = window.activation_token(FILES).ok();
        let started = launch_with_arguments(&entry, &[location.to_owned()], token.as_deref());
        if let Err(error) = started {
            tracing::warn!("cannot open {location}: {error}");
        }
    }

    fn report(&mut self, window: &mut Window) {
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

    fn move_to(&mut self, window: &mut Window, edge: BarEdge, now: Instant) {
        let state = self.state.borrow();
        if state.edge() == edge || state.rail().is_some() {
            return;
        }
        drop(state);
        self.state.borrow_mut().set_edge(edge, now);
        self.reported.clear();
        self.input = None;
        window.set_layer_anchor(anchor(edge));
        let icon_size = self.state.borrow().icon_size();
        if let Err(error) = window.set_layer_size(surface_size(edge, icon_size, (0, 0))) {
            tracing::warn!("cannot move the Taskbar: {error}");
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
        let size = if self.state.borrow().rail().is_some() {
            rail_surface_size(icon_size)
        } else {
            surface_size(edge, icon_size, (0, 0))
        };
        if let Err(error) = window.set_layer_size(size) {
            tracing::warn!("cannot resize the Taskbar: {error}");
        }
    }

    fn receive(&mut self, window: &mut Window) {
        let now = Instant::now();
        for event in self.bus.drain() {
            match event {
                BusEvent::IconSize(size) => self.resize(window, size as f32),
                BusEvent::AutoHide(auto_hide) => {
                    self.state.borrow_mut().set_auto_hide(auto_hide, now);
                }
                BusEvent::Apps(apps) => {
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

impl Taskbar {
    pub fn set_rail(&mut self, rail: Rail) {
        self.state.borrow_mut().set_rail(rail);
    }

    pub fn icon_size(&self) -> f32 {
        self.state.borrow().icon_size()
    }

    pub fn has_menu(&self) -> bool {
        self.menu.is_some()
    }

    pub fn set_menu_open(&mut self, open: bool) {
        if !open && !self.state.borrow().is_navigating() {
            self.release_at = Some(Instant::now() + KEYBOARD_GRACE);
        }
        self.state.borrow_mut().set_menu_open(open, Instant::now());
    }

    fn menu_shown(&self) -> bool {
        self.state.borrow().is_menu_open()
    }

    fn menu_closed(&mut self) {
        self.menu = None;
        self.confirming = false;
        self.set_menu_open(false);
    }

    fn owns_open_menu_at(&self, point: Point) -> bool {
        self.menu.is_some() && self.state.borrow().target_at(point) == self.menu
    }

    pub fn revealed(&self, now: Instant) -> f32 {
        self.state.borrow().revealed(now)
    }

    fn follow_full_screen(&mut self, window: &mut Window, output: &str, full_screen: bool) {
        if output != self.output {
            return;
        }
        self.state
            .borrow_mut()
            .set_full_screen(full_screen, Instant::now());
        window.set_layer(if full_screen {
            LayerKind::Overlay
        } else {
            LayerKind::Top
        });
        window.request_redraw();
    }
}

impl App for Taskbar {
    fn scene(&mut self, window: &mut Window) -> Scene {
        self.follow_scale(window);
        let scene = scene(&mut self.ui, window.size());
        self.report(window);
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::Input(Event::Key { key, modifiers })
                if self.state.borrow().is_navigating() =>
            {
                self.state
                    .borrow_mut()
                    .navigate(key, modifiers, Instant::now());
                self.run_actions(window);
            }
            WindowEvent::FocusShell(namespace) if namespace == TASKBAR_NAMESPACE => {
                self.release_at = None;
                let active = self.active.clone();
                self.state
                    .borrow_mut()
                    .start_navigation(active.as_deref(), Instant::now());
                window.request_redraw();
            }
            WindowEvent::Focus(true) => {
                if !self.state.borrow().is_navigating() && !self.menu_shown() {
                    self.release_at = Some(Instant::now() + KEYBOARD_GRACE);
                }
            }
            WindowEvent::Focus(false) => {
                self.release_at = None;
                if !self.menu_shown() {
                    self.state.borrow_mut().end_navigation(Instant::now());
                    window.request_redraw();
                }
            }
            WindowEvent::Input(Event::PointerDown(pointer))
                if self.owns_open_menu_at(pointer.position) =>
            {
                window.close_menu();
                self.menu_closed();
                self.swallow_release = true;
                window.request_redraw();
            }
            WindowEvent::Input(Event::PointerUp(_)) if self.swallow_release => {
                self.swallow_release = false;
            }
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
                self.follow_windows(window);
                let running = &self.running;
                self.unresponsive.retain(|app_id| running.contains(app_id));
                self.sync();
                window.request_redraw();
            }
            WindowEvent::MenuAction(action) => {
                self.menu_action(window, &action);
                window.request_redraw();
            }
            WindowEvent::MenuClosed => {
                self.menu_closed();
                window.request_redraw();
            }
            WindowEvent::ActiveApp(app_id) => {
                self.state.borrow_mut().activated(&self.identity(&app_id));
                self.active = (!app_id.is_empty()).then_some(app_id);
                window.request_redraw();
            }
            WindowEvent::AppResponding {
                app_id, responding, ..
            } => {
                let app_id = self.identity(&app_id);
                self.unresponsive.retain(|known| *known != app_id);
                if !responding {
                    self.unresponsive.push(app_id);
                }
            }
            WindowEvent::Drag(drag) => {
                self.drag(window, drag);
                window.request_redraw();
            }
            WindowEvent::FullScreen {
                output,
                full_screen,
            } => self.follow_full_screen(window, &output, full_screen),
            WindowEvent::AttentionRequested(app_id) => {
                self.state
                    .borrow_mut()
                    .request_attention(&self.identity(&app_id), Instant::now());
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
        let mut sources = vec![self.bus.fd()];
        sources.extend(self.apps.fd());
        if let Some(trash) = &self.trash {
            sources.extend([trash.watch.fd(), trash.checked.fd()]);
        }
        sources
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        let watched = 1 + usize::from(self.apps.fd().is_some());
        match index {
            0 => self.receive(window),
            1 if watched == 2 => self.apps.read(),
            index if index == watched => {
                if let Some(trash) = &mut self.trash {
                    trash.watch.read();
                }
            }
            _ => {
                let Some(trash) = &mut self.trash else {
                    return;
                };
                if let Some(full) = trash.checked.drain().pop() {
                    trash.full = full;
                    self.state.borrow_mut().set_trash_full(full);
                    window.request_redraw();
                }
            }
        }
    }

    fn deadline(&self) -> Option<Instant> {
        let watch = self.trash.as_ref().and_then(|trash| trash.watch.deadline());
        [
            self.state.borrow().deadline(),
            watch,
            self.apps.deadline(),
            self.release_at,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        if self.release_at.is_some_and(|at| now >= at) {
            self.release_at = None;
            if !self.menu_shown() && !self.state.borrow().is_navigating() {
                self.release_keyboard(window);
            }
        }
        self.state.borrow_mut().tick(now);
        self.follow_installed_apps(window, now);
        if let Some(trash) = &mut self.trash
            && !trash.watch.take_changed(now).is_empty()
        {
            trash.rewatch();
            trash.check();
        }
        self.run_actions(window);
    }

    fn animating(&self) -> bool {
        let now = Instant::now();
        let state = self.state.borrow();
        self.ui.is_animating() || state.pulsing(now) || state.is_revealing(now)
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }
}

pub fn anchor(edge: BarEdge) -> Edges {
    Edges {
        left: edge != BarEdge::Right,
        right: edge != BarEdge::Left,
        top: edge != BarEdge::Bottom,
        bottom: edge != BarEdge::Top,
    }
}

pub fn taskbar_role(edge: BarEdge, icon_size: f32) -> Role {
    Role::Layer(LayerOptions {
        layer: LayerKind::Top,
        anchor: anchor(edge),
        exclusive_zone: exclusive_zone(icon_size).round() as i32,
        keyboard: KeyboardFocus::OnDemand,
        namespace: TASKBAR_NAMESPACE.to_owned(),
    })
}

impl Taskbar {
    pub fn new(
        window: &Window,
        output: &str,
        config: &BarConfig,
        text: TextSystem,
        session: Option<&Connection>,
        home: (Option<&Path>, Option<&Path>),
    ) -> std::io::Result<Self> {
        let edge = config.edge;
        let icon_size = config.icon_size as f32;
        let inbox = Inbox::new()?;
        if let Some(session) = session {
            bus::follow(session, inbox.outbox());
        }
        let appearance = window.appearance();
        let state = TaskbarState::new(edge, icon_size);
        state.borrow_mut().set_appearance(appearance);
        state
            .borrow_mut()
            .set_auto_hide(config.auto_hide, Instant::now());
        let mut ui = taskbar_ui(state.clone(), appearance.palette(), text);
        ui.set_appearance(appearance);
        let (config_home, home) = home;
        let mut taskbar = Self {
            state,
            ui,
            apps: InstalledApps::load(),
            kept: config.apps.clone(),
            windows: Vec::new(),
            running: Vec::new(),
            icons: HashMap::new(),
            icon_scale: window.scale(),
            reported: HashMap::new(),
            input: None,
            zone: exclusive_zone(icon_size).round() as i32,
            menu: None,
            confirming: false,
            swallow_release: false,
            active: None,
            release_at: None,
            unresponsive: Vec::new(),
            keep: session.map(|session| bus::keep_writer(session.clone())),
            autostart: autostart_dir(config_home, home),
            bus: inbox,
            trash: TrashWatch::start(),
            session: session.cloned(),
            output: output.to_owned(),
        };
        taskbar.follow_windows(window);
        taskbar.load_trash_icons();
        taskbar.sync();
        Ok(taskbar)
    }
}
