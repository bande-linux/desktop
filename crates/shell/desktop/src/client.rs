use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::rc::Rc;
use std::thread;
use std::time::{Instant, SystemTime};

use crate::confirm::{FORCE_EJECT, eject_failed};
use crate::{
    Artwork, Bounds, CONFIRM_DELETE_FLAG, DesktopView, EJECT_FAILED_FLAG, Item, Kind, Layer,
    Layouts, Luma, MenuApp, Request, Selection, WallpaperKey, command, confirm_delete,
    desktop_menu, items_menu, layouts_path, list, parse_layouts, save_layouts,
};
use accesskit::TreeUpdate;
use bande_config::{BarEdge, Config, Wallpaper, WallpaperFit};
use bande_fs::{Action, Answer, Change, FolderWatch, Operations, Purpose, Resolution, Task, Trash};
use bande_inbox::Inbox;
use bande_thumbnails::{Request as ThumbnailRequest, ThumbnailSize, Thumbnails};
use bande_toolkit::ui::frame_time;
use bande_toolkit::{
    App, DragAction, DragEvent, Edges, Image, KeyboardFocus, LayerKind, LayerOptions,
    MenuPlacement, Point, Role, Scene, Scheme, TextSystem, Transfer, Window, WindowEvent,
};
use bande_volumes::{EjectFailure, eject, media_roots, mounted_volumes};
use bande_wallpaper::{
    Choice, Fit, Picture, RenderError, Variant, choose, follow, render, render_cached,
};
use bande_xdg::{
    Apps, DIRECTORY, DesktopEntry, ICON_THEMES, OCTET_STREAM, UserDir, cache_home, config_home,
    data_dirs, file_uri, find_icon, launch, launch_with, launch_with_arguments, path_from_file_uri,
    user_dir,
};

const SETTINGS_FOLDER: &str = "bande";
const FILES_SETTINGS: &str = "files.conf";
const SHOW_EXTENSIONS: &str = "show-extensions";
const SHOW_DISKS: &str = "desktop-disks";
const BUSY: &str = "busy";
const PDF: &str = "application/pdf";
const WALLPAPER_CACHE: &str = "bande/wallpapers";
const SETTINGS_APP: &str = "org.bande.Settings";
const WALLPAPER_PANE: &str = "appearance";
const FILES_INFO_FLAG: &str = "--info";

type Loaded = (WallpaperKey, Option<(Picture, Luma)>);

const CONFIRMED: &str = "delete";

struct Confirming {
    child: Child,
    answer: ChildStdout,
    paths: Vec<PathBuf>,
}

pub struct DesktopSurface {
    view: DesktopView,
    paste_moves: bool,
    apps: Rc<RefCell<Apps>>,
    operations: Operations,
    watch: FolderWatch,
    desktop_dir: PathBuf,
    wallpaper_only: bool,
    media: Vec<PathBuf>,
    data_dirs: Vec<PathBuf>,
    home: Option<PathBuf>,
    icons: HashMap<(String, u32), Option<Image>>,
    thumbnails: Option<Thumbnails>,
    items: Vec<Item>,
    shown: Shown,
    pictures: HashMap<PathBuf, (Option<SystemTime>, Option<Option<Image>>)>,
    layouts_file: Option<PathBuf>,
    bar_edge: BarEdge,
    combined_bar: bool,
    choice: Option<Choice>,
    choices: Inbox<Choice>,
    loaded: Inbox<Loaded>,
    pending: HashSet<WallpaperKey>,
    wallpapers: HashMap<WallpaperKey, Layer>,
    scheme: Scheme,
    viewport: Option<((u32, u32), f64)>,
    confirming: Option<Confirming>,
}

impl DesktopSurface {
    fn variant(&self) -> Variant {
        match self.scheme {
            Scheme::Light => Variant::Light,
            Scheme::Dark => Variant::Dark,
        }
    }

    fn wallpaper_key(&self, variant: Variant) -> Option<WallpaperKey> {
        let choice = self.choice.as_ref()?;
        let ((width, height), scale) = self.viewport?;
        let path = choice.path(variant, self.home.as_deref(), &self.data_dirs)?;
        Some(WallpaperKey {
            path,
            fit: choice.fit,
            width: (f64::from(width) * scale).ceil() as u32,
            height: (f64::from(height) * scale).ceil() as u32,
        })
    }

