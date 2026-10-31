#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::Cell;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Role, TreeId};
use bande_polkit::agent::{Answer, Identity, Notice};
use bande_polkit::dialog::{Dialog, Message, Prompt};
use bande_toolkit::ui::{Event, Key, Modifiers};
use bande_toolkit::{Appearance, CpuRenderer, TextSystem};

const SIZE: (u32, u32) = (560, 340);
const SETTLED: Duration = Duration::from_secs(3);

struct Asking {
    dialog: Dialog,
    clock: Rc<Cell<Instant>>,
}

impl Asking {
    fn new() -> (Self, Receiver<Answer>) {
        Self::with(prompt())
    }

    fn with(prompt: Prompt) -> (Self, Receiver<Answer>) {
        let (reply, answer) = mpsc::channel();
        let mut dialog = Dialog::new(prompt, reply, TextSystem::new(), Appearance::default());
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

fn identity(login: &str, name: &str) -> Identity {
    Identity {
        login: login.to_owned(),
        name: name.to_owned(),
    }
}

fn prompt() -> Prompt {
    Prompt {
        message: "Authentication is required to set the system timezone.".to_owned(),
        identities: vec![identity("anna", "Anna Lindqvist")],
        user: "anna".to_owned(),
    }
}

fn two_admins() -> Prompt {
    Prompt {
        identities: vec![
            identity("anna", "Anna Lindqvist"),
            identity("omar", "Omar Haddad"),
        ],
        ..prompt()
    }
}

fn answered(answer: &Receiver<Answer>) -> Option<String> {
    match answer.try_recv().expect("an answer") {
        Answer::Password(password) => Some(password.as_str().to_owned()),
        Answer::SwitchTo(login) => Some(format!("switch to {login}")),
        Answer::Cancel => None,
    }
}

fn description(asking: &Asking) -> String {
    asking
        .dialog
        .ui()
        .accessibility()
        .nodes
        .into_iter()
        .find(|(_, node)| node.role() == Role::AlertDialog)
        .and_then(|(_, node)| node.description().map(str::to_owned))
        .expect("the dialog's description")
}

#[test]
fn the_typed_password_is_sent_with_enter() {
    let (mut asking, answer) = Asking::new();
    bande_golden::assert_golden_rgba!("asking", SIZE, asking.settle());
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
    bande_golden::assert_golden_rgba!("wrong-password", SIZE, asking.settle());
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

#[test]
fn the_dialog_names_the_user_by_full_name() {
    let (asking, _answer) = Asking::new();
    assert_eq!(
        description(&asking),
        "Enter the password for Anna Lindqvist to allow this."
    );
}

#[test]
fn choosing_another_administrator_restarts_for_that_user() {
    let (mut asking, first) = Asking::with(two_admins());
    bande_golden::assert_golden_rgba!("choose-user", SIZE, asking.settle());
    let picker = asking
        .dialog
        .ui()
        .accessibility()
        .nodes
        .into_iter()
        .find(|(_, node)| node.role() == Role::ComboBox)
        .expect("a user picker");
    assert_eq!(picker.1.value(), Some("Anna Lindqvist"));
    asking.type_text("half");
    asking.dialog.perform(&ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: picker.0,
        data: None,
    });
    assert!(asking.dialog.ui_mut().take_menu_request().is_some());
    asking.dialog.ui_mut().menu_action("1");
    asking.dialog.settle_choices();
    assert_eq!(answered(&first), Some("switch to omar".to_owned()));
    assert_eq!(
        description(&asking),
        "Enter the password for Omar Haddad to allow this."
    );

    let (reply, second) = mpsc::channel();
    asking.dialog.receive(Message::Prompt {
        prompt: Prompt {
            user: "omar".to_owned(),
            ..two_admins()
        },
        reply,
    });
    asking.type_text("right");
    asking.press(Key::Enter);
    assert_eq!(answered(&second), Some("right".to_owned()));
}

#[test]
fn a_single_administrator_gets_no_picker() {
    let (asking, _answer) = Asking::new();
    assert!(
        !asking
            .dialog
            .ui()
            .accessibility()
            .nodes
            .into_iter()
            .any(|(_, node)| node.role() == Role::ComboBox)
    );
}
