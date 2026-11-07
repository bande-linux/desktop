#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::time::{Duration, Instant};

use bande_files::{Browser, FilesView, Location, Sheet};
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
