use std::cell::{Cell, RefCell};
use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use bande_bluetooth::PairingPrompt;
use bande_toolkit::ui::{Alert, Event, Key, TextField, Ui, Widget};
use bande_toolkit::{App, Appearance, Rect, Role, Scene, TextSystem, Window, WindowEvent};

use crate::L10N;
use crate::dialogs::{DIALOGS, alert_scene, fitted};

pub const PAIR_FLAG: &str = "--pair";
const ANSWER_POLL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Question {
    Confirm { device: String, code: String },
    Authorize { device: String },
    Show { device: String, code: String },
    Pin { device: String },
}

impl Question {
    pub fn arguments(&self) -> Vec<String> {
        let words: Vec<&str> = match self {
            Self::Confirm { device, code } => vec!["confirm", device, code],
            Self::Authorize { device } => vec!["authorize", device],
            Self::Show { device, code } => vec!["show", device, code],
            Self::Pin { device } => vec!["pin", device],
        };
        words.into_iter().map(str::to_owned).collect()
    }

    pub fn from_arguments(arguments: &[&str]) -> Option<Self> {
        let owned = |text: &&str| (*text).to_owned();
        Some(match arguments {
            ["confirm", device, code] => Self::Confirm {
                device: owned(device),
                code: owned(code),
            },
            ["authorize", device] => Self::Authorize {
                device: owned(device),
            },
            ["show", device, code] => Self::Show {
                device: owned(device),
                code: owned(code),
            },
            ["pin", device] => Self::Pin {
                device: owned(device),
            },
            _ => return None,
        })
    }

    fn text(&self) -> String {
        match self {
            Self::Confirm { device, code } => L10N.tr_args(
                "pair-confirm-code",
                [
                    ("device", device.clone().into()),
                    ("code", code.clone().into()),
                ],
            ),
            Self::Authorize { device } => {
                L10N.tr_args("pair-authorize", [("device", device.clone().into())])
            }
            Self::Show { device, code } => L10N.tr_args(
                "pair-type-code",
                [
                    ("device", device.clone().into()),
                    ("code", code.clone().into()),
                ],
            ),
            Self::Pin { device } => {
                L10N.tr_args("pair-enter-pin", [("device", device.clone().into())])
            }
        }
    }

    fn answer_button(&self) -> Option<String> {
        match self {
            Self::Confirm { .. } | Self::Pin { .. } => Some(L10N.tr("pair")),
            Self::Authorize { .. } => Some(L10N.tr("pair-allow")),
            Self::Show { .. } => None,
        }
    }

    fn decline_button(&self) -> String {
        match self {
            Self::Authorize { .. } => L10N.tr("pair-deny"),
            Self::Confirm { .. } | Self::Show { .. } | Self::Pin { .. } => L10N.tr("button-cancel"),
        }
    }
}

fn lock(shown: &Mutex<Option<Child>>) -> MutexGuard<'_, Option<Child>> {
    shown.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
pub struct PairingDialogs {
    shown: Mutex<Option<Child>>,
}

impl PairingDialogs {
    fn open(&self, question: &Question) -> Option<()> {
        self.dismiss();
        let program = std::env::current_exe()
            .inspect_err(|error| tracing::warn!("cannot find the shell to ask: {error}"))
            .ok()?;
        let child = Command::new(program)
            .arg(PAIR_FLAG)
            .args(question.arguments())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .inspect_err(|error| tracing::warn!("cannot ask about pairing: {error}"))
            .ok()?;
        *lock(&self.shown) = Some(child);
        Some(())
    }

    fn answer(&self, question: &Question) -> Option<(ExitStatus, String)> {
        self.open(question)?;
        loop {
            {
                let mut shown = lock(&self.shown);
                let child = shown.as_mut()?;
                if let Some(status) = child.try_wait().ok()? {
                    let mut output = String::new();
                    if let Some(mut stdout) = shown.take().and_then(|child| child.stdout) {
                        let _ = stdout.read_to_string(&mut output);
                    }
                    return Some((status, output));
                }
            }
            thread::sleep(ANSWER_POLL);
        }
    }

    fn accepted(&self, question: &Question) -> Option<String> {
        self.answer(question)
            .filter(|(status, _)| status.success())
            .map(|(_, output)| output)
    }
}

