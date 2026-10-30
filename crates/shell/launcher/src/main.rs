use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use accesskit::TreeUpdate;
use unbound_files::{Apps, Entry, preview};
use unbound_launcher::{
    Action, AppInfo, Catalog, Hit, ICON_SIZE, Kind, Launcher, Loaded, PaneInfo, SettingInfo, Target,
};
use unbound_search::{FileIndex, Usage, usage_path};
use unbound_toolkit::{
    App, Edges, Image, KeyboardFocus, LayerKind, LayerOptions, MenuPlacement, Rect, Role, Scene,
    TextSystem, Window, WindowError, WindowEvent,
};
use unbound_xdg::{
    DIRECTORY, DesktopEntry, LaunchError, Locale, applications, config_dirs, current_desktops,
    data_dirs, find_icon, launch, launch_with, spawn_detached,
};

const ICON_THEMES: [&str; 1] = ["Adwaita"];
const FALLBACK_DOCUMENT: &str = "application-x-generic";
const SETTINGS_APP: &str = "org.unbound.Settings";

struct Library {
    apps: Rc<RefCell<Apps>>,
    data_dirs: Vec<PathBuf>,
    app_icons: HashMap<String, Option<Image>>,
    icons: HashMap<String, Option<Image>>,
}

impl Library {
    fn named_icon(&mut self, names: &[String]) -> Option<Image> {
        let data_dirs = &self.data_dirs;
        names.iter().find_map(|name| {
            self.icons
                .entry(name.clone())
                .or_insert_with(|| {
                    find_icon(name, ICON_SIZE, &ICON_THEMES, data_dirs)
                        .and_then(|path| Image::load(&path, ICON_SIZE))
                })
                .clone()
        })
    }
}

impl Catalog for Library {
    fn icon(&mut self, hit: &Hit) -> Option<Image> {
        match (&hit.target, hit.kind) {
            (Target::App(app_id), _) => self.app_icons.get(app_id).cloned().flatten(),
            (Target::Settings(_), _) => self.app_icons.get(SETTINGS_APP).cloned().flatten(),
            (Target::Path(_), Kind::Folder) => {
                let names = self.apps.borrow().icon_names(DIRECTORY);
                self.named_icon(&names)
            }
            (Target::Path(path), _) => {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let mut names = self.apps.borrow().icon_names_for_name(&name);
                names.push(FALLBACK_DOCUMENT.to_owned());
                self.named_icon(&names)
            }
            (Target::Value(_), _) => None,
        }
    }

    fn kind(&mut self, path: &Path, folder: bool) -> String {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.apps.borrow_mut().kind_for_name(&name, folder)
    }
}

struct Previewer {
    requests: Sender<PathBuf>,
    results: Receiver<Loaded>,
    wake: UnixStream,
}

impl Previewer {
    fn spawn() -> std::io::Result<Self> {
        let (requests, wanted) = channel::<PathBuf>();
        let (sender, results) = channel();
        let (wake, signal) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        signal.set_nonblocking(true)?;
        std::thread::Builder::new()
            .name("preview".to_owned())
            .spawn(move || {
                while let Ok(mut path) = wanted.recv() {
                    while let Ok(newer) = wanted.try_recv() {
                        path = newer;
                    }
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let entry = Entry::read(path, name);
                    let content = preview(&entry);
                    if sender.send(Loaded { entry, content }).is_err() {
                        return;
                    }
                    let _ = std::io::Write::write(&mut &signal, &[1]);
                }
            })?;
        Ok(Self {
            requests,
            results,
            wake,
        })
    }

    fn take(&self) -> Vec<Loaded> {
        let mut drained = [0; 64];
        while matches!((&self.wake).read(&mut drained), Ok(read) if read > 0) {}
        self.results.try_iter().collect()
    }
}