    fn load(&mut self, key: WallpaperKey) {
        if self.wallpapers.contains_key(&key) || !self.pending.insert(key.clone()) {
            return;
        }
        let loader = self.loaded.outbox();
        let work = move || {
            let picture = draw_wallpaper(&key);
            let rendered = match picture {
                Ok(picture) => {
                    let luma = Luma::of(&picture);
                    Some((picture, luma))
                }
                Err(error) => {
                    tracing::warn!("cannot show the wallpaper {}: {error}", key.path.display());
                    None
                }
            };
            loader.send((key, rendered));
        };
        if let Err(error) = thread::Builder::new()
            .name("wallpaper".to_owned())
            .spawn(work)
        {
            tracing::warn!("cannot start loading the wallpaper: {error}");
        }
    }

    fn refresh_wallpaper(&mut self, window: &mut Window) {
        let Some(wanted) = self.wallpaper_key(self.variant()) else {
            if self.choice.is_some() && self.viewport.is_some() {
                self.view.wallpaper_unavailable();
                window.request_redraw();
            }
            return;
        };
        let other = match self.variant() {
            Variant::Light => Variant::Dark,
            Variant::Dark => Variant::Light,
        };
        let spare = self.wallpaper_key(other);
        self.wallpapers
            .retain(|key, _| *key == wanted || Some(key) == spare.as_ref());
        match self.wallpapers.get(&wanted) {
            Some(layer) => {
                if self.view.wallpaper_key() != Some(&wanted) {
                    self.view
                        .show_wallpaper(layer.clone(), frame_time().on_screen());
                    window.request_redraw();
                }
                if let Some(spare) = spare {
                    self.load(spare);
                }
            }
            None => self.load(wanted),
        }
    }

    fn icon(&mut self, names: &[String], pixels: u32) -> Option<Image> {
        names.iter().find_map(|name| {
            self.icons
                .entry((name.clone(), pixels))
                .or_insert_with(|| {
                    find_icon(name, pixels, &ICON_THEMES, &self.data_dirs)
                        .and_then(|path| Image::load(&path, pixels))
                })
                .clone()
        })
    }

    fn icon_names(&self, item: &Item) -> Vec<String> {
        let mut names = match item.kind {
            Kind::Volume => vec!["drive-removable-media".to_owned()],
            Kind::Folder => vec!["folder".to_owned()],
            Kind::File => {
                let mime = self.apps.borrow().mime_for(&item.path);
                let launcher = self
                    .apps
                    .borrow()
                    .launcher(&item.path, &mime)
                    .and_then(|entry| entry.icon);
                launcher
                    .into_iter()
                    .chain(self.apps.borrow().icon_names(&mime))
                    .collect()
            }
        };
        names.push("application-x-generic".to_owned());
        names
    }

    fn artwork(
        &mut self,
        item: &Item,
        pixels: u32,
        wanted: &mut Vec<ThumbnailRequest>,
    ) -> Option<Artwork> {
        let mime = match item.kind {
            Kind::File => self.apps.borrow().mime_for(&item.path),
            _ => String::new(),
        };
        if self.thumbnails.is_some() && (mime.starts_with("image/") || mime == PDF) {
            let modified = std::fs::metadata(&item.path)
                .and_then(|metadata| metadata.modified())
                .ok();
            let known = self
                .pictures
                .get(&item.path)
                .filter(|(made, _)| *made == modified);
            match known {
                Some((_, Some(Some(picture)))) => {
                    return Some(Artwork::Thumbnail(picture.clone()));
                }
                Some((_, Some(None))) => {}
                _ => {
                    self.pictures.insert(item.path.clone(), (modified, None));
                    wanted.push(ThumbnailRequest {
                        path: item.path.clone(),
                        mime,
                        size: ThumbnailSize::for_pixels(pixels),
                    });
                }
            }
        }
        let names = self.icon_names(item);
        self.icon(&names, pixels).map(Artwork::Icon)
    }

    fn thumbnails_ready(&mut self) {
        let Some(thumbnails) = &mut self.thumbnails else {
            return;
        };
        let done = thumbnails.take();
        if done.is_empty() {
            return;
        }
        let size = ThumbnailSize::for_pixels(self.icon_pixels());
        for thumbnailed in done.into_iter().filter(|done| done.size == size) {
            let picture = thumbnailed
                .picture
                .and_then(|picture| Image::from_rgba(picture.width, picture.height, picture.rgba));
            if let Some((_, kept)) = self.pictures.get_mut(&thumbnailed.path) {
                *kept = Some(picture);
            }
        }
        self.show_items();
    }

