use bande_file_chooser::choice::{answers, control};
use bande_l10n::tr;
use bande_toolkit::Image;
use bande_toolkit::ui::{Alert, Axis, Stack};
use bande_xdg::{
    ICON_THEMES, Locale, applications, current_desktops, data_dirs, find_app, find_icon,
};
use std::cell::Cell;
use std::io::Write;
use std::process::ExitCode;
use std::rc::Rc;
use xdg_desktop_portal_bande::access::Question;
use xdg_desktop_portal_bande::dialog_process::{CANCELLED, CHOSEN, encode_pairs};

const GRANT: &str = "grant";
const SPACING: f32 = 8.0;
const ICON_POINTS: f64 = 36.0;
const UNAVAILABLE: u8 = 2;

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
    let answers = answers(&question.choices);
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
            .unwrap_or_else(|| tr!("Don’t Allow"));
        let grant = question.grant_label.clone().unwrap_or_else(|| tr!("Allow"));
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