struct LauncherShell {
    launcher: Launcher,
    entries: Vec<DesktopEntry>,
    index: Option<FileIndex>,
    previewer: Option<Previewer>,
    apps: Rc<RefCell<Apps>>,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

impl LauncherShell {
    fn close(&mut self, window: &mut Window) {
        if let Err(error) = window.hide() {
            tracing::warn!("cannot hide: {error}");
        }
        self.launcher.clear();
    }

    fn sync(&mut self, window: &mut Window) {
        if let Some((menu, point)) = self.launcher.ui.take_menu_request()
            && let Err(error) = window.show_menu(menu, MenuPlacement::AtPoint(point))
        {
            tracing::warn!("cannot show the menu: {error}");
        }
        if let (Some(index), Some((generation, query))) = (&self.index, self.launcher.file_query())
        {
            index.query(generation, &query);
        }
        if let (Some(previewer), Some(path)) = (&self.previewer, self.launcher.preview_request()) {
            let _ = previewer.requests.send(path);
        }
        for action in self.launcher.take_actions() {
            match action {
                Action::Open(target) => self.open(window, &target),
                Action::Reveal(path) => self.reveal(window, &path),
                Action::Copy(text) => window.copy(&text),
                Action::Close => self.close(window),
            }
        }
        window.request_redraw();
    }

    fn open(&mut self, window: &mut Window, target: &Target) {
        let opened = match target {
            Target::App(app_id) => self
                .entries
                .iter()
                .find(|entry| entry.app_id() == app_id)
                .map(|entry| {
                    let token = window.activation_token(app_id).ok();
                    launch(entry, token.as_deref()).map(|_| ())
                }),
            Target::Path(path) => {
                let apps = self.apps.borrow();
                let handler = apps.default_for(&apps.mime_for(path)).cloned();
                drop(apps);
                handler.map(|entry| {
                    let token = window.activation_token(entry.app_id()).ok();
                    launch_with(&entry, std::slice::from_ref(path), token.as_deref())
                })
            }
            Target::Settings(keyword) => self
                .entries
                .iter()
                .find(|entry| entry.app_id() == SETTINGS_APP)
                .map(|entry| {
                    let token = window.activation_token(SETTINGS_APP).ok();
                    let mut command = entry.command().map_err(LaunchError::Exec)?;
                    command.push(keyword.clone());
                    spawn_detached(&command, None, token.as_deref())
                        .map(|_| ())
                        .map_err(LaunchError::Spawn)
                }),
            Target::Value(_) => None,
        };
        match opened {
            Some(Ok(())) => {
                if let Err(error) = self.launcher.record_use(target) {
                    tracing::warn!("cannot remember what was opened: {error}");
                }
                if let (Target::Path(_), Some(index)) = (target, &self.index) {
                    index.set_boosts(self.launcher.file_boosts());
                }
            }
            Some(Err(error)) => tracing::warn!("cannot open {target:?}: {error}"),
            None => tracing::warn!("nothing opens {target:?}"),
        }
        self.close(window);
    }

    fn reveal(&mut self, window: &mut Window, path: &Path) {
        let files = self.apps.borrow().default_for(DIRECTORY).cloned();
        if let Some(files) = files {
            let token = window.activation_token(files.app_id()).ok();
            if let Err(error) = launch_with(&files, &[path.to_path_buf()], token.as_deref()) {
                tracing::warn!("cannot show {}: {error}", path.display());
            }
        }
        self.close(window);
    }
}

impl App for LauncherShell {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let now = Instant::now();
        let scene = self.launcher.scene(width, height, now);
        if self.launcher.is_animating(now) {
            window.request_redraw();
        }
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::ShowLauncher if window.is_visible() => self.close(window),
            WindowEvent::ShowLauncher => {
                self.launcher.set_now(unix_now());
                self.launcher.appear(Instant::now());
                if let Err(error) = window.show() {
                    tracing::warn!("cannot show: {error}");
                }
            }
            WindowEvent::Input(input) => {
                self.launcher.handle(&input);
                self.sync(window);
            }
            WindowEvent::Accessibility(request) => {
                self.launcher.perform(&request);
                self.sync(window);
            }
            WindowEvent::AppearanceChanged(appearance) => {
                self.launcher.set_appearance(appearance);
            }
            WindowEvent::MenuAction(action) => {
                self.launcher.ui.menu_action(&action);
                self.sync(window);
            }
            _ => {}
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.index
            .iter()
            .map(FileIndex::fd)
            .chain(
                self.previewer
                    .iter()
                    .map(|previewer| previewer.wake.as_fd()),
            )
            .collect()
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        match (index, &mut self.index) {
            (0, Some(files)) => {
                if let Some(results) = files.take() {
                    self.launcher.set_files(results);
                }
            }
            _ => {
                for loaded in self.previewer.iter().flat_map(Previewer::take) {
                    self.launcher.set_preview(loaded);
                }
            }
        }
        self.sync(window);
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.launcher.ui.text_input_area()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.launcher.ui.accessibility())
    }
}