    fn full_name(&self, path: &Path, typed: String) -> String {
        let hidden = self
            .items
            .iter()
            .find(|item| item.path == path)
            .and_then(|item| item.name.get(item.label.len()..))
            .unwrap_or_default();
        match hidden.is_empty()
            || typed.is_empty()
            || self.apps.borrow().extension(&typed).is_some()
        {
            true => typed,
            false => format!("{typed}{hidden}"),
        }
    }

    fn relist(&mut self) {
        if self.wallpaper_only {
            return;
        }
        let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
        self.shown = Shown::read();
        let volumes = match self.shown.disks {
            true => mounted_volumes(&mountinfo, &self.media),
            false => Vec::new(),
        };
        self.items = list(
            &mut self.apps.borrow_mut(),
            &self.desktop_dir,
            &volumes,
            self.shown.extensions,
        );
        self.show_items();
        self.watch_folders();
        self.save_layouts();
    }

    fn show_items(&mut self) {
        let pixels = self.icon_pixels();
        let mut wanted = Vec::new();
        let with_icons = self
            .items
            .clone()
            .into_iter()
            .map(|item| {
                let artwork = self.artwork(&item, pixels, &mut wanted);
                (item, artwork)
            })
            .collect::<Vec<_>>();
        let present: HashSet<&PathBuf> = with_icons.iter().map(|(item, _)| &item.path).collect();
        self.pictures.retain(|path, _| present.contains(path));
        if let Some(thumbnails) = self.thumbnails.as_ref().filter(|_| !wanted.is_empty()) {
            thumbnails.want(wanted);
        }
        self.view.set_items(with_icons, frame_time().on_screen());
    }

    fn icon_pixels(&self) -> u32 {
        let scale = self.viewport.map_or(1.0, |(_, scale)| scale);
        (f64::from(crate::ICON_SIZE) * scale).ceil() as u32
    }

    fn watch_folders(&mut self) {
        let mut folders = vec![self.desktop_dir.clone()];
        folders.extend(config_home().map(|config| config.join(SETTINGS_FOLDER)));
        for root in &self.media {
            folders.push(root.clone());
            if let Some(parent) = root.parent() {
                folders.push(parent.to_path_buf());
            }
        }
        folders.retain(|folder| folder.is_dir());
        self.watch.watch(&folders);
    }

    fn save_layouts(&mut self) {
        let Some(path) = &self.layouts_file else {
            return;
        };
        if let Some(layouts) = self.view.take_changed_layouts()
            && let Err(error) = save_layouts(path, layouts)
        {
            tracing::warn!("cannot remember the desktop layout: {error}");
        }
    }

    fn area(&self, size: (u32, u32)) -> Bounds {
        let (width, height) = (size.0 as f32, size.1 as f32);
        let top = bande_bar::menu_bar::HEIGHT as f32;
        let taskbar = bande_bar::taskbar::exclusive_zone(bande_bar::taskbar::ICON_SIZE);
        let bar = bande_bar::rail::bar_width(bande_bar::taskbar::ICON_SIZE);
        let (left, right, bottom, top) = match (self.bar_edge, self.combined_bar) {
            (_, true) => (bar, 0.0, 0.0, 0.0),
            (BarEdge::Left, false) => (taskbar, 0.0, 0.0, top),
            (BarEdge::Right, false) => (0.0, taskbar, 0.0, top),
            (BarEdge::Bottom, false) => (0.0, 0.0, taskbar, top),
            (BarEdge::Top, false) => (0.0, 0.0, 0.0, top + taskbar),
        };
        Bounds::new(
            left,
            top,
            (width - left - right).max(1.0),
            (height - top - bottom).max(1.0),
        )
    }

