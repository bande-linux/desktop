use std::collections::{HashMap, HashSet};
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;
use std::time::Instant;

use accesskit::TreeUpdate;
use unbound_config::{Config, DockEdge, config_path};
use unbound_desktop_surface::{
    Bounds, DesktopView, Item, Kind, Layer, Layouts, Luma, MenuApp, Request, Selection,
    WallpaperKey, command, desktop_menu, eject, items_menu, layouts_path, list, media_roots,
    mounted_volumes, parse_layouts, save_layouts,
};
use unbound_fs::{
    Action, Answer, Change, FolderWatch, Operations, Purpose, Resolution, Task, Trash,
};
use unbound_toolkit::{
    App, Edges, Image, KeyboardFocus, LayerKind, LayerOptions, MenuPlacement, Point, Role, Scene,
    Scheme, Window, WindowEvent,
};
use unbound_wallpaper::{Choice, Inbox, Outbox, Picture, Variant, choose, follow, inbox, render};
use unbound_xdg::{
    Apps, DIRECTORY, DesktopEntry, OCTET_STREAM, UserDir, config_dirs, config_home, data_dirs,
    find_icon, launch, launch_with, user_dir,
};

const ICON_THEMES: [&str; 1] = ["Adwaita"];
const SETTINGS_APP: &str = "org.unbound.Settings";
const WALLPAPER_PANE: &str = "appearance";

type Loaded = (WallpaperKey, Option<(Picture, Luma)>);

struct Surface {
    view: DesktopView,
    apps: Apps,
    operations: Operations,
    watch: FolderWatch,
    desktop_dir: PathBuf,
    media: Vec<PathBuf>,
    data_dirs: Vec<PathBuf>,
    home: Option<PathBuf>,
    icons: HashMap<(String, u32), Option<Image>>,
    layouts_file: Option<PathBuf>,
    dock_edge: DockEdge,
    choice: Option<Choice>,
    choices: Inbox<Choice>,
    loaded: Inbox<Loaded>,
    loader: Outbox<Loaded>,
    pending: HashSet<WallpaperKey>,
    wallpapers: HashMap<WallpaperKey, Layer>,
    scheme: Scheme,
    viewport: Option<((u32, u32), f64)>,
}

