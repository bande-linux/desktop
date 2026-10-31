#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::Cell;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Role, TreeId};
use unbound_polkit::agent::Notice;
use unbound_polkit::dialog::{Dialog, Message, Password, Prompt};
use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{Appearance, CpuRenderer, TextSystem};

const SIZE: (u32, u32) = (560, 340);
const SETTLED: Duration = Duration::from_secs(3);

struct Asking {
    dialog: Dialog,
    clock: Rc<Cell<Instant>>,
}

impl Asking {
    fn new() -> (Self, Receiver<Option<Password>>) {
        let (reply, answer) = mpsc::channel();
        let mut dialog = Dialog::new(prompt(), reply, TextSystem::new(), Appearance::default());
        let clock = Rc::new(Cell::new(Instant::now()));
        let now = Rc::clone(&clock);
        dialog.ui_mut().set_clock(move || now.get());
        (Self { dialog, clock }, answer)
    }

    fn settle(&mut self) -> Vec<u8> {
        self.clock.set(self.clock.get() + SETTLED);
        self.dialog.scene(SIZE.0, SIZE.1);
        self.clock.set(self.clock.get() + SETTLED);
        let mut scene = self.dialog.scene(SIZE.0, SIZE.1);
        scene.background = Appearance::default().palette().chassis;
        CpuRenderer::new().render(&scene).expect("frame").rgba
    }

    fn press(&mut self, key: Key) {
        self.dialog.handle(&Event::Key {
            key,
            modifiers: Modifiers::default(),
        });
    }

    fn type_text(&mut self, text: &str) {
        for character in text.chars() {
            self.press(Key::Character(character));
        }
    }
}

fn prompt() -> Prompt {
    Prompt {
        message: "Authentication is required to set the system timezone.".to_owned(),
        user: "anna".to_owned(),
    }
}

fn check_golden(name: &str, rgba: Vec<u8>) {
    unbound_golden::assert_golden!(name, &unbound_golden::from_rgba(SIZE.0, SIZE.1, rgba));
}

fn answered(answer: &Receiver<Option<Password>>) -> Option<String> {
    answer
        .try_recv()
        .expect("an answer")
        .map(|password| password.as_str().to_owned())
}

#[test]
fn the_typed_password_is_sent_with_enter() {
    let (mut asking, answer) = Asking::new();
    check_golden("asking", asking.settle());
    asking.type_text("hunter2");
    asking.press(Key::Enter);
    assert_eq!(answered(&answer), Some("hunter2".to_owned()));
    asking.press(Key::Enter);
    assert!(answer.try_recv().is_err());
}

#[test]
fn the_authenticate_button_sends_the_typed_password() {
    let (mut asking, answer) = Asking::new();
    asking.settle();
    asking.type_text("hunter2");
    let button = asking
        .dialog
        .ui()
        .accessibility()
        .nodes
        .into_iter()
        .find(|(_, node)| node.role() == Role::Button && node.label() == Some("Authenticate"))
        .expect("an Authenticate button")
        .0;
    asking.dialog.perform(&ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: button,
        data: None,
    });
    assert_eq!(answered(&answer), Some("hunter2".to_owned()));
}

#[test]
fn a_wrong_password_shakes_clears_the_field_and_asks_again() {
    let (mut asking, first) = Asking::new();
    asking.settle();
    asking.type_text("wrong");
    asking.press(Key::Enter);
    assert_eq!(answered(&first), Some("wrong".to_owned()));
    let (reply, second) = mpsc::channel();
    asking
        .dialog
        .receive(Message::Notice(Notice::WrongPassword));
    asking.dialog.receive(Message::Prompt {
        prompt: prompt(),
        reply,
    });
    assert!(asking.dialog.ui().is_animating() || asking.dialog.ui().deadline().is_some());
    check_golden("wrong-password", asking.settle());
    asking.type_text("right");
    asking.press(Key::Enter);
    assert_eq!(answered(&second), Some("right".to_owned()));
}

#[test]
fn escape_cancels_and_closes_the_dialog() {
    let (mut asking, answer) = Asking::new();
    asking.settle();
    asking.type_text("half");
    asking.press(Key::Escape);
    assert_eq!(answered(&answer), None);
    asking.settle();
    assert!(asking.dialog.finished());
    assert!(asking.dialog.declined());
}

#[test]
fn polkit_closing_the_request_dismisses_the_dialog() {
    let (mut asking, _answer) = Asking::new();
    asking.settle();
    asking.dialog.receive(Message::Close);
    asking.settle();
    assert!(asking.dialog.finished());
    assert!(!asking.dialog.declined());
}