    fn follow_viewport(&mut self, window: &mut Window) {
        let size = window.size();
        let scale = window.scale();
        if self.viewport == Some((size, scale)) || size.0 == 0 || size.1 == 0 {
            return;
        }
        let relist = self.viewport.is_none_or(|(_, previous)| previous != scale);
        self.viewport = Some((size, scale));
        self.view.set_viewport(
            (size.0 as f32, size.1 as f32),
            scale as f32,
            self.area(size),
            frame_time().on_screen(),
        );
        if relist {
            self.icons.clear();
            self.pictures.clear();
            self.relist();
        }
        self.save_layouts();
        self.refresh_wallpaper(window);
    }

    fn files_app(&self) -> Option<DesktopEntry> {
        self.apps.borrow().default_for(DIRECTORY).cloned()
    }

    fn launch_with(&self, window: &mut Window, app: &DesktopEntry, files: &[PathBuf]) {
        let token = window.activation_token(app.app_id()).ok();
        if let Err(error) = launch_with(app, files, token.as_deref()) {
            tracing::warn!("cannot open with {}: {error}", app.id);
        }
    }

    fn open(&mut self, window: &mut Window, paths: Vec<PathBuf>) {
        for path in paths {
            let kind = self.view.desktop().item(&path).map(|item| item.kind);
            let mime = match kind {
                Some(Kind::Folder | Kind::Volume) => DIRECTORY.to_owned(),
                _ => self.apps.borrow().mime_for(&path),
            };
            if let Some(launcher) = self.apps.borrow().launcher(&path, &mime) {
                let token = window.activation_token(launcher.app_id()).ok();
                if let Err(error) = launch(&launcher, token.as_deref()) {
                    tracing::warn!("cannot start {}: {error}", launcher.name);
                }
                continue;
            }
            let app = self.apps.borrow().default_for(&mime).cloned();
            match app {
                Some(app) => self.launch_with(window, &app, &[path]),
                None => self.show_in_files(window, path),
            }
        }
    }

    fn show_in_files(&mut self, window: &mut Window, path: PathBuf) {
        match self.files_app() {
            Some(files) => self.launch_with(window, &files, &[path]),
            None => tracing::warn!("no app opens folders"),
        }
    }

    fn show_info(&mut self, window: &mut Window, path: PathBuf) {
        let Some(files) = self.files_app() else {
            tracing::warn!("no app opens folders");
            return;
        };
        let token = window.activation_token(files.app_id()).ok();
        let arguments = [
            FILES_INFO_FLAG.to_owned(),
            path.to_string_lossy().into_owned(),
        ];
        if let Err(error) = launch_with_arguments(&files, &arguments, token.as_deref()) {
            tracing::warn!("cannot show the info of {}: {error}", path.display());
        }
    }

    fn common_mime(&self, paths: &[PathBuf]) -> String {
        let mimes: Vec<String> = paths
            .iter()
            .map(
                |path| match self.view.desktop().item(path).map(|item| item.kind) {
                    Some(Kind::Folder | Kind::Volume) => DIRECTORY.to_owned(),
                    _ => self.apps.borrow().mime_for(path),
                },
            )
            .collect();
        match mimes.split_first() {
            Some((first, rest)) if rest.iter().all(|mime| mime == first) => first.clone(),
            _ => OCTET_STREAM.to_owned(),
        }
    }

    fn context_menu(&mut self, window: &mut Window, at: Point, on_items: bool) {
        let menu = if on_items {
            let selected = self.view.desktop().selected();
            let mime = self.common_mime(&selected);
            let selection = Selection {
                count: selected.len(),
                kinds: selected
                    .iter()
                    .filter_map(|path| self.view.desktop().item(path).map(|item| item.kind))
                    .collect(),
                picture: mime.starts_with("image/"),
                apps: self
                    .apps
                    .borrow()
                    .apps_for(&mime)
                    .into_iter()
                    .map(|entry| MenuApp {
                        id: entry.id.clone(),
                        name: entry.name.clone(),
                    })
                    .collect(),
            };
            items_menu(&selection)
        } else {
            desktop_menu(self.view.desktop().sort(), window.clipboard_has_uris())
        };
        if let Err(error) = window.show_menu(menu, MenuPlacement::AtPoint(at)) {
            tracing::warn!("cannot show the menu: {error}");
        }
    }