fn app_icons(entries: &[DesktopEntry], data_dirs: &[PathBuf]) -> HashMap<String, Option<Image>> {
    let mut by_name: HashMap<&str, Option<Image>> = HashMap::new();
    entries
        .iter()
        .map(|entry| {
            let icon = entry.icon.as_deref().and_then(|name| {
                by_name
                    .entry(name)
                    .or_insert_with(|| {
                        find_icon(name, ICON_SIZE, &[], data_dirs)
                            .and_then(|path| Image::load(&path, ICON_SIZE))
                    })
                    .clone()
            });
            (entry.app_id().to_owned(), icon)
        })
        .collect()
}

fn settings_panes() -> Vec<PaneInfo> {
    unbound_settings_panes::panes()
        .into_iter()
        .map(|pane| PaneInfo {
            keyword: pane.id.keyword().to_owned(),
            settings: pane
                .settings()
                .map(|setting| SettingInfo {
                    label: setting.label.clone(),
                    keywords: setting.keywords.clone(),
                })
                .collect(),
            name: pane.name,
            keywords: pane.keywords,
        })
        .collect()
}

fn run() -> Result<(), WindowError> {
    let dirs = data_dirs();
    let entries = applications(&dirs, Locale::from_env().as_ref(), &current_desktops());
    let apps = entries
        .iter()
        .map(|entry| AppInfo {
            app_id: entry.app_id().to_owned(),
            name: entry.name.clone(),
            generic_name: entry.generic_name.clone(),
            comment: entry.comment.clone(),
            keywords: entry.keywords.clone(),
        })
        .collect();
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
    let state_home = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from);
    let usage = usage_path(state_home.as_deref(), Some(&home))
        .map(Usage::load)
        .unwrap_or_default();
    let apps_by_type = Rc::new(RefCell::new(Apps::load(dirs.clone(), &config_dirs())));
    let library = Library {
        apps: apps_by_type.clone(),
        app_icons: app_icons(&entries, &dirs),
        data_dirs: dirs,
        icons: HashMap::new(),
    };
    let index = FileIndex::spawn(home.clone())
        .inspect_err(|error| tracing::warn!("cannot index the home folder: {error}"))
        .ok();
    let previewer = Previewer::spawn()
        .inspect_err(|error| tracing::warn!("cannot preview: {error}"))
        .ok();
    let mut window = Window::open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Overlay,
            anchor: Edges::ALL,
            exclusive_zone: -1,
            keyboard: KeyboardFocus::Exclusive,
            namespace: "launcher".to_owned(),
        }),
        (0, 0),
    )?;
    window.hide()?;
    let launcher = Launcher::new(
        apps,
        settings_panes(),
        Box::new(library),
        home,
        usage,
        window.appearance(),
        TextSystem::new(),
    );
    if let Some(index) = &index {
        index.set_boosts(launcher.file_boosts());
    }
    let mut shell = LauncherShell {
        launcher,
        entries,
        index,
        previewer,
        apps: apps_by_type,
    };
    unbound_toolkit::run(&mut window, &mut shell)
}

fn main() -> ExitCode {
    unbound_log::init("unbound-launcher");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
