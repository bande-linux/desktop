use std::cell::Cell;
use std::process::ExitCode;
use std::rc::Rc;

use bande_file_chooser::L10N;
use bande_toolkit::Image;
use bande_toolkit::ui::{Alert, Axis, Label, Stack};
use bande_xdg::{
    ICON_THEMES, Locale, app_name, applications, current_desktops, data_dirs, find_icon,
};
use xdg_desktop_portal_bande::account::Details;
use xdg_desktop_portal_bande::dialog_process::{CANCELLED, CHOSEN};

const SHARE: &str = "share";
const SPACING: f32 = 4.0;
const PICTURE_POINTS: f64 = 36.0;
const FALLBACK_PICTURE: &str = "avatar-default";
const UNAVAILABLE: u8 = 2;

fn picture(details: &Details, pixels: u32) -> Option<Image> {
    details
        .picture
        .as_deref()
        .and_then(|path| Image::load(path, pixels))
        .or_else(|| {
            find_icon(FALLBACK_PICTURE, pixels, &ICON_THEMES, &data_dirs())
                .and_then(|path| Image::load(&path, pixels))
        })
}

pub fn share(details: &Details) -> ExitCode {
    let entries = applications(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    );
    let app = app_name(&entries, &details.app_id);
    let shared = Rc::new(Cell::new(false));
    let chosen = Rc::clone(&shared);
    let asked = bande_toolkit::ask("share-account", |window| {
        let pixels = (PICTURE_POINTS * window.scale().max(2.0)).ceil() as u32;
        let message = details
            .reason
            .clone()
            .unwrap_or_else(|| L10N.tr_args("account-message", [("app", app.clone().into())]));
        let alert = Alert::new(
            &L10N.tr_args("account-title", [("app", app.into())]),
            &message,
            move |action| chosen.set(action == SHARE),
        )
        .accessory(Stack::new(
            Axis::Vertical,
            SPACING,
            0.0,
            vec![
                Box::new(Label::new(&details.name)),
                Box::new(Label::new(&details.login).secondary()),
            ],
        ))
        .cancel(&L10N.tr("button-cancel"), "cancel")
        .primary(&L10N.tr("button-share"), SHARE);
        match picture(details, pixels) {
            Some(picture) => alert.icon(picture),
            None => alert,
        }
    });
    match asked {
        Ok(()) if shared.get() => ExitCode::from(CHOSEN),
        Ok(()) => ExitCode::from(CANCELLED),
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::from(UNAVAILABLE)
        }
    }
}
