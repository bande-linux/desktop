use std::cell::Cell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use unbound_polkit::agent::{Identity, Notice};
use unbound_polkit::dialog::{Dialog, Message, Prompt};
use unbound_toolkit::{Appearance, TextSystem};

const SIZE: (u32, u32) = (560, 340);
const SETTLED: Duration = Duration::from_secs(3);
const MESSAGE: &str = "Authentication is required to set the system timezone.";
const DATA: [&str; 3] = [MESSAGE, "Anna Lindqvist", "Omar Haddad"];

fn prompt() -> Prompt {
    Prompt {
        message: MESSAGE.to_owned(),
        identities: [("anna", "Anna Lindqvist"), ("omar", "Omar Haddad")]
            .map(|(login, name)| Identity {
                login: login.to_owned(),
                name: name.to_owned(),
            })
            .to_vec(),
        user: "anna".to_owned(),
    }
}

#[test]
fn the_password_dialog_fits_longer_translations_and_has_no_hard_coded_text() {
    unbound_golden::use_pseudo_locale();
    let (reply, _answer) = mpsc::channel();
    let mut dialog = Dialog::new(prompt(), reply, TextSystem::new(), Appearance::default());
    let clock = Rc::new(Cell::new(Instant::now()));
    let now = Rc::clone(&clock);
    dialog.ui_mut().set_clock(move || now.get());
    dialog.receive(Message::Notice(Notice::WrongPassword));
    dialog.scene(SIZE.0, SIZE.1);
    clock.set(clock.get() + SETTLED);
    let mut scene = dialog.scene(SIZE.0, SIZE.1);
    scene.background = Appearance::default().palette().chassis;
    let problems = unbound_golden::pseudo_problems!("polkit", &scene, &DATA);
    assert!(problems.is_empty(), "{problems:#?}");
}
