use std::cell::RefCell;
use std::fs;
use std::io;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use accesskit::TreeUpdate;
use bande_app::View;
use bande_inbox::Inbox;
use bande_toolkit::ui::Ui;
use bande_toolkit::{Appearance, Rect, Scene, Window, WindowEvent};
use bande_watch::{Changes, FolderWatch};

pub trait Stored: Clone + PartialEq + Default + 'static {
    const FILE: &'static str;

    fn parse(text: &str) -> Self;

    fn to_text(&self) -> String;

    fn load(folder: &Path) -> Self {
        fs::read_to_string(folder.join(Self::FILE))
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    fn save(&self, folder: &Path) -> io::Result<()> {
        bande_xdg::replace_file(&folder.join(Self::FILE), &self.to_text())
    }
}

pub type Shared<S> = Rc<RefCell<S>>;

pub fn settings_folder() -> Option<PathBuf> {
    bande_xdg::config_home().map(|config| config.join("bande"))
}

pub fn pairs(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()))
}

pub fn flag(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

pub struct SettingsWatch {
    _watch: FolderWatch,
    changes: Inbox<()>,
}

impl SettingsWatch {
    pub fn start(folder: &Path, file: &'static str) -> Option<Self> {
        let started = fs::create_dir_all(folder).and_then(|()| {
            let changes = Inbox::new()?;
            let changed = changes.outbox();
            let watch = FolderWatch::spawn(Changes::Saved, move |paths| {
                if paths
                    .iter()
                    .any(|path| path.file_name().is_some_and(|name| name == file))
                {
                    changed.send(());
                }
            })?;
            watch.watch(folder)?;
            Ok(Self {
                _watch: watch,
                changes,
            })
        });
        started
            .inspect_err(|error| tracing::warn!("cannot follow the settings: {error}"))
            .ok()
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.changes.fd()
    }

    pub fn drain(&mut self) -> bool {
        !self.changes.drain().is_empty()
    }
}

pub type Build<S> = fn(Appearance, &Shared<S>) -> Ui;

pub struct SettingsWindow<S: Stored> {
    ui: Ui,
    settings: Shared<S>,
    saved: S,
    unsaved: Option<S>,
    folder: Option<PathBuf>,
    watch: Option<SettingsWatch>,
    build: Build<S>,
}

impl<S: Stored> SettingsWindow<S> {
    pub fn new(appearance: Appearance, folder: Option<PathBuf>, build: Build<S>) -> Self {
        let saved = folder.as_deref().map(S::load).unwrap_or_default();
        let settings = Rc::new(RefCell::new(saved.clone()));
        let watch = folder
            .as_deref()
            .and_then(|folder| SettingsWatch::start(folder, S::FILE));
        Self {
            ui: build(appearance, &settings),
            settings,
            saved,
            unsaved: None,
            folder,
            watch,
            build,
        }
    }

    pub fn settings(&self) -> S {
        self.settings.borrow().clone()
    }

    pub fn follow(&mut self, appearance: Appearance) -> bool {
        let Some(folder) = &self.folder else {
            return false;
        };
        let stored = S::load(folder);
        if stored == self.saved {
            return false;
        }
        self.saved = stored.clone();
        *self.settings.borrow_mut() = stored;
        self.ui = (self.build)(appearance, &self.settings);
        true
    }

    fn save(&mut self) {
        let settings = self.settings();
        if settings == self.saved || self.unsaved.as_ref() == Some(&settings) {
            return;
        }
        let Some(folder) = &self.folder else {
            return;
        };
        match settings.save(folder) {
            Ok(()) => {
                self.saved = settings;
                self.unsaved = None;
            }
            Err(error) => {
                tracing::warn!("cannot save {}: {error}", S::FILE);
                self.unsaved = Some(settings);
            }
        }
    }
}

impl<S: Stored> View for SettingsWindow<S> {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let mut scene = self.ui.scene(width, height);
        scene.background = window.appearance().palette().sheet;
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        let handled = View::event(&mut self.ui, window, event);
        self.save();
        handled
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.ui.set_window_active(active);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.watch.iter().map(SettingsWatch::fd).collect()
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) -> bool {
        if let Some(watch) = &mut self.watch {
            watch.drain();
        }
        self.follow(window.appearance())
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }
}
