#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::Child;
use std::time::{Duration, Instant};

use bande_compositor::{Compositor, Snapshot};
use bande_search::{BUS_NAME, Found, OBJECT_PATH};
use bande_test_wayland::keys::{KEY_A, KEY_BACKSPACE, KEY_LEFTMETA};
use bande_test_wayland::{Frames, chord, headless};
use zbus::blocking::connection::Builder;

const WIDTH: i32 = 1280;
const HEIGHT: i32 = 800;
const PATIENCE: Duration = Duration::from_secs(40);
const SETTLED: Duration = Duration::from_millis(700);
const CARD: (i32, i32) = (WIDTH / 2 - 360, 176);
const LOGO: (i32, i32) = (40, 34);

struct Index;

#[zbus::interface(name = "org.bande.Search")]
impl Index {
    fn find(&self, text: String, _within: String, _limit: u32) -> Vec<Found> {
        let (wait, count) = match text.len() {
            1 => (40, 12),
            2 => (20, 5),
            3 => (400, 12),
            4 => (40, 0),
            _ => (0, 3),
        };
        std::thread::sleep(Duration::from_millis(wait));
        (0..count)
            .map(|number| Found {
                domain: String::new(),
                kind: "file".to_owned(),
                target: format!("/home/ada/Paper {}{number}.txt", text.len()),
                title: String::new(),
                detail: String::new(),
                score: 500,
            })
            .collect()
    }
}

struct Session {
    server: Compositor,
    frames: Frames,
    shell: Child,
}

impl Session {
    fn open() -> Self {
        let mut server = headless(WIDTH, HEIGHT);
        let shell = server
            .spawn_shell_client(Path::new(env!("CARGO_BIN_EXE_bande-shell")))
            .expect("bande-shell");
        let frames = Frames::start();
        let mut session = Self {
            server,
            frames,
            shell,
        };
        let bar = |shown: &Snapshot| (0..64).any(|x| pixel(shown, (8 + x, LOGO.1))[0] > 200);
        session.wait("the taskbar", bar);
        session.pass(SETTLED);
        session
    }

    fn turn(&mut self) -> Option<Snapshot> {
        let rendered = self.frames.rendered();
        let now = self.frames.now();
        self.frames.draw_at(&mut self.server, now);
        std::thread::sleep(Duration::from_millis(1));
        (self.frames.rendered() != rendered).then(|| self.frames.snapshot(&self.server))
    }

    fn pass(&mut self, time: Duration) -> Vec<Snapshot> {
        let until = Instant::now() + time;
        let mut shown = Vec::new();
        while Instant::now() < until {
            shown.extend(self.turn());
        }
        shown
    }

    fn wait(&mut self, what: &str, mut done: impl FnMut(&Snapshot) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !self.turn().is_some_and(|shown| done(&shown)) {
            assert!(Instant::now() < deadline, "waited in vain for {what}");
        }
    }

    fn settled(&mut self) -> Snapshot {
        self.pass(SETTLED);
        self.frames.snapshot(&self.server)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.shell.kill();
        let _ = self.shell.wait();
    }
}

fn pixel(shown: &Snapshot, (x, y): (i32, i32)) -> [u8; 4] {
    let at = ((y * shown.size.w + x) * 4) as usize;
    shown.rgba[at..at + 4].try_into().unwrap()
}

fn card(shown: &Snapshot) -> ([u8; 4], i32) {
    let at = |down: i32| pixel(shown, (CARD.0 + 4, CARD.1 + down));
    let background = at(30);
    let height = (30..HEIGHT - CARD.1)
        .take_while(|&down| (0..4).any(|near| at((down - near).max(30)) == background))
        .count();
    (background, height as i32)
}

#[test]
fn the_background_of_search_never_flickers_while_results_load() {
    let _index = Builder::session()
        .unwrap()
        .name(BUS_NAME)
        .unwrap()
        .serve_at(OBJECT_PATH, Index)
        .unwrap()
        .build()
        .unwrap();
    let mut session = Session::open();
    chord(&mut session.server, &[KEY_LEFTMETA]);
    let mut before = card(&session.settled());
    let typed = [KEY_A; 6].into_iter().chain([KEY_BACKSPACE; 5]);
    let mut heights = vec![before.1];
    for (step, key) in typed.enumerate() {
        chord(&mut session.server, &[key]);
        let shown: Vec<_> = session.pass(SETTLED).iter().map(card).collect();
        let after = *shown.last().expect("a frame for the character");
        let mut changes: Vec<i32> = [before].iter().chain(&shown).map(|card| card.1).collect();
        changes.dedup();
        assert!(
            changes.len() <= 2 && shown.iter().all(|card| card.0 == before.0),
            "key {step} changed the panel from {before:?} in steps: {shown:?}"
        );
        before = after;
        heights.push(after.1);
    }
    heights.dedup();
    assert!(heights.len() > 3, "the panel never resized: {heights:?}");
}