    fn change_wallpaper(&mut self, window: &mut Window) {
        let settings = bande_xdg::applications(
            &self.data_dirs,
            bande_xdg::Locale::from_env().as_ref(),
            &bande_xdg::current_desktops(),
        )
        .into_iter()
        .find(|entry| entry.app_id() == SETTINGS_APP);
        let Some(settings) = settings else {
            tracing::warn!("Settings is not installed");
            return;
        };
        let token = window.activation_token(SETTINGS_APP).ok();
        let launched =
            launch_with_arguments(&settings, &[WALLPAPER_PANE.to_owned()], token.as_deref());
        if let Err(error) = launched {
            tracing::warn!("cannot open Settings: {error}");
        }
    }

    fn start(&mut self, task: Task, action: Action) {
        if let Err(error) = self.operations.start(task, Purpose::Do(action)) {
            tracing::warn!("cannot start the file operation: {error}");
        }
    }

    fn run_requests(&mut self, window: &mut Window) {
        for request in self.view.take_requests() {
            match request {
                Request::Open(paths) => self.open(window, paths),
                Request::Trash(sources) => self.start(Task::Trash { sources }, Action::Trash),
                Request::ConfirmDelete(paths) => self.confirm_delete(paths),
                Request::Duplicate(sources) => {
                    self.start(Task::Duplicate { sources }, Action::Duplicate)
                }
                Request::Transfer {
                    sources,
                    folder,
                    copy: true,
                } => self.start(
                    Task::Copy {
                        sources,
                        destination: folder,
                    },
                    Action::Copy,
                ),
                Request::Transfer {
                    sources, folder, ..
                } => self.start(
                    Task::Move {
                        sources,
                        destination: folder,
                    },
                    Action::Move,
                ),
                Request::Rename { path, name } => {
                    match bande_fs::rename(&path, &self.full_name(&path, name)) {
                        Ok(Some(change)) => self.operations.record(Action::Rename, vec![change]),
                        Ok(None) => {}
                        Err(error) => tracing::warn!("cannot rename {}: {error:?}", path.display()),
                    }
                }
                Request::NewFolder => match bande_fs::new_folder(&self.desktop_dir) {
                    Ok(change) => {
                        if let Change::Created(path) = &change {
                            self.view.rename_when_listed(path.clone());
                        }
                        self.operations.record(Action::NewFolder, vec![change]);
                        self.relist();
                    }
                    Err(error) => tracing::warn!("cannot make a folder: {error}"),
                },
                Request::ShowInFiles(path) => self.show_in_files(window, path),
                Request::ShowInfo(path) => self.show_info(window, path),
                Request::OpenDesktopFolder => {
                    let folder = self.desktop_dir.clone();
                    self.show_in_files(window, folder);
                }
                Request::ContextMenu { at, on_items } => self.context_menu(window, at, on_items),
                Request::OpenWith { app, paths } => {
                    let mime = self.common_mime(&paths);
                    let app = self
                        .apps
                        .borrow()
                        .apps_for(&mime)
                        .into_iter()
                        .find(|entry| entry.id == app)
                        .cloned();
                    if let Some(app) = app {
                        self.launch_with(window, &app, &paths);
                    }
                }
                Request::Copy(paths) => window.copy_transfer(file_transfer(&paths)),
                Request::StartDrag(paths) => {
                    let icon = self.view.drag_icon();
                    let actions = [DragAction::Copy, DragAction::Move];
                    if let Err(error) =
                        window.start_drag(file_transfer(&paths), &actions, Some(icon))
                    {
                        tracing::warn!("cannot start dragging: {error}");
                        self.view.drag_finished();
                    }
                }
                Request::Cut(paths) => window.copy_transfer(file_transfer(&paths).cut()),
                Request::Paste { moves } => {
                    self.paste_moves = moves;
                    if let Err(error) = window.paste_transfer() {
                        tracing::warn!("cannot paste: {error}");
                    }
                }
                Request::Clipboard(request) => {
                    if let Err(error) = window.serve_clipboard(request) {
                        tracing::warn!("cannot use the clipboard: {error}");
                    }
                }
                Request::Eject(paths) => eject(paths, false, ask_to_force_eject),
                Request::SetWallpaper(path) => choose(path),
                Request::ChangeWallpaper => self.change_wallpaper(window),
                Request::Undo => {
                    if let Err(error) = self.operations.undo() {
                        tracing::warn!("cannot undo: {error}");
                    }
                }
                Request::Redo => {
                    if let Err(error) = self.operations.redo() {
                        tracing::warn!("cannot redo: {error}");
                    }
                }
            }
        }
        self.save_layouts();
        window.request_redraw();
    }

