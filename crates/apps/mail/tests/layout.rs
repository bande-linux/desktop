#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bande_mail::net::{Security, Server};
use bande_mail::settings::MailSettings;
use bande_mail::store::{Account, Store};
use bande_mail::ui::{MailView, Panes};
use bande_toolkit::ui::{Event, Pointer};
use bande_toolkit::{Appearance, HeaderInsets, Palette, Point, Scheme, TextSystem, WindowButtons};

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
    for (variable, name) in [
        ("HOME", ""),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
    ] {
        let path = folder.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        // SAFETY: this test is alone in its binary and sets the variables before any other thread starts.
        unsafe { std::env::set_var(variable, path) };
    }
    let store_path = folder.path().join("mail.sqlite3");
    let store = Store::open(&store_path).unwrap();
    store.add_account(&account()).unwrap();
    let mut view = MailView::new(
        store_path,
        MailSettings::default(),
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
    )
    .unwrap();
    view.reload();
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
    let kept = std::fs::read_to_string(folder.path().join("config/bande/mail.conf")).unwrap();
    assert!(
        kept.contains("list-width = 380"),
        "the width is kept: {kept}"
    );
}
