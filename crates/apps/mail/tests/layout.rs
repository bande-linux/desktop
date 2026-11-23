#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bande_mail::net::{Security, Server};
use bande_mail::settings::MailSettings;
use bande_mail::store::{Account, Store};
use bande_mail::ui::{MailView, Panes, Request};
use std::path::{Path, PathBuf};

use bande_mail::imap::{Fetched, Folder, Role};
use bande_toolkit::ui::{Event, Key, Modifiers, Pointer, Scroll, ScrollPhase, ScrollSource};
use bande_toolkit::{Appearance, HeaderInsets, Palette, Point, Scheme, TextSystem, WindowButtons};

fn view_of(store_path: PathBuf) -> MailView {
    let mut view = MailView::new(
        store_path,
        MailSettings::default(),
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
    )
    .unwrap();
    view.reload();
    view
}

fn account() -> Account {
    let server = Server {
        host: "127.0.0.1".to_owned(),
        port: 1,
        security: Security::Plain,
    };
    Account {
        id: 0,
        name: "Ann".to_owned(),
        email: "ann@bande.test".to_owned(),
        user: "ann@bande.test".to_owned(),
        imap: server.clone(),
        smtp: server,
        signature: String::new(),
        enabled: true,
        description: String::new(),
    }
}

#[test]
fn every_pane_keeps_its_header_controls_clear_of_the_window_buttons_at_any_width() {
    let folder = tempfile::tempdir().unwrap();
    let store_path = folder.path().join("mail.sqlite3");
    let store = Store::open(&store_path).unwrap();
    store.add_account(&account()).unwrap();
    let mut view = view_of(store_path);
    let height = 800;
    for (width, panes) in [(1280, Panes::Three), (900, Panes::Two), (600, Panes::One)] {
        for buttons in [WindowButtons::Right, WindowButtons::Left] {
            view.set_appearance(Appearance {
                window_buttons: buttons,
                ..Appearance::default()
            });
            view.scene(width, height);
            view.scene(width, height);
            assert_eq!(view.panes(), panes);
            let insets = HeaderInsets::of(buttons);
            let header = view.header();
            assert!(header.title_x >= width as f32, "the panes own the header");
            for item in &header.items {
                let clear = match buttons {
                    WindowButtons::Right => item.x + item.width <= width as f32 - insets.trailing,
                    WindowButtons::Left => item.x >= insets.leading,
                };
                assert!(
                    item.x >= 0.0 && clear,
                    "{item:?} runs into the window buttons on the {buttons:?} at {width}"
                );
            }
            let names = view
                .accessibility()
                .nodes
                .into_iter()
                .filter_map(|(_, node)| node.label().map(str::to_owned))
                .collect::<Vec<_>>();
            for name in [
                "New Message",
                "Reply",
                "Archive",
                "More",
                "Get Mail",
                "Sort by",
            ] {
                assert!(
                    names.iter().any(|known| known == name),
                    "no {name} with the buttons on the {buttons:?} at {width}"
                );
            }
            assert_eq!(view.sidebar_region() > 0.0, panes == Panes::Three);
        }
    }

    view.scene(1280, height);
    let edge = Point::new(220.0 + 340.0, 400.0);
    let moved = Point::new(edge.x + 40.0, edge.y);
    view.handle(&Event::PointerDown(Pointer::at(edge)));
    view.handle(&Event::PointerMove(Pointer::at(moved)));
    view.handle(&Event::PointerUp(Pointer::at(moved)));
    assert_eq!(view.settings().list_width, 380, "the list edge drags");
    let kept = std::fs::read_to_string(
        Path::new(&std::env::var_os("XDG_CONFIG_HOME").unwrap()).join("bande/mail.conf"),
    )
    .unwrap();
    assert!(
        kept.contains("list-width = 380"),
        "the width is kept: {kept}"
    );
}

fn node(view: &MailView, label: &str) -> Option<accesskit::Node> {
    view.accessibility()
        .nodes
        .into_iter()
        .map(|(_, node)| node)
        .find(|node| node.label() == Some(label))
}

