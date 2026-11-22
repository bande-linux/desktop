use std::cell::{Cell, RefCell};
use std::io::Write;
use std::process::ExitCode;
use std::rc::Rc;

use bande_keyboard::Trigger;
use bande_l10n::tr;
use bande_toolkit::ui::{Alert, Axis, CheckState, Checkbox, ShortcutField, Stack, Widget};
use bande_xdg::{Locale, app_name, applications, current_desktops, data_dirs};
use xdg_desktop_portal_bande::dialog_process::{CANCELLED, CHOSEN, encode_decisions};
use xdg_desktop_portal_bande::global_shortcuts::{Consent, Proposal};

const ALLOW: &str = "allow";
const SPACING: f32 = 8.0;
const UNAVAILABLE: u8 = 2;

struct Answer {
    id: String,
    on: bool,
    trigger: Option<Trigger>,
}

type Answers = Rc<RefCell<Vec<Answer>>>;

fn row(
    index: usize,
    proposal: &Proposal,
    initially_on: bool,
    answers: &Answers,
    taken: &Rc<[Trigger]>,
) -> Box<dyn Widget> {
    let title = match proposal.description.is_empty() {
        true => proposal.id.clone(),
        false => proposal.description.clone(),
    };
    let switching = Rc::clone(answers);
    let checkbox = Checkbox::new(
        &title,
        match initially_on {
            true => CheckState::On,
            false => CheckState::Off,
        },
        move |on| {
            if let Some(answer) = switching.borrow_mut().get_mut(index) {
                answer.on = on;
            }
        },
    );
    let (recording, taken) = (Rc::clone(answers), Rc::clone(taken));
    let keys = proposal.trigger.map(Trigger::keys);
    let field = ShortcutField::new(&title, keys.as_deref(), move |chord| {
        let mut answers = recording.borrow_mut();
        let Some(trigger) = Trigger::parse(chord).filter(|trigger| {
            trigger.assignable()
                && !taken.contains(trigger)
                && !answers
                    .iter()
                    .enumerate()
                    .any(|(other, answer)| other != index && answer.trigger == Some(*trigger))
        }) else {
            return false;
        };
        match answers.get_mut(index) {
            Some(answer) => {
                answer.trigger = Some(trigger);
                true
            }
            None => false,
        }
    });
    Box::new(Stack::new(
        Axis::Horizontal,
        SPACING,
        0.0,
        vec![Box::new(checkbox), Box::new(field)],
    ))
}

pub fn ask(consent: &Consent) -> ExitCode {
    let entries = applications(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    );
    let app = app_name(&entries, &consent.app_id);
    let initially_on = |proposal: &Proposal| !consent.configuring || proposal.trigger.is_some();
    let answers: Answers = Rc::new(RefCell::new(
        consent
            .shortcuts
            .iter()
            .map(|proposal| Answer {
                id: proposal.id.clone(),
                on: initially_on(proposal),
                trigger: proposal.trigger,
            })
            .collect(),
    ));
    let taken: Rc<[Trigger]> = consent.taken.clone().into();
    let allowed = Rc::new(Cell::new(false));
    let chosen = Rc::clone(&allowed);
    let asked = bande_toolkit::ask("global-shortcuts", |_window| {
        let (title, message, cancel, accept) = match consent.configuring {
            true => (
                tr!("Shortcuts for {app}", app = app),
                tr!(
                    "Choose which shortcuts {app} responds to, and their keys. A shortcut holds Super, or both Ctrl and Alt.",
                    app = app
                ),
                tr!("Cancel"),
                tr!("Done"),
            ),
            false => (
                tr!("Allow {app} to use keyboard shortcuts?", app = app),
                tr!(
                    "{app} wants to respond to these shortcuts while you work in other apps. A shortcut holds Super, or both Ctrl and Alt.",
                    app = app
                ),
                tr!("Don’t Allow"),
                tr!("Allow"),
            ),
        };
        let rows = consent
            .shortcuts
            .iter()
            .enumerate()
            .map(|(index, proposal)| row(index, proposal, initially_on(proposal), &answers, &taken))
            .collect();
        Alert::new(&title, &message, move |action| chosen.set(action == ALLOW))
            .accessory(Stack::new(Axis::Vertical, SPACING, 0.0, rows))
            .cancel(&cancel, "cancel")
            .primary(&accept, ALLOW)
    });
    match asked {
        Ok(()) if allowed.get() => {
            let decisions: Vec<(String, Option<Trigger>)> = answers
                .borrow()
                .iter()
                .map(|answer| (answer.id.clone(), answer.trigger.filter(|_| answer.on)))
                .collect();
            let mut stdout = std::io::stdout();
            match stdout
                .write_all(&encode_decisions(&decisions))
                .and_then(|()| stdout.flush())
            {
                Ok(()) => ExitCode::from(CHOSEN),
                Err(error) => {
                    tracing::error!("cannot hand the shortcuts to the portal: {error}");
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
