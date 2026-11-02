use std::cell::{Cell, RefCell};
use std::io::Write;
use std::process::ExitCode;
use std::rc::Rc;

use bande_file_chooser::L10N;
use bande_toolkit::Image;
use bande_toolkit::ui::{Alert, Axis, CheckState, Checkbox, Label, PopUpButton, Stack, Widget};
use bande_xdg::{
    ICON_THEMES, Locale, applications, current_desktops, data_dirs, find_app, find_icon,
};
use xdg_desktop_portal_bande::access::Question;
use xdg_desktop_portal_bande::dialog_process::{CANCELLED, CHOSEN, encode_pairs};
use xdg_desktop_portal_bande::file_chooser::Choice;

const GRANT: &str = "grant";
const SPACING: f32 = 8.0;
const ICON_POINTS: f64 = 36.0;
const UNAVAILABLE: u8 = 2;

type Answers = Rc<RefCell<Vec<(String, String)>>>;

fn initial(choice: &Choice) -> String {
    match choice.options.is_empty() {
        true if choice.initial == "true" => "true".to_owned(),
        true => "false".to_owned(),
        false => choice.initial.clone(),
    }
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

fn control(choice: &Choice, answers: &Answers) -> Box<dyn Widget> {
    let (answers, id) = (Rc::clone(answers), choice.id.clone());
    if choice.options.is_empty() {
        let state = match choice.initial.as_str() {
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
    let selected = keys
        .iter()
        .position(|key| *key == choice.initial)
        .unwrap_or(0);
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

fn icon(question: &Question, pixels: u32) -> Option<Image> {
    let dirs = data_dirs();
    let app_icon = || {
        let entries = applications(&dirs, Locale::from_env().as_ref(), &current_desktops());
        find_app(&entries, &question.app_id).and_then(|entry| entry.icon.clone())
    };
    let name = question.icon.clone().or_else(app_icon)?;
    find_icon(&name, pixels, &ICON_THEMES, &dirs).and_then(|path| Image::load(&path, pixels))
}

pub fn ask(question: &Question) -> ExitCode {
    let answers: Answers = Rc::new(RefCell::new(
        question
            .choices
            .iter()
            .map(|choice| (choice.id.clone(), initial(choice)))
            .collect(),
    ));
    let granted = Rc::new(Cell::new(false));
    let chosen = Rc::clone(&granted);
    let message = [question.subtitle.as_str(), question.body.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let asked = bande_toolkit::ask("access", |window| {
        let pixels = (ICON_POINTS * window.scale().max(2.0)).ceil() as u32;
        let deny = question
            .deny_label
            .clone()
            .unwrap_or_else(|| L10N.tr("button-deny"));
        let grant = question
            .grant_label
            .clone()
            .unwrap_or_else(|| L10N.tr("button-allow"));
        let mut alert = Alert::new(&question.title, &message, move |action| {
            chosen.set(action == GRANT)
        })
        .cancel(&deny, "deny")
        .primary(&grant, GRANT);
        if !question.choices.is_empty() {
            let controls = question
                .choices
                .iter()
                .map(|choice| control(choice, &answers))
                .collect();
            alert = alert.accessory(Stack::new(Axis::Vertical, SPACING, 0.0, controls));
        }
        match icon(question, pixels) {
            Some(icon) => alert.icon(icon),
            None => alert,
        }
    });
    match asked {
        Ok(()) if granted.get() => {
            let mut stdout = std::io::stdout();
            match stdout
                .write_all(&encode_pairs(&answers.borrow()))
                .and_then(|()| stdout.flush())
            {
                Ok(()) => ExitCode::from(CHOSEN),
                Err(error) => {
                    tracing::error!("cannot hand the answer to the portal: {error}");
                    ExitCode::from(UNAVAILABLE)
                }
            }
        }
        Ok(()) => ExitCode::from(CANCELLED),
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::from(UNAVAILABLE)
        }
    }
}
