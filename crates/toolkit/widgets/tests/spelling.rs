#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use bande_graphics::{Palette, Scheme, TextSystem};
use bande_widgets::{
    Context, Event, Key, Modifiers, Size, SpellReply, SpellRequest, TextField, Ui, Widget,
    install_spelling, receive_spelling,
};

fn typing(field: TextField) -> (Ui, bande_widgets::WidgetId, Rc<RefCell<Vec<SpellRequest>>>) {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let recorder = requests.clone();
    install_spelling(move |request| recorder.borrow_mut().push(request));
    let id = field.id();
    let mut ui = Ui::new(
        Box::new(field),
        Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light)),
    );
    ui.layout(Size::new(400.0, 40.0));
    ui.set_focus(id);
    (ui, id, requests)
}

fn press(ui: &mut Ui, key: Key) {
    ui.handle(&Event::Key {
        key,
        modifiers: Modifiers::default(),
    });
}

fn type_text(ui: &mut Ui, text: &str) {
    for character in text.chars() {
        let key = if character == ' ' {
            Key::Space
        } else {
            Key::Character(character)
        };
        press(ui, key);
    }
}

#[test]
fn a_correction_is_applied_at_word_end_and_backspace_restores_the_original() {
    let (mut ui, id, requests) = typing(TextField::new("", "", |_| {}));
    type_text(&mut ui, "helo ");
    assert!(
        requests
            .borrow()
            .contains(&SpellRequest::Correct("helo".to_owned(), true))
    );
    receive_spelling(SpellReply::Corrected("helo".to_owned(), "hello".to_owned()));
    ui.spelling_answered();
    assert_eq!(ui.widget_ref::<TextField>(id).unwrap().text(), "hello ");
    press(&mut ui, Key::Backspace);
    assert_eq!(ui.widget_ref::<TextField>(id).unwrap().text(), "helo ");
}

#[test]
fn quote_style_comes_from_language_data() {
    let (mut ui, id, _) = typing(TextField::new("", "", |_| {}));
    receive_spelling(SpellReply::Rules(
        HashMap::from([
            ("smart-quotes-on".to_owned(), "true".to_owned()),
            ("quote-open".to_owned(), "«".to_owned()),
        ]),
        HashMap::new(),
    ));
    type_text(&mut ui, "\"");
    assert_eq!(ui.widget_ref::<TextField>(id).unwrap().text(), "«");
}

#[test]
fn a_field_that_opts_out_asks_nothing() {
    let (mut ui, _, requests) = typing(TextField::new("", "", |_| {}).without_text_checking());
    type_text(&mut ui, "helo ");
    let _ = ui.scene(400, 40);
    assert!(requests.borrow().is_empty());
}
