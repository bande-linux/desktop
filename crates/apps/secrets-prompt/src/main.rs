use std::cell::RefCell;
use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Instant;

use accesskit::TreeUpdate;
use bande_l10n::{tr, trn};
use bande_toolkit::ui::{Alert, Context, FreeLayout, SecureField, Ui, Widget, WidgetId};
use bande_toolkit::{
    App, Color, Edges, KeyboardFocus, LayerKind, LayerOptions, Role, Scene, TextSystem, Window,
    WindowError, WindowEvent, run,
};
use bande_xdg::{Locale, app_for_identity, current_desktops, data_dirs, handlers};
use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
use rustix::process::{DumpableBehavior, set_dumpable_behavior};
use zeroize::Zeroizing;

const NAMESPACE: &str = "bande-secrets-prompt";
const FIELD_FONT: f32 = 13.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Unlock,
    Allow,
    Create,
    UpdateLogin,
    Network,
    Reveal,
}

struct Request {
    mode: Mode,
    keychain: String,
    app: String,
    items: Vec<String>,
}

fn app_name(identity: Option<&str>) -> String {
    let Some(identity) = identity else {
        return tr!("An app");
    };
    let entries = handlers(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    );
    match app_for_identity(&entries, identity) {
        Some(entry) => entry.name.clone(),
        None => identity
            .strip_prefix("flatpak:")
            .or_else(|| Path::new(identity).file_name()?.to_str())
            .unwrap_or(identity)
            .to_owned(),
    }
}

