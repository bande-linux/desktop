use unbound_inbox::Outbox;
use unbound_notify::WirePresentation;
use unbound_notify::client::NotificationCenter;
use zbus::blocking::Connection;

use crate::Update;

fn unread((_, center, ..): &WirePresentation) -> bool {
    !center.is_empty()
}

pub(crate) fn follow_unread(connection: &Connection, outbox: &Outbox<Update>) -> zbus::Result<()> {
    let center = NotificationCenter::new(connection)?;
    let changes = center.receive_presentation_changed()?;
    if !outbox.send(Update::Unread(unread(&center.get_presentation()?))) {
        return Ok(());
    }
    for change in changes {
        if let Ok(arguments) = change.args()
            && !outbox.send(Update::Unread(unread(&arguments.presentation)))
        {
            break;
        }
    }
    Ok(())
}
