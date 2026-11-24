#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::time::Instant;

use accesskit::{Action, ActionRequest, Node, Role, TreeId};
use bande_mail::imap::{Fetched, Flag, Folder, Role as Kind};
use bande_mail::net::{Security, Server};
use bande_mail::settings::MailSettings;
use bande_mail::store::{Account, Action as Change, Store};
use bande_mail::ui::{MailView, Panes, Request};
use bande_toolkit::ui::{Event, Key, Modifiers, Pointer, Scroll, ScrollPhase, ScrollSource};
use bande_toolkit::{Appearance, HeaderInsets, Palette, Point, Scheme, TextSystem, WindowButtons};

fn find(view: &MailView, found: impl Fn(&Node) -> bool) -> Option<Node> {
    view.accessibility()
        .nodes
        .into_iter()
        .map(|(_, node)| node)
        .find(found)
}

fn named(view: &MailView, label: &str) -> Option<Node> {
    find(view, |node| node.label() == Some(label))
}

fn list(view: &MailView) -> Node {
    find(view, |node| {
        node.role() == Role::ListBox && node.label() == Some("Inbox")
    })
    .unwrap()
}

fn press(view: &mut MailView, key: Key, control: bool) {
    view.handle(&Event::Key {
        key,
        modifiers: Modifiers {
            control,
            ..Modifiers::default()
        },
    });
    view.scene(1280, 800);
}