fn request() -> Option<Request> {
    let mode = match std::env::args().nth(1)?.as_str() {
        "--unlock" => Mode::Unlock,
        "--allow" => Mode::Allow,
        "--create" => Mode::Create,
        "--update-login" => Mode::UpdateLogin,
        "--network" => Mode::Network,
        "--reveal" => Mode::Reveal,
        _ => return None,
    };
    let mut keychain = String::new();
    let mut app = None;
    let mut items = Vec::new();
    for line in std::io::stdin().lines() {
        let line = line.ok()?;
        if line.is_empty() {
            break;
        }
        match line.split_once('\t')? {
            ("keychain", value) => value.clone_into(&mut keychain),
            ("app", value) => app = Some(value.to_owned()),
            ("item", value) => items.push(value.to_owned()),
            _ => {}
        }
    }
    Some(Request {
        mode,
        keychain,
        app: app_name(app.as_deref()),
        items,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Asking,
    Waiting,
    Done,
}

struct Prompt {
    ui: Ui,
    request: Request,
    choice: Rc<RefCell<Option<String>>>,
    field: WidgetId,
    first: Option<Zeroizing<String>>,
    stage: Stage,
    input: Vec<u8>,
    stdin: std::io::Stdin,
}

fn reply(line: &str) -> bool {
    let mut output = std::io::stdout().lock();
    output
        .write_all(line.as_bytes())
        .and_then(|()| output.flush())
        .is_ok()
}

impl Prompt {
    fn texts(&self, note: Option<String>) -> (String, String) {
        let request = &self.request;
        let app = || request.app.clone();
        let keychain = || request.keychain.clone();
        let (title, message) = match (request.mode, self.first.is_some()) {
            (Mode::Unlock, _) => (
                tr!(
                    "“{app}” wants to use the “{keychain}” keychain.",
                    app = app(),
                    keychain = keychain()
                ),
                tr!("Enter the keychain password."),
            ),
            (Mode::Allow, _) => (
                trn!(
                    "“{app}” wants to use your confidential information stored in “{item}” in your keychain.",
                    "“{app}” wants to use {count} passwords stored in your keychain.",
                    count = request.items.len(),
                    app = app(),
                    item = request.items.first().cloned().unwrap_or_default(),
                ),
                tr!(
                    "To allow this, enter the “{keychain}” keychain password.",
                    keychain = keychain()
                ),
            ),
            (Mode::Create, false) => (
                tr!(
                    "“{app}” wants to create a new keychain, “{keychain}”.",
                    app = app(),
                    keychain = keychain()
                ),
                tr!("Choose a password for the new keychain."),
            ),
            (Mode::Create, true) => (
                tr!(
                    "“{app}” wants to create a new keychain, “{keychain}”.",
                    app = app(),
                    keychain = keychain()
                ),
                tr!("Enter the password again to verify it."),
            ),
            (Mode::UpdateLogin, _) => (
                tr!("Your login keychain was not unlocked with your login password."),
                tr!(
                    "Enter the keychain’s old password to update it to your login password, or create a new keychain. The old keychain is kept."
                ),
            ),
            (Mode::Reveal, _) => (
                tr!("“{app}” wants to show your saved passwords.", app = app()),
                tr!(
                    "To allow this, enter the “{keychain}” keychain password.",
                    keychain = keychain()
                ),
            ),
            (Mode::Network, _) => (
                tr!(
                    "The network “{network}” requires a password.",
                    network = keychain()
                ),
                tr!("Enter the password to join it."),
            ),
        };
        match note {
            Some(note) => (title, format!("{note} {message}")),
            None => (title, message),
        }
    }

    fn present(&mut self, note: Option<String>) {
        let (title, message) = self.texts(note);
        let choice = Rc::clone(&self.choice);
        let submitted = Rc::clone(&self.choice);
        let placeholder = match self.first {
            Some(_) => tr!("Verify"),
            None => tr!("Password"),
        };
        let field = SecureField::new(&placeholder, FIELD_FONT, move |_| {
            submitted
                .borrow_mut()
                .get_or_insert_with(|| "allow".to_owned());
        });
        self.field = field.id();
        let alert = Alert::new(&title, &message, move |action| {
            choice.borrow_mut().get_or_insert_with(|| action.to_owned());
        })
        .accessory(field);
        let alert = match self.request.mode {
            Mode::Unlock => alert
                .cancel(&tr!("Cancel"), "cancel")
                .primary(&tr!("OK"), "allow"),
            Mode::Allow => alert
                .other(&tr!("Always Allow"), "always")
                .cancel(&tr!("Deny"), "cancel")
                .primary(&tr!("Allow"), "allow"),
            Mode::Create => alert
                .cancel(&tr!("Cancel"), "cancel")
                .primary(&tr!("Create"), "allow"),
            Mode::Reveal => alert
                .cancel(&tr!("Cancel"), "cancel")
                .primary(&tr!("Allow"), "allow"),
            Mode::Network => alert
                .cancel(&tr!("Cancel"), "cancel")
                .primary(&tr!("Join"), "allow"),
            Mode::UpdateLogin => alert
                .other(&tr!("Create New Keychain"), "new")
                .cancel(&tr!("Not Now"), "cancel")
                .primary(&tr!("Update Password"), "allow"),
        };
        self.ui.present_dialog(alert);
        self.stage = Stage::Asking;
    }

    fn secret(&self) -> Zeroizing<String> {
        Zeroizing::new(
            self.ui
                .widget_ref::<SecureField>(self.field)
                .map(|field| field.secret().to_owned())
                .unwrap_or_default(),
        )
    }

    fn settle(&mut self) -> bool {
        if self.stage != Stage::Asking {
            return false;
        }
        let Some(action) = self.choice.borrow_mut().take() else {
            return false;
        };
        let secret = self.secret();
        match action.as_str() {
            "allow" | "always" if secret.is_empty() => {
                self.present(None);
            }
            "allow" if self.request.mode == Mode::Create && self.first.is_none() => {
                self.first = Some(secret);
                self.present(None);
            }
            "allow"
                if self.request.mode == Mode::Create && self.first.as_ref() != Some(&secret) =>
            {
                self.first = None;
                self.present(Some(tr!("The passwords don’t match. Try again.")));
            }
            "allow" | "always" => {
                let line = Zeroizing::new(format!("{action}\t{}\n", secret.as_str()));
                self.stage = match reply(&line) {
                    true => Stage::Waiting,
                    false => Stage::Done,
                };
            }
            "new" => {
                self.stage = match reply("new\n") {
                    true => Stage::Waiting,
                    false => Stage::Done,
                };
            }
            _ => {
                reply("cancel\n");
                self.stage = Stage::Done;
            }
        }
        true
    }

    fn answer(&mut self, line: &str) {
        match line {
            "wrong" if self.stage == Stage::Waiting => {
                self.first = None;
                self.present(Some(tr!("The password you entered is incorrect.")));
            }
            _ => self.stage = Stage::Done,
        }
    }
}

impl App for Prompt {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let mut scene = self.ui.scene(width, height);
        scene.background = Color::TRANSPARENT;
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let changed = match event {
            WindowEvent::Input(input) => self.ui.handle(&input),
            WindowEvent::Accessibility(request) => self.ui.perform(&request),
            WindowEvent::AppearanceChanged(appearance) => {
                self.ui.set_appearance(appearance);
                true
            }
            _ => false,
        };
        if self.settle() || changed {
            window.request_redraw();
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.stdin.as_fd()]
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) {
        let mut buffer = [0u8; 256];
        match self.stdin.lock().read(&mut buffer) {
            Ok(0) => self.stage = Stage::Done,
            Ok(read) => self.input.extend_from_slice(&buffer[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => self.stage = Stage::Done,
        }
        while let Some(end) = self.input.iter().position(|byte| *byte == b'\n') {
            let line = String::from_utf8_lossy(&self.input[..end]).into_owned();
            self.input.drain(..=end);
            self.answer(&line);
        }
        window.request_redraw();
    }

    fn deadline(&self) -> Option<Instant> {
        self.ui.deadline()
    }

    fn tick(&mut self, window: &mut Window, _now: Instant) {
        window.request_redraw();
    }

    fn finished(&self) -> bool {
        self.stage == Stage::Done
    }

    fn should_close(&mut self, _window: &mut Window) -> bool {
        if self.stage != Stage::Done {
            reply("cancel\n");
            self.stage = Stage::Done;
        }
        true
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }
}

fn show(request: Request) -> Result<(), WindowError> {
    let mut window = Window::open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Overlay,
            anchor: Edges::ALL,
            exclusive_zone: -1,
            keyboard: KeyboardFocus::Exclusive,
            namespace: NAMESPACE.to_owned(),
        }),
        (0, 0),
    )?;
    let appearance = window.appearance();
    let mut ui = Ui::new(
        Box::new(FreeLayout::new()),
        Context::new(TextSystem::new(), appearance.palette()),
    );
    ui.set_appearance(appearance);
    let mut prompt = Prompt {
        ui,
        request,
        choice: Rc::default(),
        field: WidgetId::next(),
        first: None,
        stage: Stage::Asking,
        input: Vec::new(),
        stdin: std::io::stdin(),
    };
    prompt.present(None);
    run(&mut window, &mut prompt)
}

fn main() -> ExitCode {
    bande_log::init(NAMESPACE);
    if set_dumpable_behavior(DumpableBehavior::NotDumpable).is_err() {
        return ExitCode::FAILURE;
    }
    let Some(request) = request() else {
        tracing::error!(
            "usage: {NAMESPACE} --unlock|--allow|--create|--update-login|--network|--reveal, with the request on standard input"
        );
        return ExitCode::FAILURE;
    };
    let stdin = std::io::stdin();
    let nonblocking = fcntl_getfl(stdin.as_fd())
        .and_then(|flags| fcntl_setfl(stdin.as_fd(), flags | OFlags::NONBLOCK));
    if nonblocking.is_err() {
        return ExitCode::FAILURE;
    }
    match show(request) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
