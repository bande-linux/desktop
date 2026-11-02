use std::cell::RefCell;
use std::collections::HashMap;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{Sender, channel};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    Action, AppInfo, Catalog, Clips, Hit, ICON_SIZE, Kind, Launcher, Loaded, Mode, PaneInfo, Reply,
    Request, SettingInfo, SystemCommand, Target,
};
use accesskit::TreeUpdate;
use bande_config::{Config, Search, SearchCategory, config_path};
use bande_fs::Entry;
use bande_inbox::Inbox;
use bande_preview::{Preview, preview};
use bande_search::{FileIndex, RecentlyUsed, Usage, recently_used_path, usage_path};
use bande_thumbnails::{Cache, ThumbnailSize, Thumbnailers, thumbnail};
use bande_toolkit::{
    App, Color, Edges, Image, KeyboardFocus, LauncherMode, LayerKind, LayerOptions, MenuPlacement,
    Rect, Role, Scene, SessionEnd, TextSystem, Window, WindowError, WindowEvent, ui::frame_time,
};
use bande_xdg::{
    Apps, DIRECTORY, DesktopEntry, ICON_THEMES, LaunchError, Locale, MimeDatabase, applications,
    current_desktops, data_dirs, find_icon, launch, launch_with, launch_with_arguments,
};

const FALLBACK_DOCUMENT: &str = "application-x-generic";
const CLIP_TEXT: &str = "text-x-generic";
const CLIP_IMAGE: &str = "image-x-generic";
const CLIP_FILES: &str = "folder";
const SETTINGS_APP: &str = "org.bande.Settings";

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

impl Library {
    fn command_glyph(&mut self, command: SystemCommand) -> Option<Image> {
        let name = match command {
            SystemCommand::LockScreen => "system-lock-screen-symbolic",
            SystemCommand::Sleep => "weather-clear-night-symbolic",
            SystemCommand::LogOut => "system-log-out-symbolic",
            SystemCommand::Restart => "system-reboot-symbolic",
            SystemCommand::ShutDown => "system-shutdown-symbolic",
        };
        let data_dirs = &self.data_dirs;
        self.icons
            .entry(name.to_owned())
            .or_insert_with(|| {
                find_icon(name, ICON_SIZE, &ICON_THEMES, data_dirs)
                    .and_then(|path| Image::load_symbolic(&path, ICON_SIZE, Color::WHITE))
            })
            .clone()
    }
}

fn carry_out(window: &mut Window, command: SystemCommand) -> std::io::Result<()> {
    let ending = match command {
        SystemCommand::LockScreen | SystemCommand::Sleep => {
            std::thread::spawn(move || {
                let result = zbus::blocking::Connection::system().and_then(|system| {
                    if command == SystemCommand::Sleep {
                        bande_status::devices::sleep(&system)
                    } else {
                        bande_status::devices::lock_screen(&system)
                    }
                });
                if let Err(error) = result {
                    tracing::warn!("cannot carry out {command:?}: {error}");
                }
            });
            return Ok(());
        }
        SystemCommand::LogOut => SessionEnd::LogOut,
        SystemCommand::Restart => SessionEnd::Restart,
        SystemCommand::ShutDown => SessionEnd::ShutDown,
    };
    window
        .confirm_session_end(ending)
        .map_err(std::io::Error::other)
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
            (Target::Clip(_), Kind::CopiedImage) => self.named_icon(&[CLIP_IMAGE.to_owned()]),
            (Target::Clip(_), Kind::CopiedFiles) => self.named_icon(&[CLIP_FILES.to_owned()]),
            (Target::Clip(_), _) => self.named_icon(&[CLIP_TEXT.to_owned()]),
            (Target::Command(command), _) => self.command_glyph(*command),
            (Target::Value(_) | Target::Emoji(_), _) => None,
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
    results: Inbox<Loaded>,
}

impl Previewer {
    fn spawn() -> std::io::Result<Self> {
        let (requests, wanted) = channel::<PathBuf>();
        let results = Inbox::new()?;
        let sender = results.outbox();
        std::thread::Builder::new()
            .name("preview".to_owned())
            .spawn(move || {
                let cache = Cache::for_user();
                let thumbnailers = Thumbnailers::load(&data_dirs());
                let mimes = MimeDatabase::load(&data_dirs());
                while let Ok(mut path) = wanted.recv() {
                    while let Ok(newer) = wanted.try_recv() {
                        path = newer;
                    }
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let entry = Entry::read(path, name);
                    let mut content = preview(&entry, Image::from_rgba);
                    if let (Preview::Other { .. }, Some(cache)) = (&content, &cache)
                        && let Some(picture) = thumbnail(
                            cache,
                            &thumbnailers,
                            &entry.path,
                            &mimes.for_path(&entry.path),
                            ThumbnailSize::XLarge,
                        )
                        && let Some(image) =
                            Image::from_rgba(picture.width, picture.height, picture.rgba)
                    {
                        content = Preview::Image(image);
                    }
                    if !sender.send(Loaded { entry, content }) {
                        return;
                    }
                }
            })?;
        Ok(Self { requests, results })
    }
}

