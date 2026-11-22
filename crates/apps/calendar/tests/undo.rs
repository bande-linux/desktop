#![allow(clippy::unwrap_used)]

use bande_calendar::store::{Calendar, CalendarInfo, Library, Pending, Stored};
use bande_calendar::sync::{Outcome, apply};

const HREF: &str = "/dav/work/meeting.ics";
const EVENT: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:meeting\r\n\
DTSTART:20261103T100000Z\r\nDTEND:20261103T110000Z\r\nSUMMARY:Meeting\r\nEND:VEVENT\r\n\
END:VCALENDAR\r\n";

fn deleted_on_the_server() -> Outcome {
    Outcome {
        calendar: "work".into(),
        ctag: None,
        sync_token: None,
        upserts: Vec::new(),
        removed: vec![HREF.into()],
        conflicts: Vec::new(),
        copies: Vec::new(),
    }
}

#[test]
fn an_undone_delete_comes_back_at_its_own_address_even_while_the_delete_is_being_pushed() {
    let mut library = Library::default();
    let mut calendar = Calendar::new(CalendarInfo {
        id: "work".into(),
        account: "caldav".into(),
        url: Some("/dav/work/".into()),
        name: "Work".into(),
        color: "#3366CC".into(),
        read_only: false,
        visible: true,
        ctag: None,
        sync_token: None,
        refreshed: None,
    });
    calendar.objects.push(Stored::new(
        HREF.into(),
        Some("\"1\"".into()),
        EVENT.into(),
        None,
    ));
    library.calendars.push(calendar);

    library.remove_object(0, 0);
    library.restore(0, HREF, EVENT.into()).unwrap();
    apply(&mut library, deleted_on_the_server(), false);
    let restored = &library.calendars[0].objects;
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].href, HREF);
    assert_eq!(restored[0].etag, None);
    assert_eq!(restored[0].pending, Some(Pending::Create));
    assert_eq!(restored[0].parsed.as_ref().unwrap().uid(), "meeting");

    library.calendars[0].objects[0].pending = None;
    library.remove_object(0, 0);
    apply(&mut library, deleted_on_the_server(), false);
    assert!(library.calendars[0].objects.is_empty());
    library.restore(0, HREF, EVENT.into()).unwrap();
    assert_eq!(library.calendars[0].objects[0].href, HREF);
    assert_eq!(
        library.calendars[0].objects[0].pending,
        Some(Pending::Create)
    );
}
