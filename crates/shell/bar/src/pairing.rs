use std::cell::{Cell, RefCell};
use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use bande_bluetooth::PairingPrompt;
use bande_toolkit::ui::{Axis, Button, Event, Key, Label, Stack, TextField, Ui, Widget};
use bande_toolkit::{App, Palette, Rect, Role, Scene, TextSystem, Window, WindowEvent};

use crate::L10N;
use crate::dialogs::DIALOGS;

pub const PAIR_FLAG: &str = "--pair";
const ANSWER_POLL: Duration = Duration::from_millis(100);
const DIALOG_SIZE: (u32, u32) = (400, 100);
const PIN_DIALOG_SIZE: (u32, u32) = (400, 140);

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

    fn size(&self) -> (u32, u32) {
        if matches!(self, Self::Pin { .. }) {
            PIN_DIALOG_SIZE
        } else {
            DIALOG_SIZE
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

struct Dialog {
    ui: Ui,
    choice: Rc<Cell<Choice>>,
    pin: Rc<RefCell<String>>,
    asks_pin: bool,
}

impl App for Dialog {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.ui.scene(width, height)
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
    fn new(question: &Question, palette: Palette) -> Self {
        let asks_pin = matches!(question, Question::Pin { .. });
        let choice = Rc::new(Cell::new(Choice::Undecided));
        let pin = Rc::new(RefCell::new(String::new()));
        let button = |title: &str, picked: Choice| {
            let choice = Rc::clone(&choice);
            Button::new(title, move || choice.set(picked))
        };
        let mut rows: Vec<Box<dyn Widget>> = vec![Box::new(Label::new(&question.text()))];
        let mut field = None;
        if asks_pin {
            let typed = Rc::clone(&pin);
            let input = TextField::new(&L10N.tr("pair-pin"), "", move |text| {
                *typed.borrow_mut() = text.to_owned();
            });
            field = Some(input.id());
            rows.push(Box::new(input));
        }
        let mut buttons: Vec<Box<dyn Widget>> = vec![Box::new(button(
            &question.decline_button(),
            Choice::Decline,
        ))];
        if let Some(title) = question.answer_button() {
            buttons.push(Box::new(button(&title, Choice::Accept).primary()));
        }
        rows.push(Box::new(Stack::new(Axis::Horizontal, 12.0, 0.0, buttons)));
        let mut ui = Ui::new(
            Box::new(Stack::new(Axis::Vertical, 10.0, 20.0, rows)),
            bande_toolkit::ui::Context::new(TextSystem::new(), palette),
        );
        if let Some(field) = field {
            ui.set_focus(field);
        }
        Self {
            ui,
            choice,
            pin,
            asks_pin,
        }
    }
}

pub fn pairing_scene(question: &Question, palette: Palette) -> Scene {
    let (width, height) = question.size();
    Dialog::new(question, palette).ui.scene(width, height)
}

pub fn run_pairing(arguments: &[&str]) -> Result<bool, Box<dyn std::error::Error>> {
    let question = Question::from_arguments(arguments)
        .ok_or_else(|| format!("unknown pairing question {arguments:?}"))?;
    let mut window = Window::open(
        Role::toplevel(&L10N.tr("pairing-request"), DIALOGS),
        question.size(),
    )?;
    let mut dialog = Dialog::new(&question, window.appearance().palette());
    bande_toolkit::run(&mut window, &mut dialog)?;
    Ok(dialog.choice.get() == Choice::Accept)
}

#[cfg(test)]
mod tests {
    use super::Question;

    #[test]
    fn every_question_crosses_to_the_dialog_process_unchanged() {
        let device = "Jo’s Phone".to_owned();
        let code = "004207".to_owned();
        for question in [
            Question::Confirm {
                device: device.clone(),
                code: code.clone(),
            },
            Question::Authorize {
                device: device.clone(),
            },
            Question::Show {
                device: device.clone(),
                code,
            },
            Question::Pin { device },
        ] {
            let arguments = question.arguments();
            let words: Vec<&str> = arguments.iter().map(String::as_str).collect();
            assert_eq!(Question::from_arguments(&words), Some(question));
        }
    }
}
