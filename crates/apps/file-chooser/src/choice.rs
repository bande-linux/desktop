use std::cell::RefCell;
use std::rc::Rc;

use bande_toolkit::ui::{Axis, CheckState, Checkbox, Label, PopUpButton, Stack, Widget};
use xdg_desktop_portal_bande::file_chooser::Choice;

const SPACING: f32 = 8.0;

pub type Answers = Rc<RefCell<Vec<(String, String)>>>;

pub fn answers(choices: &[Choice]) -> Answers {
    let initial = |choice: &Choice| match choice.options.is_empty() {
        true if choice.initial == "true" => "true".to_owned(),
        true => "false".to_owned(),
        false => choice.initial.clone(),
    };
    Rc::new(RefCell::new(
        choices
            .iter()
            .map(|choice| (choice.id.clone(), initial(choice)))
            .collect(),
    ))
}

fn set(answers: &Answers, id: &str, value: String) {
    if let Some(answer) = answers
        .borrow_mut()
        .iter_mut()
        .find(|answer| answer.0 == id)
    {
        answer.1 = value;
    }
}

pub fn control(choice: &Choice, answers: &Answers) -> Box<dyn Widget> {
    let current = answers
        .borrow()
        .iter()
        .find(|answer| answer.0 == choice.id)
        .map(|answer| answer.1.clone())
        .unwrap_or_default();
    let (answers, id) = (Rc::clone(answers), choice.id.clone());
    if choice.options.is_empty() {
        let state = match current.as_str() {
            "true" => CheckState::On,
            _ => CheckState::Off,
        };
        return Box::new(Checkbox::new(&choice.label, state, move |on| {
            set(&answers, &id, on.to_string());
        }));
    }
    let keys: Vec<String> = choice
        .options
        .iter()
        .map(|option| option.0.clone())
        .collect();
    let titles: Vec<&str> = choice
        .options
        .iter()
        .map(|option| option.1.as_str())
        .collect();
    let selected = keys.iter().position(|key| *key == current).unwrap_or(0);
    let pop_up = PopUpButton::new(&choice.label, &titles, selected, move |index| {
        if let Some(key) = keys.get(index) {
            set(&answers, &id, key.clone());
        }
    });
    Box::new(Stack::new(
        Axis::Horizontal,
        SPACING,
        0.0,
        vec![Box::new(Label::new(&choice.label)), Box::new(pop_up)],
    ))
}