impl Surface {
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
        let loader = self.loader.clone();
        let work = move || {
            let rendered = match render(&key.path, key.fit, key.width, key.height) {
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
                    self.view.show_wallpaper(layer.clone(), Instant::now());
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
                let mime = self.apps.mime_for(&item.path);
                let launcher = self
                    .apps
                    .launcher(&item.path, &mime)
                    .and_then(|entry| entry.icon);
                launcher
                    .into_iter()
                    .chain(self.apps.icon_names(&mime))
                    .collect()
            }
        };
        names.push("application-x-generic".to_owned());
        names
    }

    fn relist(&mut self) {
        let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
        let volumes = mounted_volumes(&mountinfo, &self.media);
        let items = list(&mut self.apps, &self.desktop_dir, &volumes);
        let pixels = self.icon_pixels();
        let with_icons = items
            .into_iter()
            .map(|item| {
                let names = self.icon_names(&item);
                let icon = self.icon(&names, pixels);
                (item, icon)
            })
            .collect();
        self.view.set_items(with_icons, Instant::now());
        self.watch_folders();
        self.save_layouts();
    }

    fn icon_pixels(&self) -> u32 {
        let scale = self.viewport.map_or(1.0, |(_, scale)| scale);
        (f64::from(unbound_desktop_surface::ICON_SIZE) * scale).ceil() as u32
    }

    fn watch_folders(&mut self) {
        let mut folders = vec![self.desktop_dir.clone()];
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
        let top = unbound_menubar::HEIGHT as f32;
        let dock = unbound_dock::exclusive_zone(unbound_dock::ICON_SIZE);
        let (left, right, bottom, top) = match self.dock_edge {
            DockEdge::Left => (dock, 0.0, 0.0, top),
            DockEdge::Right => (0.0, dock, 0.0, top),
            DockEdge::Bottom => (0.0, 0.0, dock, top),
            DockEdge::Top => (0.0, 0.0, 0.0, top + dock),
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
            Instant::now(),
        );
        if relist {
            self.icons.clear();
            self.relist();
        }
        self.save_layouts();
        self.refresh_wallpaper(window);
    }

    fn files_app(&self) -> Option<DesktopEntry> {
        self.apps.default_for(DIRECTORY).cloned()
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
                _ => self.apps.mime_for(&path),
            };
            if let Some(launcher) = self.apps.launcher(&path, &mime) {
                let token = window.activation_token(launcher.app_id()).ok();
                if let Err(error) = launch(&launcher, token.as_deref()) {
                    tracing::warn!("cannot start {}: {error}", launcher.name);
                }
                continue;
            }
            match self.apps.default_for(&mime).cloned() {
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

    fn common_mime(&self, paths: &[PathBuf]) -> String {
        let mimes: Vec<String> = paths
            .iter()
            .map(
                |path| match self.view.desktop().item(path).map(|item| item.kind) {
                    Some(Kind::Folder | Kind::Volume) => DIRECTORY.to_owned(),
                    _ => self.apps.mime_for(path),
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
            desktop_menu(self.view.desktop().sort())
        };
        if let Err(error) = window.show_menu(menu, MenuPlacement::AtPoint(at)) {
            tracing::warn!("cannot show the menu: {error}");
        }
    }

    fn change_wallpaper(&mut self, window: &mut Window) {
        let settings = unbound_xdg::applications(
            &self.data_dirs,
            unbound_xdg::Locale::from_env().as_ref(),
            &unbound_xdg::current_desktops(),
        )
        .into_iter()
        .find(|entry| entry.app_id() == SETTINGS_APP);
        let Some(settings) = settings else {
            tracing::warn!("Settings is not installed");
            return;
        };
        let token = window.activation_token(SETTINGS_APP).ok();
        let launched = settings
            .command()
            .map_err(|error| error.to_string())
            .and_then(|mut command| {
                command.push(WALLPAPER_PANE.to_owned());
                unbound_xdg::spawn_detached(
                    &command,
                    settings.working_directory.as_deref(),
                    token.as_deref(),
                )
                .map_err(|error| error.to_string())
            });
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
                Request::Rename { path, name } => match unbound_fs::rename(&path, &name) {
                    Ok(Some(change)) => self.operations.record(Action::Rename, change),
                    Ok(None) => {}
                    Err(error) => tracing::warn!("cannot rename {}: {error:?}", path.display()),
                },
                Request::NewFolder => match unbound_fs::new_folder(&self.desktop_dir) {
                    Ok(change) => {
                        if let Change::Created(path) = &change {
                            self.view.rename_when_listed(path.clone());
                        }
                        self.operations.record(Action::NewFolder, change);
                        self.relist();
                    }
                    Err(error) => tracing::warn!("cannot make a folder: {error}"),
                },
                Request::ShowInFiles(path) => self.show_in_files(window, path),
                Request::OpenDesktopFolder => {
                    let folder = self.desktop_dir.clone();
                    self.show_in_files(window, folder);
                }
                Request::ContextMenu { at, on_items } => self.context_menu(window, at, on_items),
                Request::OpenWith { app, paths } => {
                    let app = self
                        .apps
                        .apps_for(&self.common_mime(&paths))
                        .into_iter()
                        .find(|entry| entry.id == app)
                        .cloned();
                    if let Some(app) = app {
                        self.launch_with(window, &app, &paths);
                    }
                }
                Request::Copy(paths) => {
                    let text: Vec<String> = paths
                        .iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect();
                    window.copy(&text.join("\n"));
                }
                Request::Eject(paths) => eject(paths),
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

impl App for Surface {
    fn scene(&mut self, window: &mut Window) -> Scene {
        self.follow_viewport(window);
        self.view.scene(Instant::now())
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::Input(input) => {
                if self.view.handle(&input, Instant::now()) {
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
                    self.view.run_command(command, Instant::now());
                }
                self.run_requests(window);
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
        sources.extend(self.operations.fds());
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
                for (key, rendered) in self.loaded.drain() {
                    self.pending.remove(&key);
                    let layer = rendered.and_then(|(picture, luma)| {
                        Image::from_rgba(picture.width, picture.height, picture.rgba).map(|image| {
                            Layer {
                                key: key.clone(),
                                image,
                                luma,
                            }
                        })
                    });
                    if let Some(layer) = layer {
                        self.wallpapers.insert(key, layer);
                    }
                }
                self.refresh_wallpaper(window);
            }
            index => {
                self.poll_operation(index - 3);
                window.request_redraw();
            }
        }
    }

    fn deadline(&self) -> Option<Instant> {
        [self.watch.deadline(), self.view.deadline(Instant::now())]
            .into_iter()
            .flatten()
            .min()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        if !self.watch.take_changed(now).is_empty() {
            self.relist();
        }
        self.view.tick(now);
        window.request_redraw();
    }

    fn should_close(&mut self, _window: &mut Window) -> bool {
        false
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.view.accessibility())
    }
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

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config_home_dir = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let dock_edge = config_path(config_home_dir.as_deref(), home.as_deref())
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|source| Config::parse(&source).ok())
        .unwrap_or_default()
        .dock_edge;
    let desktop_dir = desktop_folder(home.as_deref().unwrap_or(Path::new("/")));
    let user = std::env::var("USER").unwrap_or_default();
    let trash = Trash::for_user().ok_or("no home folder for the Trash")?;
    let layouts_file = layouts_path();
    let layouts: Layouts = layouts_file
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| parse_layouts(&text))
        .unwrap_or_default();
    let mut window = Window::open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Background,
            anchor: Edges::ALL,
            exclusive_zone: -1,
            keyboard: KeyboardFocus::OnDemand,
            namespace: "desktop".to_owned(),
        }),
        (0, 0),
    )?;
    let (choices, choice_sender) = inbox()?;
    follow(choice_sender);
    let (loaded, loader) = inbox()?;
    let appearance = window.appearance();
    let data_dirs = data_dirs();
    let mut surface = Surface {
        view: DesktopView::new(appearance, layouts),
        apps: Apps::load(data_dirs.clone(), &config_dirs()),
        operations: Operations::new(trash),
        watch: FolderWatch::new()?,
        desktop_dir,
        media: media_roots(&user),
        data_dirs,
        home,
        icons: HashMap::new(),
        layouts_file,
        dock_edge,
        choice: None,
        choices,
        loaded,
        loader,
        pending: HashSet::new(),
        wallpapers: HashMap::new(),
        scheme: appearance.scheme,
        viewport: None,
    };
    unbound_toolkit::run(&mut window, &mut surface)?;
    Ok(())
}

fn main() -> ExitCode {
    unbound_log::init("unbound-desktop-surface");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
