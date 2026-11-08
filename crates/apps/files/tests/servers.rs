#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::time::{Duration, Instant};

use bande_files::{
    Browser, FilesView, Location, Request, ServerAnswer, ServerRequest, ServersSeen, Sheet,
};
use bande_fs::Trash;
use bande_toolkit::ui::{Event, Key, Modifiers};
use bande_toolkit::{Palette, Scheme, TextSystem};
use bande_xdg::Apps;
use rustix::event::{PollFd, PollFlags, Timespec, poll};

#[test]
fn a_connected_folder_never_loads_on_the_ui_thread_and_connect_to_server_opens() {
    let runtime = tempfile::tempdir().expect("runtime dir");
    let trash = tempfile::tempdir().expect("trash dir");
    let share = runtime.path().join("bande/volumes/share");
    fs::create_dir_all(&share).expect("share");
    fs::write(share.join("film.mkv"), b"x").expect("file");
    unsafe { std::env::set_var("XDG_RUNTIME_DIR", runtime.path()) };

    let browser = Browser::open(
        Location::Folder(trash.path().to_path_buf()),
        Trash::isolated(trash.path().join("trash"), 0),
    );
    let mut view = FilesView::new(
        browser,
        Vec::new(),
        Apps::load(Vec::new(), &[]),
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
    )
    .expect("view");
    view.scene(1000, 600);

    view.go(Location::Folder(share.clone()));
    assert!(view.browser().columns()[0].entries.is_empty());
    assert!(view.browser().columns()[0].is_listing());

    let deadline = Instant::now() + Duration::from_secs(5);
    while view.browser().columns()[0].entries.is_empty() {
        assert!(Instant::now() < deadline, "the listing never arrived");
        let ready = {
            let fds = view.fds();
            let mut polls: Vec<_> = fds
                .iter()
                .map(|fd| PollFd::new(fd, PollFlags::IN))
                .collect();
            let wait = Timespec {
                tv_sec: 0,
                tv_nsec: 20_000_000,
            };
            poll(&mut polls, Some(&wait)).expect("poll");
            polls
                .iter()
                .enumerate()
                .filter(|(_, fd)| !fd.revents().is_empty())
                .map(|(index, _)| index)
                .collect::<Vec<_>>()
        };
        for index in ready.into_iter().rev() {
            view.source_ready(index);
        }
    }
    assert_eq!(view.browser().columns()[0].entries[0].name, "film.mkv");

    view.handle(&Event::Key {
        key: Key::Character('k'),
        modifiers: Modifiers {
            control: true,
            ..Modifiers::default()
        },
    });
    assert!(matches!(view.sheet(), Some(Sheet::ConnectServer { .. })));
}

#[test]
fn the_network_folder_lists_computers_whose_shares_connect_when_opened() {
    let trash = tempfile::tempdir().expect("trash dir");
    let browser = Browser::open(
        Location::Network,
        Trash::isolated(trash.path().join("trash"), 0),
    );
    let mut view = FilesView::new(
        browser,
        Vec::new(),
        Apps::load(Vec::new(), &[]),
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
    )
    .expect("view");
    view.set_nearby(ServersSeen {
        nearby: vec![("Studio NAS".to_owned(), "smb://nas.local".to_owned())],
        unresponsive: Vec::new(),
    });
    view.scene(1000, 600);
    assert_eq!(view.browser().columns()[0].entries[0].name, "Studio NAS");

    let press = |view: &mut FilesView, key: Key, control: bool| {
        view.handle(&Event::Key {
            key,
            modifiers: Modifiers {
                control,
                ..Modifiers::default()
            },
        });
    };
    press(&mut view, Key::Down, false);
    assert_eq!(
        view.browser().chosen_server().as_deref(),
        Some("smb://nas.local")
    );
    assert!(view.browser().selection().is_empty());
    press(&mut view, Key::Delete, false);
    press(&mut view, Key::Character('o'), true);
    let connecting = |view: &mut FilesView| {
        view.take_requests()
            .into_iter()
            .find_map(|request| match request {
                Request::ConnectServer { token, request } => Some((token, request)),
                _ => None,
            })
            .expect("connecting")
    };
    let (token, request) = connecting(&mut view);
    assert_eq!(request.url, "smb://nas.local");
    assert!(view.operations().running().is_empty());
    assert_eq!(view.browser().columns()[0].entries.len(), 1);

    view.server_answered(ServerAnswer {
        token,
        request: ServerRequest {
            user: "mikael".to_owned(),
            ..request
        },
        status: "shares".to_owned(),
        details: vec!["Media".to_owned(), "Backups".to_owned()],
    });
    assert_eq!(
        *view.browser().root(),
        Location::Server("smb://nas.local".to_owned())
    );
    let shares: Vec<_> = view.browser().columns()[0]
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(shares, ["Backups", "Media"]);
    press(&mut view, Key::Down, false);
    press(&mut view, Key::Down, false);
    press(&mut view, Key::Character('o'), true);
    let (_, request) = connecting(&mut view);
    assert_eq!(request.url, "smb://nas.local/Media");
    assert_eq!(request.user, "mikael");
}
