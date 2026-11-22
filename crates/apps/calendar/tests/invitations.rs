#![allow(clippy::unwrap_used)]

use bande_calendar::ical::CalendarObject;
use bande_calendar::invite::{self, Answer};

fn object(method: &str, organizer: &str, sequence: u32, extra: &str) -> CalendarObject {
    let text = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:{method}\r\nBEGIN:VEVENT\r\nUID:meeting\r\n\
DTSTAMP:20261101T000000Z\r\nDTSTART:20261103T100000Z\r\nDTEND:20261103T110000Z\r\n\
SEQUENCE:{sequence}\r\nSUMMARY:Meeting\r\nORGANIZER:mailto:{organizer}\r\n{extra}END:VEVENT\r\n\
END:VCALENDAR\r\n"
    );
    CalendarObject::parse(&text).unwrap().remove(0)
}

#[test]
fn only_the_organizer_can_update_or_cancel_and_only_an_invitee_can_answer_for_itself() {
    let alarm = "BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\n";
    let me = ["me@example.org".to_owned()];
    let invited = "ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:me@example.org\r\n";
    let stored = object(
        "REQUEST",
        "boss@example.org",
        2,
        &format!("{invited}{alarm}"),
    );

    let older = object("REQUEST", "boss@example.org", 1, invited);
    let stranger = object("CANCEL", "mallory@example.org", 3, invited);
    assert!(!invite::may_update(&stored, &older));
    assert!(!invite::may_update(&stored, &stranger));

    let newer = object("REQUEST", "boss@example.org", 3, invited);
    assert!(invite::may_update(&stored, &newer));
    let merged = invite::merge_update(&stored, newer);
    assert_eq!(merged.master().unwrap().alarms.len(), 1);
    let mut moved = merged.clone();
    let master = moved.master_mut().unwrap();
    master.alarms.clear();
    master.sequence += 1;
    invite::settle_sequence(&merged, &mut moved, &me);
    assert_eq!(moved.master().unwrap().sequence, 3);

    let attendees = "ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:ann@example.org\r\n\
ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:bob@example.org\r\n";
    let mine = object("REQUEST", "me@example.org", 0, attendees);
    let theirs = object("REQUEST", "boss@example.org", 0, attendees);
    let reply = object(
        "REPLY",
        "me@example.org",
        0,
        "ATTENDEE;PARTSTAT=ACCEPTED:mailto:ann@example.org\r\n",
    );
    let for_everyone = object(
        "REPLY",
        "me@example.org",
        0,
        "ATTENDEE;PARTSTAT=DECLINED:mailto:ann@example.org\r\n\
ATTENDEE;PARTSTAT=DECLINED:mailto:bob@example.org\r\n",
    );
    assert!(invite::take_reply(&theirs, &reply, &me).is_none());
    assert!(invite::take_reply(&mine, &for_everyone, &me).is_none());
    let (updated, replier) = invite::take_reply(&mine, &reply, &me).unwrap();
    assert_eq!(replier.address, "ann@example.org");
    let answers: Vec<Answer> = invite::invitees(updated.master().unwrap())
        .into_iter()
        .filter(|invitee| !invitee.organizer)
        .map(|invitee| invitee.answer)
        .collect();
    assert_eq!(answers, [Answer::Accepted, Answer::Pending]);
}
