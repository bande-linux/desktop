use bande_l10n::{text, tr};
use bande_toolkit::Image;
use bande_toolkit::ui::{Alert, Axis, Label, Stack, TextField, Widget};
use bande_xdg::{Locale, app_name, applications, current_desktops, data_dirs};
use std::cell::{Cell, RefCell};
use std::io::{Read, Write};
use std::process::ExitCode;
use std::rc::Rc;
use xdg_desktop_portal_bande::dialog_process::{CANCELLED, CHOSEN};
use xdg_desktop_portal_bande::dynamic_launcher::Launcher;

const ADD: &str = "add";
const SPACING: f32 = 8.0;
const ICON_POINTS: f64 = 36.0;
const UNAVAILABLE: u8 = 2;

fn icon(bytes: &[u8], pixels: u32) -> Option<Image> {
    match bytes.windows(4).take(1024).any(|window| window == b"<svg") {
        true => Image::from_svg(bytes, pixels, pixels),
        false => Image::decode(bytes, pixels),
    }
}

pub fn confirm(launcher: &Launcher) -> ExitCode {
    let mut bytes = Vec::new();
    if let Err(error) = std::io::stdin().read_to_end(&mut bytes) {
        tracing::warn!("cannot read the launcher icon: {error}");
    }
    let entries = applications(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    );
    let asking = app_name(&entries, &launcher.app_id);
    let name = Rc::new(RefCell::new(launcher.name.clone()));
    let added = Rc::new(Cell::new(false));
    let chosen = Rc::clone(&added);
    let asked = bande_toolkit::ask("create-launcher", |window| {
        let pixels = (ICON_POINTS * window.scale().max(2.0)).ceil() as u32;
        let (title, message) = match launcher.webapp {
            true => (
                text!("Add this web app to Applications?"),
                text!("{app} wants to add a web app you can open from the Launcher."),
            ),
            false => (
                text!("Add this app to Applications?"),
                text!("{app} wants to add an app you can open from the Launcher."),
            ),
        };
        let typed = Rc::clone(&name);
        let mut parts: Vec<Box<dyn Widget>> = vec![match launcher.editable_name {
            true => Box::new(
                TextField::new(&tr!("Name"), &launcher.name, move |text| {
                    *typed.borrow_mut() = text.to_owned()
                })
                .without_text_checking(),
            ),
            false => Box::new(Label::new(&launcher.name)),
        }];
        if let Some(target) = &launcher.target {
            parts.push(Box::new(Label::new(target).secondary()));
        }
        let alert = Alert::new(
            &title.tr(),
            &message.tr_args(&[("app", &asking as &dyn std::fmt::Display)]),
            move |action| chosen.set(action == ADD),
        )
        .accessory(Stack::new(Axis::Vertical, SPACING, 0.0, parts))
        .cancel(&tr!("Cancel"), "cancel")
        .primary(&tr!("Add"), ADD);
        match icon(&bytes, pixels) {
            Some(icon) => alert.icon(icon),
            None => alert,
        }
    });
    match asked {
        Ok(()) if added.get() => {
            let mut stdout = std::io::stdout();
            match stdout
                .write_all(name.borrow().trim().as_bytes())
                .and_then(|()| stdout.flush())
            {
                Ok(()) => ExitCode::from(CHOSEN),
                Err(error) => {
                    tracing::error!("cannot hand the name to the portal: {error}");
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