impl PairingPrompt for PairingDialogs {
    fn confirm_passkey(&self, device: &str, passkey: u32) -> bool {
        self.accepted(&Question::Confirm {
            device: device.to_owned(),
            code: format!("{passkey:06}"),
        })
        .is_some()
    }

    fn show_code(&self, device: &str, code: &str) {
        self.open(&Question::Show {
            device: device.to_owned(),
            code: code.to_owned(),
        });
    }

    fn ask_pin_code(&self, device: &str) -> Option<String> {
        self.accepted(&Question::Pin {
            device: device.to_owned(),
        })
        .map(|pin| pin.trim().to_owned())
        .filter(|pin| !pin.is_empty())
    }

    fn authorize(&self, device: &str) -> bool {
        self.accepted(&Question::Authorize {
            device: device.to_owned(),
        })
        .is_some()
    }

    fn dismiss(&self) {
        if let Some(mut child) = lock(&self.shown).take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Choice {
    Undecided,
    Decline,
    Accept,
}

const DECLINE: &str = "decline";
const ACCEPT: &str = "accept";

struct Dialog {
    ui: Ui,
    size: (u32, u32),
    choice: Rc<Cell<Choice>>,
    pin: Rc<RefCell<String>>,
    asks_pin: bool,
}

impl App for Dialog {
    fn scene(&mut self, _window: &mut Window) -> Scene {
        alert_scene(&mut self.ui, self.size)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match &event {
            WindowEvent::Input(Event::Key {
                key: Key::Escape, ..
            }) => self.choice.set(Choice::Decline),
            WindowEvent::Input(Event::Key {
                key: Key::Enter, ..
            }) => self.choice.set(Choice::Accept),
            WindowEvent::Input(input) => {
                if self.ui.handle(input) {
                    window.request_redraw();
                }
            }
            WindowEvent::Accessibility(request) => {
                if self.ui.perform(request) {
                    window.request_redraw();
                }
            }
            WindowEvent::AppearanceChanged(appearance) => {
                self.ui.set_appearance(*appearance);
                self.size = fitted(&mut self.ui);
                window.request_redraw();
            }
            _ => {}
        }
        match self.choice.get() {
            Choice::Undecided => {}
            Choice::Decline => window.close(),
            Choice::Accept => {
                if self.asks_pin {
                    println!("{}", self.pin.borrow());
                }
                window.close();
            }
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.ui.deadline()
    }

    fn tick(&mut self, window: &mut Window, _now: Instant) {
        window.request_redraw();
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.ui.accessibility())
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }
}

impl Dialog {
    fn new(question: &Question, appearance: Appearance) -> Self {
        let asks_pin = matches!(question, Question::Pin { .. });
        let choice = Rc::new(Cell::new(Choice::Undecided));
        let pin = Rc::new(RefCell::new(String::new()));
        let picked = Rc::clone(&choice);
        let mut alert = Alert::new(&question.text(), "", move |action| {
            picked.set(match action {
                ACCEPT => Choice::Accept,
                _ => Choice::Decline,
            })
        });
        let mut field = None;
        if asks_pin {
            let typed = Rc::clone(&pin);
            let input = TextField::new(&L10N.tr("pair-pin"), "", move |text| {
                *typed.borrow_mut() = text.to_owned();
            });
            field = Some(input.id());
            alert = alert.accessory(input);
        }
        if let Some(title) = question.answer_button() {
            alert = alert.primary(&title, ACCEPT);
        }
        alert = alert.cancel(&question.decline_button(), DECLINE);
        let mut ui = Ui::new(
            Box::new(alert),
            bande_toolkit::ui::Context::new(TextSystem::new(), appearance.palette()),
        );
        ui.set_appearance(appearance);
        if let Some(field) = field {
            ui.set_focus(field);
        }
        Self {
            size: fitted(&mut ui),
            ui,
            choice,
            pin,
            asks_pin,
        }
    }
}

pub fn run_pairing(arguments: &[&str]) -> Result<bool, Box<dyn std::error::Error>> {
    let question = Question::from_arguments(arguments)
        .ok_or_else(|| format!("unknown pairing question {arguments:?}"))?;
    let size = Dialog::new(&question, Appearance::default()).size;
    let mut window = Window::open(Role::dialog(&L10N.tr("pairing-request"), DIALOGS), size)?;
    let mut dialog = Dialog::new(&question, window.appearance());
    bande_toolkit::run(&mut window, &mut dialog)?;
    Ok(dialog.choice.get() == Choice::Accept)
}
