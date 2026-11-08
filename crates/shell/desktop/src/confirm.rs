use std::cell::Cell;
use std::rc::Rc;

use bande_fs::ExtensionChange;
use bande_l10n::{tr, trn};
use bande_toolkit::WindowError;
use bande_toolkit::ui::Alert;

pub const CONFIRM_DELETE_FLAG: &str = "--confirm-delete";
pub const EJECT_FAILED_FLAG: &str = "--eject-failed";
pub const CONFIRM_EXTENSION_FLAG: &str = "--confirm-extension";
pub const FORCE_EJECT: &str = "force";

pub fn confirm_delete(count: usize) -> Result<bool, WindowError> {
    let confirmed = Rc::new(Cell::new(false));
    let chosen = confirmed.clone();
    bande_toolkit::ask("desktop-confirmation", |_| {
        Alert::new(
            &trn!(
                "Delete this item immediately?",
                "Delete these {count} items immediately?",
                count = count
            ),
            &tr!("You can’t undo this action."),
            move |action| chosen.set(action == "delete"),
        )
        .cancel(&tr!("Cancel"), "cancel")
        .primary(&tr!("Delete"), "delete")
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
                &tr!("The disk “{name}” wasn’t ejected because one or more programs may be using it.", name = name.to_owned()),
                &tr!("To eject the disk immediately, click Force Eject. Programs using it may lose unsaved changes."),
                choose,
            )
            .other(&tr!("Force Eject"), FORCE_EJECT)
            .primary(&tr!("OK"), "ok"),
            false => Alert::new(
                &tr!("The disk “{name}” couldn’t be ejected.", name = name.to_owned()),
                message,
                choose,
            )
            .primary(&tr!("OK"), "ok"),
        }
    })?;
    Ok(forced.get())
}
