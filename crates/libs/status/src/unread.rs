use bande_inbox::Outbox;
use bande_notify::WirePresentation;
use bande_notify::client::NotificationCenter;
use zbus::blocking::Connection;

use crate::Update;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClockMark {
    #[default]
    Clear,
    Unread,
    Quiet,
}

impl ClockMark {
    pub fn of((_, center, do_not_disturb, ..): &WirePresentation) -> Self {
        if *do_not_disturb {
            Self::Quiet
        } else if center.is_empty() {
            Self::Clear
        } else {
            Self::Unread
        }
    }
}

pub fn set_do_not_disturb(connection: &Connection, on: bool) -> zbus::Result<()> {
    NotificationCenter::new(connection)?.set_do_not_disturb(on)
}

pub fn follow_unread(connection: &Connection, outbox: &Outbox<Update>) -> zbus::Result<()> {
    let center = NotificationCenter::new(connection)?;
    let changes = center.receive_presentation_changed()?;
    if !outbox.send(Update::ClockMark(ClockMark::of(
        &center.get_presentation()?,
    ))) {
        return Ok(());
    }
    for change in changes {
        if let Ok(arguments) = change.args()
            && !outbox.send(Update::ClockMark(ClockMark::of(&arguments.presentation)))
        {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn do_not_disturb_marks_the_clock_quiet_over_unread_notifications() {
        let stack = || (String::new(), Vec::new());
        assert_eq!(
            ClockMark::of(&(Vec::new(), Vec::new(), false, false, false)),
            ClockMark::Clear
        );
        assert_eq!(
            ClockMark::of(&(Vec::new(), vec![stack()], false, false, false)),
            ClockMark::Unread
        );
        assert_eq!(
            ClockMark::of(&(Vec::new(), vec![stack()], true, false, false)),
            ClockMark::Quiet
        );
        assert_eq!(
            ClockMark::of(&(Vec::new(), Vec::new(), true, false, false)),
            ClockMark::Quiet
        );
    }
}
