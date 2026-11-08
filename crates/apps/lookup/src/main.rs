mod online;
mod panel;

use std::cell::Cell;
use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use bande_app::{Cx, Info, Keys, Kind, Program, WindowId};
use bande_inbox::Inbox;
use bande_l10n::Localizer;

use crate::panel::{LANGUAGE_ARGUMENT, PANEL_SIZE, Panel};

static L10N: LazyLock<Localizer> =
    LazyLock::new(|| Localizer::new(bande_l10n::translations!("bande-lookup")));

static INFO: Info = Info {
    id: "org.bande.Lookup",
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || L10N.tr("app-name"),
    keys: Keys::Standard,
    settings: None,
};

const NO_REQUEST_LIMIT: Duration = Duration::from_secs(30);

struct Lookup {
    shown: Option<WindowId>,
    dismissed: Rc<Cell<bool>>,
    give_up_at: Option<Instant>,
}

impl Lookup {
    fn show(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>) {
        let mut language = None;
        let mut words = Vec::new();
        for argument in arguments {
            match argument.strip_prefix(LANGUAGE_ARGUMENT) {
                Some(tag) => language = Some(tag.to_owned()),
                None => words.push(argument),
            }
        }
        let word = words.join(" ");
        if word.trim().is_empty() {
            return;
        }
        if let Some(window) = self.shown.take() {
            cx.close(window);
        }
        self.give_up_at = None;
        self.dismissed.set(false);
        let dismissed = self.dismissed.clone();
        let answers = match Inbox::new() {
            Ok(answers) => answers,
            Err(error) => {
                tracing::warn!("cannot look up: {error}");
                return;
            }
        };
        let title = word.clone();
        let opened = cx.open(
            Kind::Popover("lookup"),
            &title,
            PANEL_SIZE,
            move |appearance| Panel::new(appearance, word, language, answers, dismissed),
        );
        match opened {
            Ok(window) => self.shown = Some(window),
            Err(error) => tracing::warn!("cannot show Look Up: {error}"),
        }
    }
}

impl Program for Lookup {
    type View = Panel;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let mut lookup = Self {
            shown: None,
            dismissed: Rc::new(Cell::new(false)),
            give_up_at: Some(Instant::now() + NO_REQUEST_LIMIT),
        };
        lookup.show(cx, arguments);
        Ok(lookup)
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, _directory: &Path) {
        self.show(cx, arguments);
    }

    fn reopen(&mut self, _cx: &mut Cx<Self>) {}

    fn save_window(&self, _cx: &Cx<Self>, _window: WindowId) -> Option<bande_app::Saved> {
        None
    }

    fn deadline(&self) -> Option<Instant> {
        match self.dismissed.get() {
            true => Some(Instant::now()),
            false => self.give_up_at,
        }
    }

    fn tick(&mut self, cx: &mut Cx<Self>, now: Instant) {
        if self.dismissed.replace(false)
            && let Some(window) = self.shown.take()
        {
            cx.close(window);
        }
        if self.give_up_at.is_some_and(|limit| limit <= now) {
            cx.quit();
        }
    }

    fn waits_for_activation() -> bool {
        true
    }
}

fn main() -> ExitCode {
    bande_app::run::<Lookup>(&INFO)
}
