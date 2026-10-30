use std::cell::Cell;
use std::rc::Rc;

use unbound_toolkit::WindowError;
use unbound_toolkit::ui::Alert;

use crate::L10N;

pub const CONFIRM_DELETE_FLAG: &str = "--confirm-delete";

pub fn confirm_delete(count: usize) -> Result<bool, WindowError> {
    let confirmed = Rc::new(Cell::new(false));
    let chosen = confirmed.clone();
    unbound_toolkit::ask("desktop-confirmation", |_| {
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