    fn confirm_delete(&mut self, paths: Vec<PathBuf>) {
        if self.confirming.is_some() {
            return;
        }
        let spawned = std::env::current_exe().and_then(|program| {
            Command::new(program)
                .arg(CONFIRM_DELETE_FLAG)
                .arg(paths.len().to_string())
                .stdout(Stdio::piped())
                .spawn()
        });
        match spawned {
            Ok(mut child) => match child.stdout.take() {
                Some(answer) => {
                    self.confirming = Some(Confirming {
                        child,
                        answer,
                        paths,
                    })
                }
                None => tracing::warn!("cannot hear the delete confirmation"),
            },
            Err(error) => tracing::warn!("cannot ask to delete: {error}"),
        }
    }

    fn confirmed(&mut self) {
        let Some(mut confirming) = self.confirming.take() else {
            return;
        };
        let mut answer = String::new();
        if let Err(error) = confirming.answer.read_to_string(&mut answer) {
            tracing::warn!("cannot read the delete confirmation: {error}");
        }
        if let Err(error) = confirming.child.wait() {
            tracing::warn!("cannot finish the delete confirmation: {error}");
        }
        if answer.trim() == CONFIRMED {
            let task = Task::Delete {
                sources: confirming.paths,
            };
            if let Err(error) = self.operations.start(task, Purpose::Erase) {
                tracing::warn!("cannot delete: {error}");
            }
        }
    }

    fn paste_files(&mut self, transfer: &Transfer) {
        let sources: Vec<PathBuf> = transfer
            .uris
            .iter()
            .filter_map(|uri| path_from_file_uri(uri))
            .collect();
        if sources.is_empty() {
            return;
        }
        let destination = self.desktop_dir.clone();
        match std::mem::take(&mut self.paste_moves) || transfer.cut {
            true => self.start(
                Task::Move {
                    sources,
                    destination,
                },
                Action::Move,
            ),
            false => self.start(
                Task::Copy {
                    sources,
                    destination,
                },
                Action::Copy,
            ),
        }
    }

    fn poll_operation(&mut self, index: usize) {
        if let Some((conflict, _)) = self.operations.conflict() {
            self.operations.answer(
                conflict,
                Answer {
                    resolution: Resolution::KeepBoth,
                    apply_to_all: true,
                },
            );
        }
        let Some(finished) = self.operations.poll(index) else {
            return;
        };
        for failure in &finished.outcome.failures {
            tracing::warn!("{}: {}", failure.path.display(), failure.error);
        }
        let created: Vec<PathBuf> = finished
            .outcome
            .changes
            .iter()
            .filter_map(|change| match change {
                Change::Created(path) | Change::Moved { to: path, .. } => Some(path.clone()),
                Change::Trashed(_) => None,
            })
            .filter(|path| path.parent() == Some(self.desktop_dir.as_path()))
            .collect();
        self.relist();
        if !created.is_empty() {
            self.view.select_paths(&created);
        }
    }
}