pub struct LauncherShell {
    launcher: Launcher,
    entries: Vec<DesktopEntry>,
    index: Option<FileIndex>,
    previewer: Option<Previewer>,
    clips: Option<Clips>,
    apps: Rc<RefCell<Apps>>,
    recently_used: Option<RecentlyUsed>,
    config: Option<PathBuf>,
    home: PathBuf,
    skin_tone: PathBuf,
    scrim: Option<f32>,
}

fn load_skin_tone(path: &Path) -> Option<usize> {
    std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse::<usize>()
        .ok()?
        .checked_sub(1)
}

fn save_skin_tone(path: &Path, tone: Option<usize>) -> std::io::Result<()> {
    bande_xdg::replace_file(path, tone.map_or(0, |tone| tone + 1).to_string())
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

    fn follow_search(&mut self) -> bool {
        let search: Search = self
            .config
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|source| Config::load(&source).ok())
            .map(|(config, _)| config.search)
            .unwrap_or_default();
        let excluded = search.excluded(Some(&self.home));
        let recents = search.shows(SearchCategory::RecentDocuments);
        if !search.shows(SearchCategory::Documents) && !search.shows(SearchCategory::Folders) {
            self.index = None;
        } else if let Some(index) = &self.index {
            index.exclude(excluded.clone());
        } else {
            self.index = FileIndex::spawn(self.home.clone(), excluded.clone())
                .inspect_err(|error| tracing::warn!("cannot index the home folder: {error}"))
                .ok();
            if let Some(index) = &self.index {
                index.set_boosts(self.launcher.file_boosts());
            }
        }
        self.launcher.set_search(search, excluded);
        recents
    }

    fn ask(&self, request: Request) {
        if let Some(clips) = &self.clips {
            clips.ask(request);
        }
    }

    fn toggle(&mut self, window: &mut Window, mode: Mode, inserts: bool) {
        if window.is_visible() && self.launcher.mode() == mode {
            self.close(window);
            return;
        }
        self.launcher.set_now(unix_now());
        if mode == Mode::Search {
            let recents = match (self.follow_search(), &self.recently_used) {
                (true, Some(recently_used)) => recently_used.recents(),
                _ => &[],
            };
            self.launcher.set_recently_used(recents);
        }
        self.launcher.open(mode, inserts);
        if mode == Mode::Clipboard {
            self.ask(Request::List);
        }
        if !window.is_visible() {
            if let Err(error) = self.show(window) {
                tracing::warn!("cannot show: {error}");
            }
            self.launcher.appear(frame_time().on_screen());
        }
        window.request_redraw();
    }

    fn show(&mut self, window: &mut Window) -> Result<(), WindowError> {
        window.set_layer_anchor(Edges::ALL);
        window.set_layer_margin(0);
        window.set_layer_size((0, 0))?;
        window.show()?;
        let surface = self.launcher.surface(window.size());
        window.set_layer_anchor(Edges {
            top: true,
            bottom: false,
            left: false,
            right: false,
        });
        window.set_layer_margin(surface.top);
        self.scrim = None;
        window.set_layer_size(surface.size)
    }

    fn sync(&mut self, window: &mut Window) {
        if let Some((menu, point)) = self.launcher.take_menu_request()
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
        if let Some((id, mime)) = self.launcher.clip_picture_request() {
            self.ask(Request::Picture(id, mime));
        }
        for action in self.launcher.take_actions() {
            match action {
                Action::Open(target) => self.open(window, &target),
                Action::Reveal(path) => self.reveal(window, &path),
                Action::Copy(text) => window.copy(&text),
                Action::Insert(text) => {
                    if let Err(error) = window.insert_text(&text) {
                        tracing::warn!("cannot insert {text}: {error}");
                    }
                }
                Action::PasteClip(id) => self.ask(Request::Paste(id)),
                Action::RemoveClip(id) => self.ask(Request::Remove(id)),
                Action::ClearClips => self.ask(Request::Clear),
                Action::SkinTone(tone) => {
                    if let Err(error) = save_skin_tone(&self.skin_tone, tone) {
                        tracing::warn!("cannot remember the skin tone: {error}");
                    }
                }
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
                let handler = apps.default_for_path(path).cloned();
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
                    launch_with_arguments(entry, std::slice::from_ref(keyword), token.as_deref())
                        .map(|_| ())
                }),
            Target::Command(command) => {
                Some(carry_out(window, *command).map_err(LaunchError::Spawn))
            }
            Target::Value(_) | Target::Emoji(_) | Target::Clip(_) => None,
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
        let now = frame_time().presentation;
        let scrim = self.launcher.opacity(now);
        if self.scrim != Some(scrim) {
            self.scrim = Some(scrim);
            if let Err(error) = window.set_scrim(scrim) {
                tracing::info!("the screen behind the launcher is not dimmed: {error}");
            }
        }
        self.launcher.scene(width, height, now)
    }

    fn animating(&self) -> bool {
        self.launcher.is_animating(frame_time().presentation)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::ShowLauncher => self.toggle(window, Mode::Search, false),
            WindowEvent::ScrimPressed => self.close(window),
            WindowEvent::OpenLauncher { mode, text_field } => {
                let mode = match mode {
                    LauncherMode::Clipboard => Mode::Clipboard,
                    _ => Mode::Emoji,
                };
                self.toggle(window, mode, text_field);
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
                self.launcher.menu_action(&action);
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
                    .map(|previewer| previewer.results.fd()),
            )
            .chain(self.clips.iter().map(Clips::fd))
            .chain(self.recently_used.iter().map(RecentlyUsed::fd))
            .collect()
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) {
        if let Some(recently_used) = &mut self.recently_used {
            recently_used.take();
        }
        if let Some(results) = self.index.as_mut().and_then(FileIndex::take) {
            self.launcher.set_files(results);
        }
        let previews = self
            .previewer
            .as_mut()
            .map(|previewer| previewer.results.drain())
            .unwrap_or_default();
        for loaded in previews {
            self.launcher.set_preview(loaded);
        }
        let replies = self.clips.as_mut().map(Clips::take).unwrap_or_default();
        for reply in replies {
            match reply {
                Reply::Listed(clips) => self.launcher.set_clips(clips),
                Reply::Picture(id, picture) => self.launcher.set_clip_picture(id, picture),
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
    bande_settings_panes::panes()
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

pub fn role() -> Role {
    Role::Layer(LayerOptions {
        layer: LayerKind::Overlay,
        anchor: Edges::ALL,
        exclusive_zone: -1,
        keyboard: KeyboardFocus::Exclusive,
        namespace: "launcher".to_owned(),
    })
}

impl LauncherShell {
    pub fn new(
        window: &mut Window,
        text: TextSystem,
        session: Option<&zbus::blocking::Connection>,
        apps_by_type: Rc<RefCell<Apps>>,
    ) -> std::io::Result<Self> {
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
        let recently_used = RecentlyUsed::follow(recently_used_path(
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .as_deref(),
            &home,
        ))
        .inspect_err(|error| tracing::warn!("cannot follow recent documents: {error}"))
        .ok();
        let usage_file = usage_path(bande_xdg::state_home().as_deref(), Some(&home));
        let skin_tone = usage_file
            .as_deref()
            .and_then(Path::parent)
            .unwrap_or(&home)
            .join("launcher-skin-tone");
        let usage = usage_file.map(Usage::load).unwrap_or_default();
        let library = Library {
            apps: apps_by_type.clone(),
            app_icons: app_icons(&entries, &dirs),
            data_dirs: dirs,
            icons: HashMap::new(),
        };
        let previewer = Previewer::spawn()
            .inspect_err(|error| tracing::warn!("cannot preview: {error}"))
            .ok();
        if let Err(error) = window.hide() {
            tracing::warn!("cannot hide: {error}");
        }
        let mut launcher = Launcher::new(
            apps,
            settings_panes(),
            Box::new(library),
            home.clone(),
            usage,
            window.appearance(),
            text,
        );
        launcher.set_skin_tone(load_skin_tone(&skin_tone));
        let clips = session.and_then(|session| {
            Clips::spawn(session.clone())
                .inspect_err(|error| tracing::warn!("cannot read the clipboard history: {error}"))
                .ok()
        });
        let mut shell = Self {
            launcher,
            entries,
            index: None,
            previewer,
            clips,
            apps: apps_by_type,
            recently_used,
            config: config_path(
                std::env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .as_deref(),
                Some(&home),
            ),
            home,
            skin_tone,
            scrim: None,
        };
        shell.follow_search();
        Ok(shell)
    }
}
