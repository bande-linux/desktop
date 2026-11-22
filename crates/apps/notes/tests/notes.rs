#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use bande_inbox::Inbox;
use bande_mail_core::accounts::{self, Account};
use bande_mail_core::imap::{Flag, Session};
use bande_mail_core::net::Network;
use bande_mail_core::testing::{Dovecot, PASSWORD, USER, local};
use bande_notes::note::{self, from_plain};
use bande_notes::store::Store;
use bande_notes::worker::{Command, Event, Worker};

const WAIT: Duration = Duration::from_secs(20);
const FROM_ELSEWHERE: &str = "From: <ann@bande.test>\r\n\
Date: Sun, 16 Mar 2014 08:00:00 +0100\r\n\
X-Uniform-Type-Identifier: com.apple.mail-note\r\n\
X-Universally-Unique-Identifier: 0F1E2D3C-4B5A-4978-8695-A4B3C2D1E0F9\r\n\
X-Mail-Created-Date: Sat, 15 Mar 2014 10:12:00 +0100\r\n\
Subject: Groceries\r\n\
Content-Type: text/html; charset=utf-8\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\r\n\
<html><head></head><body>Groceries<div>bread</div></body></html>\r\n";

fn wait_for(events: &mut Inbox<Event>, wanted: impl Fn(&Event) -> bool) {
    let deadline = Instant::now() + WAIT;
    loop {
        if events.drain().iter().any(&wanted) {
            return;
        }
        assert!(Instant::now() < deadline, "the notes never synced");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn server_notes(session: &mut Session) -> Vec<note::Note> {
    session.select("Notes").unwrap();
    let uids = session.uids().unwrap();
    session
        .fetch_bodies(&uids)
        .unwrap()
        .into_iter()
        .filter_map(|fetched| note::read(&fetched.body?))
        .collect()
}

#[test]
fn a_note_written_elsewhere_is_edited_here_and_a_conflicting_edit_keeps_both() {
    if !Dovecot::installed() {
        eprintln!("dovecot is not installed; skipping");
        return;
    }
    let dovecot = Dovecot::start().unwrap();
    let path = dovecot.folder().join("notes.sqlite3");
    let store = Store::open(&path).unwrap();
    let mail = accounts::open(&dovecot.folder().join("mail.sqlite3")).unwrap();
    let mut account = Account {
        id: 0,
        name: "Ann".to_owned(),
        email: USER.to_owned(),
        user: USER.to_owned(),
        imap: local(dovecot.port()),
        smtp: local(1),
        signature: String::new(),
        enabled: true,
    };
    account.id = accounts::add(&mail, &account).unwrap();
    let mut events = Inbox::new().unwrap();
    let worker = Worker::spawn(
        account.clone(),
        Some(PASSWORD.to_owned()),
        path,
        Network::default(),
        events.outbox(),
    );
    wait_for(&mut events, |event| *event == Event::Online(account.id));

    let mut elsewhere = Session::connect(&account.imap, &Network::default()).unwrap();
    elsewhere.login(USER, PASSWORD).unwrap();
    elsewhere
        .append("Notes", &[Flag::Seen], FROM_ELSEWHERE.as_bytes())
        .unwrap();
    worker.send(Command::Sync);
    wait_for(&mut events, |event| *event == Event::Changed(account.id));
    let folder = store
        .folders()
        .unwrap()
        .into_iter()
        .find(|folder| folder.account == account.id)
        .unwrap();
    let listed = store.summaries(folder.id, "bread").unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].title, "Groceries");

    store
        .edit(
            listed[0].id,
            &from_plain("Groceries\nbread\nmilk"),
            1_700_000_000,
        )
        .unwrap();
    worker.send(Command::Sync);
    wait_for(&mut events, |event| *event == Event::Changed(account.id));
    let notes = server_notes(&mut elsewhere);
    assert_eq!(notes.len(), 1, "the edit did not replace the old version");
    assert_eq!(notes[0].uuid, "0F1E2D3C-4B5A-4978-8695-A4B3C2D1E0F9");
    assert_eq!(notes[0].created, 1_394_874_720);
    assert_eq!(notes[0].modified, 1_700_000_000);
    assert_eq!(notes[0].rich.text, "Groceries\nbread\nmilk");

    let old = elsewhere.uids().unwrap();
    let theirs = note::Note {
        rich: from_plain("Groceries\nbread\neggs"),
        modified: 1_700_000_100,
        ..notes[0].clone()
    };
    elsewhere
        .append("Notes", &[Flag::Seen], &note::write(&theirs, "Ann", USER))
        .unwrap();
    elsewhere.expunge(&old).unwrap();
    store
        .edit(
            listed[0].id,
            &from_plain("Groceries\nbread\nbutter"),
            1_700_000_200,
        )
        .unwrap();
    worker.send(Command::Sync);
    wait_for(&mut events, |event| *event == Event::Changed(account.id));
    let mut texts = server_notes(&mut elsewhere)
        .into_iter()
        .map(|note| (note.rich.text, note.uuid))
        .collect::<Vec<_>>();
    texts.sort();
    assert_eq!(texts.len(), 2, "a conflicting edit lost a version");
    assert_eq!(texts[0].0, "Groceries\nbread\nbutter");
    assert_eq!(texts[1].0, "Groceries\nbread\neggs");
    assert_ne!(texts[0].1, texts[1].1);
    assert_eq!(store.summaries(folder.id, "").unwrap().len(), 2);
}