impl App for DesktopSurface {
    fn scene(&mut self, window: &mut Window) -> Scene {
        self.follow_viewport(window);
        self.view.scene(frame_time().presentation)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::Input(input) => {
                if self.view.handle(&input, frame_time().on_screen()) {
                    window.request_redraw();
                }
                self.run_requests(window);
            }
            WindowEvent::Accessibility(request) => {
                if self.view.perform(&request) {
                    window.request_redraw();
                }
                self.run_requests(window);
            }
            WindowEvent::MenuAction(action) => {
                if let Some(command) = command(&action) {
                    self.view.run_command(command, frame_time().on_screen());
                }
                self.run_requests(window);
            }
            WindowEvent::Drag(drag) => {
                match drag {
                    DragEvent::Entered { position, uris, .. } => {
                        let action = self.view.drag_entered(position, &uris);
                        window.accept_drag(action);
                    }
                    DragEvent::Moved(position) => {
                        let action = self.view.drag_moved(position);
                        window.accept_drag(action);
                    }
                    DragEvent::Left => self.view.drag_left(),
                    DragEvent::Dropped {
                        position,
                        transfer,
                        action,
                    } => {
                        self.view.dropped(
                            position,
                            &transfer.uris,
                            action,
                            frame_time().on_screen(),
                        );
                    }
                    DragEvent::Finished(_) => self.view.drag_finished(),
                }
                self.run_requests(window);
            }
            WindowEvent::PastedTransfer(transfer) => {
                self.paste_files(&transfer);
                window.request_redraw();
            }
            WindowEvent::Pasted(text) => {
                if self.view.pasted(&text) {
                    window.request_redraw();
                }
            }
            WindowEvent::Focus(focused) => {
                self.view.set_focused(focused);
                self.run_requests(window);
            }
            WindowEvent::AppearanceChanged(appearance) => {
                self.scheme = appearance.scheme;
                self.view.set_appearance(appearance);
                self.refresh_wallpaper(window);
            }
            _ => {}
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        let mut sources = vec![self.watch.fd(), self.choices.fd(), self.loaded.fd()];
        sources.extend(self.thumbnails.as_ref().map(Thumbnails::fd));
        sources.extend(self.operations.fds());
        sources.extend(
            self.confirming
                .as_ref()
                .map(|confirming| confirming.answer.as_fd()),
        );
        sources
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        match index {
            0 => self.watch.read(),
            1 => {
                if let Some(choice) = self.choices.drain().pop() {
                    self.choice = Some(choice);
                    self.refresh_wallpaper(window);
                }
            }
            2 => {
                let mut failed = false;
                for (key, rendered) in self.loaded.drain() {
                    failed |= rendered.is_none();
                    let layer = rendered.and_then(|(picture, luma)| {
                        let redrawn = key.clone();
                        Image::redrawable(picture.width, picture.height, picture.rgba, move || {
                            draw_wallpaper(&redrawn).ok().map(|picture| picture.rgba)
                        })
                        .map(|image| Layer {
                            key: key.clone(),
                            image,
                            luma,
                        })
                    });
                    if let Some(layer) = layer {
                        self.pending.remove(&key);
                        self.wallpapers.insert(key, layer);
                    }
                }
                self.refresh_wallpaper(window);
                if failed && self.view.wallpaper_key().is_none() {
                    self.view.wallpaper_unavailable();
                    window.request_redraw();
                }
            }
            3 if self.thumbnails.is_some() => {
                self.thumbnails_ready();
                window.request_redraw();
            }
            index => {
                let first = 3 + usize::from(self.thumbnails.is_some());
                if index - first == self.operations.fds().len() {
                    self.confirmed();
                } else {
                    self.poll_operation(index - first);
                    window.request_redraw();
                }
            }
        }
    }

    fn deadline(&self) -> Option<Instant> {
        [self.watch.deadline(), self.view.deadline()]
            .into_iter()
            .flatten()
            .min()
    }

    fn animating(&self) -> bool {
        self.view.is_animating()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        let changes = self.watch.take_changed(now);
        let settings = config_home().map(|config| config.join(SETTINGS_FOLDER));
        let elsewhere = changes.folders.iter().chain(&changes.entries).any(|path| {
            settings
                .as_ref()
                .is_none_or(|settings| !path.starts_with(settings))
        });
        if elsewhere || (!changes.is_empty() && Shown::read() != self.shown) {
            self.relist();
        }
        self.view.tick(now);
        self.run_requests(window);
        window.request_redraw();
    }

