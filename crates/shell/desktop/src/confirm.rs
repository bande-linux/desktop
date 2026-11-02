use std::cell::Cell;
use std::rc::Rc;

use bande_fs::ExtensionChange;
use bande_toolkit::WindowError;
use bande_toolkit::ui::Alert;

use crate::L10N;

pub const CONFIRM_DELETE_FLAG: &str = "--confirm-delete";
pub const EJECT_FAILED_FLAG: &str = "--eject-failed";
pub const CONFIRM_EXTENSION_FLAG: &str = "--confirm-extension";
pub const FORCE_EJECT: &str = "force";

pub fn confirm_delete(count: usize) -> Result<bool, WindowError> {
    let confirmed = Rc::new(Cell::new(false));
    let chosen = confirmed.clone();
    bande_toolkit::ask("desktop-confirmation", |_| {
        Alert::new(
            &L10N.tr_args("delete-title", [("count", count.into())]),
            &L10N.tr("cannot-undo"),
            move |action| chosen.set(action == "delete"),
        )
        .cancel(&L10N.tr("cancel"), "cancel")
        .primary(&L10N.tr("delete"), "delete")
    })?;
    Ok(confirmed.get())
}

pub fn confirm_extension_change(change: &ExtensionChange) -> Result<bool, WindowError> {
    let confirmed = Rc::new(Cell::new(false));
    let chosen = confirmed.clone();
    bande_toolkit::ask("desktop-confirmation", |_| {
        Alert::new(&change.title(), &ExtensionChange::detail(), move |action| {
            chosen.set(action == "change")
        })
        .cancel(&change.keep(), "keep")
        .primary(&change.change(), "change")
    })?;
    Ok(confirmed.get())
}

pub fn eject_failed(name: &str, busy: bool, message: &str) -> Result<bool, WindowError> {
    let forced = Rc::new(Cell::new(false));
    let chosen = forced.clone();
    bande_toolkit::ask("desktop-eject", |_| {
        let choose = move |action: &str| chosen.set(action == FORCE_EJECT);
        match busy {
            true => Alert::new(
                &L10N.tr_args("eject-busy", [("name", name.to_owned().into())]),
                &L10N.tr("eject-busy-detail"),
                choose,
            )
            .other(&L10N.tr("force-eject"), FORCE_EJECT)
            .primary(&L10N.tr("ok"), "ok"),
            false => Alert::new(
                &L10N.tr_args("eject-failed", [("name", name.to_owned().into())]),
                message,
                choose,
            )
            .primary(&L10N.tr("ok"), "ok"),
        }
    })?;
    Ok(forced.get())
}
