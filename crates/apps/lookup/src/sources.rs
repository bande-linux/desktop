use std::cell::{Cell, RefCell};
use std::fs;
use std::os::fd::BorrowedFd;
use std::path::Path;
use std::rc::Rc;

use bande_app::View;
use bande_dict::{Choices, Pack};
use bande_inbox::Inbox;
use bande_toolkit::ui::{
    Axis, Button, CheckState, Checkbox, DragAction, Label, ScrollView, Stack, TextField, Ui, Widget,
};
use bande_toolkit::{Appearance, DragEvent, Scene, Spacing, TextSystem, Window, WindowEvent};

use crate::L10N;
use crate::import;
use crate::panel::ui_with;

pub const SETTINGS_SIZE: (u32, u32) = (520, 460);
const ANY_LANGUAGE: &str = "*";
const DICTIONARY_PATTERNS: [&str; 7] = [
    "*.ifo",
    "*.index",
    "*.tar.gz",
    "*.tgz",
    "*.tar.bz2",
    "*.tar.xz",
    "*.tar",
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Change {
    Enable(String, bool),
    Move(usize, usize),
    Remove(String),
    AddFile,
    AddServer,
}

type Imported = Result<String, String>;

pub struct SourcesWindow {
    ui: Ui,
    imports: Option<Inbox<Imported>>,
    importing: usize,
    status: Option<String>,
    server: Rc<RefCell<String>>,
    appearance: Appearance,
    choices: Choices,
    change: Rc<RefCell<Option<Change>>>,
    changed: Rc<Cell<bool>>,
}

pub fn sources_window(appearance: Appearance) -> Box<dyn View> {
    let mut window = SourcesWindow {
        ui: ui_with(
            Box::new(Stack::new(Axis::Vertical, 0.0, 0.0, Vec::new())),
            TextSystem::new(),
            appearance,
        ),
        appearance,
        imports: Inbox::new().ok(),
        importing: 0,
        status: None,
        server: Rc::new(RefCell::new(String::new())),
        choices: Choices::load(),
        change: Rc::new(RefCell::new(None)),
        changed: Rc::new(Cell::new(false)),
    };
    window.rebuild();
    Box::new(window)
}

fn languages_of(source: &Pack) -> String {
    match source
        .languages
        .iter()
        .any(|language| language == ANY_LANGUAGE)
    {
        true => L10N.tr("source-any-language"),
        false => source.languages.join(", "),
    }
}

fn detail(source: &Pack) -> String {
    let languages = languages_of(source);
    if source.online.is_some() {
        return L10N.tr_args(
            "source-online",
            [
                ("languages", languages.into()),
                ("name", source.name.as_str().into()),
            ],
        );
    }
    match source.is_users() {
        true => L10N.tr_args("source-yours", [("languages", languages.into())]),
        false => languages,
    }
}

fn imported(path: &Path) -> Imported {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    import::import_file(path).map_err(|failure| failure.message(&name))
}

impl SourcesWindow {
    fn start(&mut self, work: impl FnOnce() -> Imported + Send + 'static) {
        let Some(imports) = &self.imports else {
            return;
        };
        let outbox = imports.outbox();
        self.importing += 1;
        self.status = None;
        std::thread::spawn(move || outbox.send(work()));
    }

    fn dropped(&mut self, uris: &[String]) {
        for path in uris
            .iter()
            .filter_map(|uri| bande_xdg::path_from_file_uri(uri))
        {
            self.start(move || imported(&path));
        }
    }

    fn button(&self, label: &str, change: Change) -> Box<dyn Widget> {
        let slot = self.change.clone();
        let changed = self.changed.clone();
        Box::new(Button::new(&L10N.tr(label), move || {
            *slot.borrow_mut() = Some(change.clone());
            changed.set(true);
        }))
    }

    fn rebuild(&mut self) {
        let languages = bande_dict::user_languages();
        let sources = self.choices.arrange(bande_dict::installed(), &languages);
        let last = sources.len().saturating_sub(1);
        let mut rows: Vec<Box<dyn Widget>> =
            vec![Box::new(Label::new(&L10N.tr("sources-intro")).secondary())];
        for (index, source) in sources.iter().enumerate() {
            let on = self.choices.enabled(source, &languages);
            let id = source.id.clone();
            let slot = self.change.clone();
            let changed = self.changed.clone();
            let state = match on {
                true => CheckState::On,
                false => CheckState::Off,
            };
            let check = Checkbox::new(&source.name, state, move |on| {
                *slot.borrow_mut() = Some(Change::Enable(id.clone(), on));
                changed.set(true);
            });
            let mut controls: Vec<Box<dyn Widget>> = Vec::new();
            if index > 0 {
                controls.push(self.button("source-move-up", Change::Move(index, index - 1)));
            }
            if index < last {
                controls.push(self.button("source-move-down", Change::Move(index, index + 1)));
            }
            if source.is_users() {
                controls.push(self.button("source-remove", Change::Remove(source.id.clone())));
            }
            rows.push(Box::new(Stack::new(
                Axis::Vertical,
                Spacing::XS as f32,
                0.0,
                vec![
                    Box::new(check),
                    Box::new(Label::new(&detail(source)).secondary()),
                    Box::new(Stack::new(
                        Axis::Horizontal,
                        Spacing::S as f32,
                        0.0,
                        controls,
                    )),
                ],
            )));
        }
        let add = self.button("add-dictionary", Change::AddFile);
        let server = self.server.clone();
        let field = TextField::new(
            &L10N.tr("server-placeholder"),
            &self.server.borrow(),
            move |text| {
                text.clone_into(&mut server.borrow_mut());
            },
        )
        .without_text_checking();
        let add_server = self.button("add-server", Change::AddServer);
        rows.push(Box::new(Stack::new(
            Axis::Vertical,
            Spacing::S as f32,
            0.0,
            vec![
                add,
                Box::new(Label::new(&L10N.tr("add-dictionary-detail")).secondary()),
                Box::new(Stack::new(
                    Axis::Horizontal,
                    Spacing::S as f32,
                    0.0,
                    vec![Box::new(field), add_server],
                )),
            ],
        )));
        let status = match (self.importing, &self.status) {
            (0, Some(status)) => Some(status.clone()),
            (0, None) => None,
            (_, _) => Some(L10N.tr("importing")),
        };
        if let Some(status) = status {
            rows.push(Box::new(Label::new(&status).secondary()));
        }
        let padding = Spacing::L as f32;
        let root = ScrollView::new(Stack::new(Axis::Vertical, Spacing::M as f32, padding, rows));
        let text = self.ui.context().text.sharing_fonts();
        self.ui = ui_with(Box::new(root), text, self.appearance);
    }

    fn apply(&mut self, change: Change) {
        let languages = bande_dict::user_languages();
        let sources = self.choices.arrange(bande_dict::installed(), &languages);
        match change {
            Change::Enable(id, on) => self.choices.set_enabled(&id, on),
            Change::Move(from, to) => {
                let mut order: Vec<String> =
                    sources.iter().map(|source| source.id.clone()).collect();
                if from < order.len() && to < order.len() {
                    order.swap(from, to);
                }
                self.choices.order = order;
            }
            Change::AddFile => {
                self.start(|| {
                    let session = bande_bus::session().map_err(|error| error.to_string())?;
                    let filter = (
                        L10N.tr("dictionary-files"),
                        DICTIONARY_PATTERNS
                            .iter()
                            .map(|pattern| (0u32, (*pattern).to_owned()))
                            .collect::<Vec<_>>(),
                    );
                    let chosen =
                        bande_app::choose_file(&session, &L10N.tr("add-dictionary"), Some(filter))
                            .map_err(|error| error.to_string())?;
                    match chosen {
                        Some(path) => imported(&path),
                        None => Ok(String::new()),
                    }
                });
                return;
            }
            Change::AddServer => {
                let url = self.server.borrow().trim().to_owned();
                self.start(move || {
                    import::add_server(&url).map_err(|failure| failure.message(&url))
                });
                return;
            }
            Change::Remove(id) => {
                if let Some(source) = sources
                    .iter()
                    .find(|source| source.id == id && source.is_users())
                    && let Err(error) = fs::remove_file(&source.path)
                {
                    tracing::warn!("cannot remove {}: {error}", source.path.display());
                }
                self.choices.order.retain(|known| *known != id);
                self.choices.on.retain(|known| *known != id);
                self.choices.off.retain(|known| *known != id);
            }
        }
        let saved = bande_xdg::config_home()
            .ok_or_else(|| std::io::Error::other("no configuration folder"))
            .and_then(|folder| {
                bande_xdg::replace_file(
                    &folder.join("bande").join(Choices::FILE),
                    self.choices.to_text(),
                )
            });
        if let Err(error) = saved {
            tracing::warn!("cannot save the dictionary settings: {error}");
        }
    }
}

impl View for SourcesWindow {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let mut scene = self.ui.scene(width, height);
        scene.background = window.appearance().palette().sheet;
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        if let WindowEvent::Drag(drag) = &event {
            match drag {
                DragEvent::Entered { uris, .. } => {
                    let files = uris
                        .iter()
                        .any(|uri| bande_xdg::path_from_file_uri(uri).is_some());
                    window.accept_drag(files.then_some(DragAction::Copy));
                }
                DragEvent::Moved(_) => window.accept_drag(Some(DragAction::Copy)),
                DragEvent::Dropped { transfer, .. } => {
                    self.dropped(&transfer.uris);
                    self.rebuild();
                    window.request_redraw();
                }
                DragEvent::Left | DragEvent::Finished(_) => {}
            }
            return true;
        }
        let handled = window
            .drive(&mut self.ui, event)
            .inspect_err(|error| tracing::warn!("{error}"))
            .is_ok_and(|unhandled| unhandled.is_none());
        if self.changed.replace(false) {
            let change = self.change.borrow_mut().take();
            if let Some(change) = change {
                self.apply(change);
            }
            self.rebuild();
            window.request_redraw();
        }
        handled
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.imports.iter().map(Inbox::fd).collect()
    }

    fn source_ready(&mut self, _window: &mut Window, _index: usize) -> bool {
        let Some(imports) = &mut self.imports else {
            return false;
        };
        let arrived = imports.drain();
        if arrived.is_empty() {
            return false;
        }
        for result in arrived {
            self.importing = self.importing.saturating_sub(1);
            self.status = match result {
                Ok(name) if name.is_empty() => None,
                Ok(name) => Some(L10N.tr_args("imported", [("name", name.into())])),
                Err(message) => Some(message),
            };
        }
        self.rebuild();
        true
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.appearance = appearance;
        self.ui.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.ui.set_window_active(active);
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.ui.accessibility())
    }
}