#[test]
fn the_list_says_when_it_is_empty_and_acts_on_messages_by_panel_swipe_and_keys() {
    let folder = tempfile::tempdir().unwrap();
    let store_path = folder.path().join("mail.sqlite3");
    let mut store = Store::open(&store_path).unwrap();
    let id = store.add_account(&account()).unwrap();
    let inbox = Folder {
        path: "INBOX".to_owned(),
        name: "Inbox".to_owned(),
        delimiter: None,
        role: Some(Role::Inbox),
        selectable: true,
    };
    let archive = Folder {
        path: "Archive".to_owned(),
        name: "Archive".to_owned(),
        delimiter: None,
        role: Some(Role::Archive),
        selectable: true,
    };
    store.replace_mailboxes(id, &[inbox, archive]).unwrap();
    let mut view = view_of(store_path);
    view.scene(1280, 800);
    let list = view
        .accessibility()
        .nodes
        .into_iter()
        .map(|(_, node)| node)
        .find(|node| node.role() == accesskit::Role::ListBox && node.label() == Some("Inbox"))
        .unwrap();
    assert!(
        list.description()
            .is_some_and(|text| text.starts_with("Inbox is empty")),
        "{list:?}"
    );

    let mailbox = store
        .mailboxes(id)
        .unwrap()
        .into_iter()
        .find(|mailbox| mailbox.role == Some(Role::Inbox))
        .unwrap()
        .id;
    let fetched = [7, 8].map(|uid| Fetched {
        uid,
        flags: None,
        size: Some(10),
        internal_date: Some(1),
        header: Some(
            format!("From: Ada <ada@example.org>\r\nSubject: Plan {uid}\r\n\r\n").into_bytes(),
        ),
        body: None,
    });
    store.insert_headers(mailbox, &fetched).unwrap();
    view.reload();
    view.scene(1280, 800);
    let width = |view: &mut MailView| {
        view.scene(1280, 800);
        node(view, "Mark as Read")
            .and_then(|node| node.bounds())
            .map_or(0.0, |bounds| bounds.x1 - bounds.x0)
    };
    assert_eq!(
        width(&mut view),
        0.0,
        "no selection panel for one message or none"
    );
    view.handle(&Event::Key {
        key: Key::Character('a'),
        modifiers: Modifiers {
            control: true,
            ..Modifiers::default()
        },
    });
    view.scene(1280, 800);
    assert!(
        width(&mut view) > 0.0,
        "choosing both messages offers Mark as Read"
    );

    view.handle(&Event::Key {
        key: Key::Escape,
        modifiers: Modifiers::default(),
    });
    let row = Point::new(400.0, 112.0 + 26.0 + 20.0);
    view.handle(&Event::PointerMove(Pointer::at(row)));
    view.scene(1280, 800);
    for (delta, phase) in [
        (-50.0, ScrollPhase::Began),
        (-60.0, ScrollPhase::Changed),
        (0.0, ScrollPhase::Ended),
    ] {
        view.handle(&Event::Scroll(Scroll {
            position: row,
            delta: Point::new(delta, 0.0),
            source: ScrollSource::Finger,
            phase,
            ..Scroll::default()
        }));
        view.scene(1280, 800);
    }
    assert_eq!(
        view.messages().len(),
        1,
        "a swipe to the right archives the row"
    );

    let press = |view: &mut MailView, key: Key, control: bool| {
        view.handle(&Event::Key {
            key,
            modifiers: Modifiers {
                control,
                ..Modifiers::default()
            },
        });
        view.scene(1280, 800);
    };
    press(&mut view, Key::Character('j'), false);
    assert!(view.selected().is_some(), "J chooses the next message");
    press(&mut view, Key::Character('v'), false);
    let finder = view
        .take_requests()
        .into_iter()
        .any(|request| matches!(request, Request::ShowMenu(menu, _) if menu.search.is_some()));
    assert!(finder, "V opens a mailbox finder to type into");
    let inbox = view.place().clone();
    press(&mut view, Key::Character('2'), true);
    assert_ne!(*view.place(), inbox, "Ctrl+2 goes to the second mailbox");
    press(&mut view, Key::Character('?'), false);
    assert!(view.has_sheet(), "? lists the shortcuts");
}