    fn should_close(&mut self, _window: &mut Window) -> bool {
        false
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.view.accessibility())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Shown {
    extensions: bool,
    disks: bool,
}

impl Shown {
    fn read() -> Self {
        let text = config_home()
            .and_then(|config| {
                std::fs::read_to_string(config.join(SETTINGS_FOLDER).join(FILES_SETTINGS)).ok()
            })
            .unwrap_or_default();
        let flag = |wanted: &str| {
            text.lines()
                .filter_map(|line| line.split_once('='))
                .find(|(key, _)| key.trim() == wanted)
                .map(|(_, value)| value.trim() == "true")
        };
        Self {
            extensions: flag(SHOW_EXTENSIONS).unwrap_or(false),
            disks: flag(SHOW_DISKS).unwrap_or(true),
        }
    }
}

fn draw_wallpaper(key: &WallpaperKey) -> Result<Picture, RenderError> {
    match cache_home() {
        Some(cache) => render_cached(
            &cache.join(WALLPAPER_CACHE),
            &key.path,
            key.fit,
            key.width,
            key.height,
        ),
        None => render(&key.path, key.fit, key.width, key.height),
    }
}

fn configured_choice(wallpaper: &Wallpaper) -> Choice {
    Choice {
        light: wallpaper.light.clone(),
        dark: wallpaper.dark.clone(),
        fit: match wallpaper.fit {
            WallpaperFit::Fill => Fit::Fill,
            WallpaperFit::Fit => Fit::Fit,
        },
    }
}

fn file_transfer(paths: &[PathBuf]) -> Transfer {
    Transfer::uris(paths.iter().map(|path| file_uri(path)).collect())
}

fn desktop_folder(home: &Path) -> PathBuf {
    let dirs = config_home()
        .map(|config| config.join("user-dirs.dirs"))
        .and_then(|path| std::fs::read_to_string(path).ok());
    let folder = user_dir(home, dirs.as_deref(), UserDir::Desktop);
    if folder != home
        && let Err(error) = std::fs::create_dir_all(&folder)
    {
        tracing::warn!("cannot make the Desktop folder: {error}");
    }
    folder
}

pub fn role() -> Role {
    Role::Layer(LayerOptions {
        layer: LayerKind::Background,
        anchor: Edges::ALL,
        exclusive_zone: -1,
        keyboard: KeyboardFocus::OnDemand,
        namespace: "desktop".to_owned(),
    })
}

impl DesktopSurface {
    pub fn new(
        window: &Window,
        main: bool,
        config: &Config,
        text: TextSystem,
        apps: Rc<RefCell<Apps>>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let bar_edge = config.bar.edge;
        let combined_bar = config.bar.menu_bar.combines(config.bar.edge);
        let desktop_dir = desktop_folder(home.as_deref().unwrap_or(Path::new("/")));
        let user = std::env::var("USER").unwrap_or_default();
        let trash = Trash::for_user().ok_or("no home folder for the Trash")?;
        let layouts_file = layouts_path();
        let layouts: Layouts = layouts_file
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| parse_layouts(&text))
            .unwrap_or_default();
        let choices = Inbox::new()?;
        follow(choices.outbox());
        let loaded = Inbox::new()?;
        let appearance = window.appearance();
        let data_dirs = data_dirs();
        let thumbnails = main.then(|| Thumbnails::for_user(&data_dirs)).flatten();
        let mut surface = DesktopSurface {
            view: DesktopView::new(appearance, layouts, text),
            paste_moves: false,
            apps,
            operations: Operations::new(trash),
            watch: FolderWatch::new()?,
            desktop_dir,
            wallpaper_only: !main,
            media: media_roots(&user),
            data_dirs,
            home,
            icons: HashMap::new(),
            thumbnails,
            items: Vec::new(),
            shown: Shown {
                extensions: false,
                disks: true,
            },
            pictures: HashMap::new(),
            layouts_file,
            bar_edge,
            combined_bar,
            choice: Some(configured_choice(&config.wallpaper)),
            choices,
            loaded,
            pending: HashSet::new(),
            wallpapers: HashMap::new(),
            scheme: appearance.scheme,
            viewport: None,
            confirming: None,
        };
        let folder = surface.desktop_dir.clone();
        surface.view.set_folder(folder);
        Ok(surface)
    }
}

fn ask_to_force_eject(failure: EjectFailure) {
    let name = failure.mount_point.file_name().map_or_else(
        || failure.mount_point.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let asked = std::env::current_exe().and_then(|program| {
        Command::new(program)
            .arg(EJECT_FAILED_FLAG)
            .arg(&name)
            .arg(if failure.busy { BUSY } else { "" })
            .arg(&failure.message)
            .output()
    });
    match asked {
        Ok(output) if String::from_utf8_lossy(&output.stdout).trim() == FORCE_EJECT => {
            eject(vec![failure.mount_point], true, |_| {});
        }
        Ok(_) => {}
        Err(error) => tracing::warn!("cannot report that {name} was not ejected: {error}"),
    }
}

pub fn report_eject_failure(
    name: &str,
    busy: &str,
    message: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if eject_failed(name, busy == BUSY, message)? {
        println!("{FORCE_EJECT}");
    }
    Ok(())
}

pub fn confirm(count: &str) -> Result<(), Box<dyn std::error::Error>> {
    if confirm_delete(count.parse().unwrap_or(1))? {
        println!("{CONFIRMED}");
    }
    Ok(())
}