#[test]
fn every_pane_keeps_clear_of_the_window_buttons_and_the_list_acts_by_swipe_and_keys() {
    let folder = tempfile::tempdir().unwrap();
    let store_path = folder.path().join("mail.sqlite3");
    let mut store = Store::open(&store_path).unwrap();
    let server = Server {
        host: "127.0.0.1".to_owned(),
        port: 1,
        security: Security::Plain,
    };
    let id = store
        .add_account(&Account {
            id: 0,
            name: "Ann".to_owned(),
            email: "ann@bande.test".to_owned(),
            user: "ann@bande.test".to_owned(),
            imap: server.clone(),
            smtp: server,
            signature: String::new(),
            enabled: true,
            description: String::new(),
        })
        .unwrap();
    let folders = [
        ("INBOX", "Inbox", Kind::Inbox),
        ("Archive", "Archive", Kind::Archive),
    ];
    let folders = folders.map(|(path, name, role)| Folder {
        path: path.to_owned(),
        name: name.to_owned(),
        delimiter: None,
        role: Some(role),
        selectable: true,
    });
    store.replace_mailboxes(id, &folders).unwrap();
    let palette = Palette::for_scheme(Scheme::Light);
    let settings = MailSettings::default();
    let mut view = MailView::new(store_path, settings, palette, TextSystem::new()).unwrap();
    view.reload();

    let widths = [
        (1280, Panes::Three),
        (1040, Panes::Three),
        (900, Panes::Two),
        (720, Panes::Two),
        (600, Panes::One),
    ];
    for (width, panes) in widths {
        for buttons in [WindowButtons::Right, WindowButtons::Left] {
            view.set_appearance(Appearance {
                window_buttons: buttons,
                reduce_motion: true,
                ..Appearance::default()
            });
            view.scene(width, 800);
            view.scene(width, 800);
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
                    "{item:?} hits the buttons at {width}"
                );
            }
            let shown = |name: &str| named(&view, name).is_some();
            assert!(shown("New Message") && shown("Sort by"));
            for name in ["Reply", "Archive", "More"] {
                assert_eq!(shown(name), panes != Panes::One, "{name} at {width}");
            }
            for name in ["Get Mail", "Mailboxes"] {
                assert_eq!(shown(name), panes == Panes::Three, "{name} at {width}");
            }
            if panes == Panes::Two {
                view.toggle_sidebar();
                view.scene(width, 800);
                assert!(
                    named(&view, "Mailboxes").is_some(),
                    "the sidebar slides over"
                );
                assert!(
                    named(&view, "Sort by").is_none(),
                    "nothing under the sidebar shows through"
                );
                press(&mut view, Key::Escape, false);
                view.scene(width, 800);
                assert!(named(&view, "Mailboxes").is_none(), "Escape closes it");
            }
        }
    }

    view.scene(1280, 800);
    let empty = list(&view).description().unwrap_or_default().to_owned();
    assert!(empty.starts_with("Inbox is empty"), "{empty}");
    let edge = Point::new(list(&view).bounds().unwrap().x1 as f32, 400.0);
    let moved = Point::new(edge.x + 40.0, edge.y);
    view.handle(&Event::PointerDown(Pointer::at(edge)));
    view.handle(&Event::PointerMove(Pointer::at(moved)));
    view.handle(&Event::PointerUp(Pointer::at(moved)));
    let config = Path::new(&std::env::var_os("XDG_CONFIG_HOME").unwrap()).join("bande/mail.conf");
    let kept = std::fs::read_to_string(config).unwrap();
    assert!(
        kept.contains("list-width = 380"),
        "the list edge drags: {kept}"
    );

    let [inbox, archive] = [Kind::Inbox, Kind::Archive].map(|role| {
        let mailbox = store.mailbox_with_role(id, role).unwrap().unwrap();
        store.set_mailbox_state(mailbox.id, 1, Some(1)).unwrap();
        mailbox.id
    });
    let plan = |uid: u32, plan: u32| Fetched {
        uid,
        flags: None,
        size: Some(10),
        internal_date: Some(1),
        header: Some(
            format!(
                "From: <ada@example.org>\r\nMessage-ID: <{plan}@x>\r\nSubject: Plan {plan}\r\n\r\n"
            )
            .into_bytes(),
        ),
        body: None,
    };
    let plans = [plan(7, 7), plan(8, 8)];
    store.insert_headers(inbox, &plans).unwrap();
    view.reload();
    let asked = |store: &Store| {
        let asked = store.pending(id).unwrap();
        for change in &asked {
            store.finish_pending(change.id).unwrap();
        }
        let asked = asked.into_iter().filter_map(|change| match change.action {
            Change::Move(target) => Some((change.mailbox, change.uids, target)),
            _ => None,
        });
        asked.collect::<Vec<_>>()
    };
    let moved = |from: &str, uid: u32, to: &str| (from.to_owned(), vec![uid], to.to_owned());
    let panel = |view: &mut MailView| {
        view.scene(1280, 800);
        named(view, "Mark as Read")
            .and_then(|node| node.bounds())
            .is_some_and(|bounds| bounds.x1 > bounds.x0)
    };
    assert!(!panel(&mut view), "no selection panel for one message");
    press(&mut view, Key::Character('a'), true);
    assert!(panel(&mut view), "choosing both offers Mark as Read");
    press(&mut view, Key::Escape, false);

    let swipe = |view: &mut MailView, subject: &str| {
        let row = named(view, subject).and_then(|node| node.bounds()).unwrap();
        let row = Point::new(row.x0 as f32 + 100.0, (row.y0 + row.y1) as f32 / 2.0);
        view.handle(&Event::PointerMove(Pointer::at(row)));
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
    };
    press(&mut view, Key::Character('2'), true);
    press(&mut view, Key::Character('1'), true);
    swipe(&mut view, "Plan 7");
    view.tick(Instant::now());
    assert_eq!(view.messages().len(), 1, "a swipe to the right archives");
    assert_eq!(view.selected(), None, "and opens no other message");
    assert_eq!(asked(&store), [moved("INBOX", 7, "Archive")]);
    store.insert_headers(archive, &[plan(1, 7)]).unwrap();
    press(&mut view, Key::Character('z'), true);
    assert_eq!(
        asked(&store),
        [moved("Archive", 1, "INBOX")],
        "Ctrl+Z moves it back"
    );
    store.insert_headers(inbox, &[plan(9, 7)]).unwrap();
    view.reload();
    assert_eq!(view.messages().len(), 2, "and it is in the list again");

    press(&mut view, Key::Character('a'), true);
    swipe(&mut view, "Plan 8");
    swipe(&mut view, "Plan 7");
    view.tick(Instant::now());
    assert!(view.messages().is_empty(), "swipes in a row all count");
    let both = [moved("INBOX", 8, "Archive"), moved("INBOX", 9, "Archive")];
    assert_eq!(asked(&store), both);
    let arrived = [plan(2, 8), plan(3, 7)];
    store.insert_headers(archive, &arrived).unwrap();
    press(&mut view, Key::Character('z'), true);
    press(&mut view, Key::Character('z'), true);
    let back = [moved("Archive", 3, "INBOX"), moved("Archive", 2, "INBOX")];
    assert_eq!(
        asked(&store),
        back,
        "each Ctrl+Z undoes one, the last first"
    );
    let returned = [plan(10, 8), plan(11, 7)];
    store.insert_headers(inbox, &returned).unwrap();
    view.reload();

    press(&mut view, Key::Character('a'), true);
    store.remove_uids(inbox, &[10]).unwrap();
    view.reload();
    let reader = find(&view, |node| node.role() == Role::Document).unwrap();
    assert!(
        !reader.children().is_empty(),
        "the one left of several chosen messages opens"
    );
    store.insert_headers(inbox, &[plan(10, 8)]).unwrap();
    press(&mut view, Key::Character('2'), true);
    press(&mut view, Key::Character('1'), true);

    let ids = view.messages().iter().map(|summary| summary.id);
    let ids = ids.collect::<Vec<_>>();
    store.set_flag(&ids, Flag::Seen, false).unwrap();
    let (unread, _) = view
        .accessibility()
        .nodes
        .into_iter()
        .find(|(_, node)| node.role() == Role::RadioButton && node.label() == Some("Unread"))
        .unwrap();
    view.perform(&ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: unread,
        data: None,
    });
    press(&mut view, Key::Escape, false);
    press(&mut view, Key::Character('k'), false);
    view.reload();
    assert_eq!(view.messages().len(), 2, "the message being read stays");
    press(&mut view, Key::Character('j'), false);
    assert!(view.selected().is_some(), "J chooses the next message");
    view.reload();
    assert_eq!(view.messages().len(), 1, "the one read before it goes");
    press(&mut view, Key::Character('v'), false);
    let finder = view
        .take_requests()
        .into_iter()
        .any(|request| matches!(request, Request::ShowMenu(menu, _) if menu.search.is_some()));
    assert!(finder, "V opens a mailbox finder");
    let place = view.place().clone();
    press(&mut view, Key::Character('2'), true);
    assert_ne!(*view.place(), place, "Ctrl+2 goes to the second mailbox");
    view.scene(600, 800);
    view.show_message(ids[0]);
    view.scene(600, 800);
    assert!(named(&view, "Sort by").is_none(), "the reader stands alone");
    press(&mut view, Key::Character('2'), true);
    view.scene(600, 800);
    assert!(named(&view, "Sort by").is_some(), "Ctrl+2 shows the list");
    press(&mut view, Key::Character('?'), false);
    assert!(view.has_sheet(), "? lists the shortcuts");
}
