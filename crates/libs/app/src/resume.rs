use std::cell::Cell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::LazyLock;
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bande_l10n::Localizer;
use bande_toolkit::ui::{Alert, Context, Ui};
use bande_toolkit::{AppConnection, Appearance, Size, TextSystem, WindowError, WindowInSession};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cx::Content;
use crate::{Cx, Kind, Program, WindowId};

const ENGLISH: &str = include_str!("../locales/en-US/bande-app.ftl");
static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

const RESUME_FOLDER: &str = "bande/resume";
const SAVE_AFTER: Duration = Duration::from_secs(1);
const SAFE_AFTER: Duration = Duration::from_secs(5);
const REOPEN: &str = "reopen";
const DO_NOT_REOPEN: &str = "do-not-reopen";

#[derive(Debug, Clone, PartialEq)]
pub struct Saved(pub(crate) Value);

impl Saved {
    pub fn empty() -> Self {
        Self(Value::Null)
    }

    pub fn of(state: &impl Serialize) -> Option<Self> {
        serde_json::to_value(state)
            .inspect_err(|error| tracing::warn!("cannot save a window: {error}"))
            .ok()
            .map(Self)
    }

    pub fn read<T: DeserializeOwned>(&self) -> Option<T> {
        serde_json::from_value(self.0.clone()).ok()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SavedWindow {
    pub(crate) name: String,
    pub(crate) state: Option<Value>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SavedApp {
    session: Option<String>,
    windows: Vec<SavedWindow>,
}

fn new_name() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{now:x}-{:x}", std::process::id())
}

struct Writer {
    sender: Sender<String>,
    thread: JoinHandle<()>,
}

#[derive(Default)]
pub(crate) struct Resume {
    file: Option<PathBuf>,
    session: Option<String>,
    saved: Vec<SavedWindow>,
    names: Vec<(WindowId, String, Kind)>,
    written: Option<String>,
    writer: Option<Writer>,
    save_at: Option<Instant>,
    safe_at: Option<Instant>,
    answer: Rc<Cell<Option<bool>>>,
    pub(crate) opening: Option<String>,
    next_name: Option<String>,
    pub(crate) session_ending: bool,
}

impl Resume {
    pub(crate) fn load(app_id: &str) -> Self {
        let file = bande_xdg::state_home().map(|state| state.join(RESUME_FOLDER).join(app_id));
        let saved: SavedApp = file
            .as_ref()
            .and_then(|file| fs::read_to_string(file).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self {
            file,
            session: saved.session,
            saved: saved.windows,
            ..Self::default()
        }
    }

    pub(crate) fn join(&mut self, link: &mut AppConnection) {
        match link.join_session(self.session.as_deref()) {
            Ok(session) => self.session = session,
            Err(error) => tracing::warn!("no window session: {error}"),
        }
    }

    fn marker(&self) -> Option<PathBuf> {
        self.file
            .as_ref()
            .map(|file| file.with_extension("reopening"))
    }

    pub(crate) fn died_while_reopening(&self) -> bool {
        !self.saved.is_empty() && self.marker().is_some_and(|marker| marker.exists())
    }

    pub(crate) fn begin_reopening(&mut self) -> Vec<SavedWindow> {
        if self.saved.is_empty() {
            return Vec::new();
        }
        if let Some(marker) = self.marker()
            && let Err(error) = bande_xdg::replace_file(&marker, "")
        {
            tracing::warn!("cannot note the reopening: {error}");
        }
        self.safe_at = Some(Instant::now() + SAFE_AFTER);
        std::mem::take(&mut self.saved)
    }

    pub(crate) fn reopened_safely(&mut self, now: Instant) {
        if self.safe_at.is_some_and(|safe_at| safe_at <= now) {
            self.safe_at = None;
            self.remove_marker();
        }
    }

    fn remove_marker(&self) {
        if let Some(marker) = self.marker()
            && let Err(error) = fs::remove_file(&marker)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("cannot remove {}: {error}", marker.display());
        }
    }

    pub(crate) fn forget_saved(&mut self, link: &AppConnection) {
        for window in std::mem::take(&mut self.saved) {
            link.leave_session(&window.name);
        }
        self.remove_marker();
    }

    pub(crate) fn next_window(
        &mut self,
        link: &AppConnection,
        kind: Kind,
    ) -> Option<WindowInSession> {
        if matches!(kind, Kind::Dialog(_) | Kind::Panel(_)) {
            return None;
        }
        let restoring = self.opening.is_some();
        let name = self.opening.take().unwrap_or_else(new_name);
        self.next_name = Some(name.clone());
        link.window_in_session(name, restoring)
    }

    pub(crate) fn opened(&mut self, window: WindowId, kind: Kind) {
        if let Some(name) = self.next_name.take() {
            self.names.push((window, name, kind));
            self.touch();
        }
    }

    pub(crate) fn has_main_window(&self) -> bool {
        self.names.iter().any(|(_, _, kind)| *kind == Kind::Main)
    }

    pub(crate) fn kind(&self, window: WindowId) -> Option<Kind> {
        self.names
            .iter()
            .find(|(id, _, _)| *id == window)
            .map(|(_, _, kind)| *kind)
    }

    pub(crate) fn window_closed(&mut self, link: &AppConnection, window: WindowId) {
        if let Some(at) = self.names.iter().position(|(id, _, _)| *id == window) {
            let (_, name, _) = self.names.remove(at);
            link.leave_session(&name);
            self.touch();
        }
    }

    pub(crate) fn touch(&mut self) {
        if self.file.is_some() {
            self.save_at
                .get_or_insert_with(|| Instant::now() + SAVE_AFTER);
        }
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.save_at.into_iter().chain(self.safe_at).min()
    }

    pub(crate) fn save_due(&mut self, now: Instant) -> bool {
        let due = self.save_at.is_some_and(|save_at| save_at <= now);
        if due {
            self.save_at = None;
        }
        due
    }

    pub(crate) fn write(&mut self, windows: Vec<(WindowId, Option<Value>)>) {
        let saved = SavedApp {
            session: self.session.clone(),
            windows: windows
                .into_iter()
                .filter_map(|(window, state)| {
                    let (_, name, _) = self.names.iter().find(|(id, _, _)| *id == window)?;
                    Some(SavedWindow {
                        name: name.clone(),
                        state,
                    })
                })
                .chain(self.saved.iter().cloned())
                .collect(),
        };
        let text = match serde_json::to_string(&saved) {
            Ok(text) => text,
            Err(error) => return tracing::warn!("cannot save the windows: {error}"),
        };
        if self.written.as_ref() == Some(&text) {
            return;
        }
        self.written = Some(text.clone());
        if self.writer.is_none() {
            self.writer = self.file.clone().and_then(start_writer);
        }
        if let Some(writer) = &self.writer
            && writer.sender.send(text).is_err()
        {
            tracing::warn!("the window saver has stopped");
        }
    }

    pub(crate) fn finish_writing(&mut self) {
        self.remove_marker();
        if let Some(Writer { sender, thread }) = self.writer.take() {
            drop(sender);
            if thread.join().is_err() {
                tracing::warn!("the window saver failed");
            }
        }
    }

    pub(crate) fn discard(&mut self, link: &mut AppConnection) {
        self.finish_writing();
        link.end_session();
        if let Some(file) = &self.file
            && let Err(error) = fs::remove_file(file)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("cannot remove {}: {error}", file.display());
        }
    }

    pub(crate) fn take_answer(&self) -> Option<bool> {
        self.answer.take()
    }
}

fn start_writer(file: PathBuf) -> Option<Writer> {
    let (sender, receiver) = mpsc::channel::<String>();
    let thread = std::thread::Builder::new()
        .name("window-saver".to_owned())
        .spawn(move || {
            while let Ok(mut text) = receiver.recv() {
                while let Ok(newer) = receiver.try_recv() {
                    text = newer;
                }
                if let Err(error) = bande_xdg::replace_file(&file, &text) {
                    tracing::warn!("cannot save the windows: {error}");
                }
            }
        })
        .inspect_err(|error| tracing::warn!("cannot save the windows: {error}"))
        .ok()?;
    Some(Writer { sender, thread })
}

pub(crate) struct ReopenPrompt;

impl ReopenPrompt {
    pub(crate) fn open<P: Program>(cx: &mut Cx<P>) -> Result<(), WindowError> {
        let app = (cx.info.name)();
        let title = L10N.tr_args("reopen-question", [("app", app.into())]);
        let answer = Rc::clone(&cx.resume.answer);
        let alert = Alert::new(&title, &L10N.tr("reopen-detail"), move |action: &str| {
            answer.set(Some(action == REOPEN));
        })
        .primary(&L10N.tr("reopen-yes"), REOPEN)
        .cancel(&L10N.tr("reopen-no"), DO_NOT_REOPEN);
        let appearance = Appearance::default();
        let mut ui = Ui::new(
            Box::new(alert),
            Context::new(TextSystem::new(), appearance.palette()),
        );
        let fitted = ui.measure(Size::new(f32::INFINITY, f32::INFINITY));
        let size = (fitted.width.ceil() as u32, fitted.height.ceil() as u32);
        cx.open_content(Kind::Dialog("reopen"), &title, size, move |appearance| {
            ui.set_appearance(appearance);
            Content::Prompt(Box::new(ui))
        })
        .map(|_| ())
    }
}
