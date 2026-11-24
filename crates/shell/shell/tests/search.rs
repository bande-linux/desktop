#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::{Duration, Instant};

use bande_compositor::{Compositor, OutputConfig, Snapshot};
use bande_search::{BUS_NAME, Found, OBJECT_PATH};
use bande_test_wayland::keys::{BTN_LEFT, KEY_A, KEY_BACKSPACE, KEY_LEFTMETA};
use bande_test_wayland::{Frames, chord, headless};
use smithay::backend::input::{
    AbsolutePositionEvent, Device, DeviceCapability, Event, InputBackend, InputEvent,
    TouchDownEvent, TouchEvent, TouchFrameEvent, TouchSlot, TouchUpEvent, UnusedEvent,
};
use smithay::utils::{Logical, Point, Size};
use zbus::blocking::connection::Builder;

const WIDTH: i32 = 1280;
const HEIGHT: i32 = 800;
const SECOND: &str = "Second";
const PATIENCE: Duration = Duration::from_secs(40);
const HELD: Duration = Duration::from_millis(60);
const SETTLED: Duration = Duration::from_millis(700);
const CARD: (i32, i32) = (WIDTH / 2 - 360, 176);
const LOGO: (i32, i32) = (40, 34);
const QUICK_SETTINGS: (i32, i32) = (28, HEIGHT - 110);
const CLOCK: (i32, i32) = (40, HEIGHT - 33);
const DESKTOP: (i32, i32) = (WIDTH - 100, HEIGHT - 100);
const SEARCH_AREA: (i32, i32, i32, i32) = (CARD.0 + 20, CARD.1 + 14, 600, 30);
const PANEL_AREA: (i32, i32, i32, i32) = (90, HEIGHT - 160, 110, 40);

type Press = fn(&mut Session, (i32, i32));

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

#[derive(Debug, PartialEq, Eq, Hash)]
struct Finger;

#[derive(Debug)]
struct Touch(Point<f64, Logical>);

impl Device for Finger {
    fn id(&self) -> String {
        "finger".to_owned()
    }
    fn name(&self) -> String {
        self.id()
    }
    fn has_capability(&self, capability: DeviceCapability) -> bool {
        capability == DeviceCapability::Touch
    }
    fn usb_id(&self) -> Option<(u32, u32)> {
        None
    }
    fn syspath(&self) -> Option<PathBuf> {
        None
    }
}

impl Event<Finger> for Touch {
    fn time(&self) -> u64 {
        0
    }
    fn device(&self) -> Finger {
        Finger
    }
}

impl AbsolutePositionEvent<Finger> for Touch {
    fn x(&self) -> f64 {
        self.0.x
    }
    fn y(&self) -> f64 {
        self.0.y
    }
    fn x_transformed(&self, _: i32) -> f64 {
        self.0.x
    }
    fn y_transformed(&self, _: i32) -> f64 {
        self.0.y
    }
}

impl TouchEvent<Finger> for Touch {
    fn slot(&self) -> TouchSlot {
        Some(0).into()
    }
}

impl TouchDownEvent<Finger> for Touch {}
impl TouchUpEvent<Finger> for Touch {}
impl TouchFrameEvent<Finger> for Touch {}

impl InputBackend for Finger {
    type Device = Self;
    type KeyboardKeyEvent = UnusedEvent;
    type PointerAxisEvent = UnusedEvent;
    type PointerButtonEvent = UnusedEvent;
    type PointerMotionEvent = UnusedEvent;
    type PointerMotionAbsoluteEvent = UnusedEvent;
    type GestureSwipeBeginEvent = UnusedEvent;
    type GestureSwipeUpdateEvent = UnusedEvent;
    type GestureSwipeEndEvent = UnusedEvent;
    type GesturePinchBeginEvent = UnusedEvent;
    type GesturePinchUpdateEvent = UnusedEvent;
    type GesturePinchEndEvent = UnusedEvent;
    type GestureHoldBeginEvent = UnusedEvent;
    type GestureHoldEndEvent = UnusedEvent;
    type TouchDownEvent = Touch;
    type TouchUpEvent = Touch;
    type TouchMotionEvent = UnusedEvent;
    type TouchCancelEvent = UnusedEvent;
    type TouchFrameEvent = Touch;
    type TabletToolAxisEvent = UnusedEvent;
    type TabletToolProximityEvent = UnusedEvent;
    type TabletToolTipEvent = UnusedEvent;
    type TabletToolButtonEvent = UnusedEvent;
    type SwitchToggleEvent = UnusedEvent;
    type SpecialEvent = ();
}

struct Session {
    server: Compositor,
    frames: Frames,
    shell: Child,
}

impl Session {
    fn open() -> Self {
        let mut server = headless(WIDTH, HEIGHT);
        server.add_output(OutputConfig::virtual_display(
            SECOND,
            Size::from((WIDTH, HEIGHT)),
        ));
        let shell = server
            .spawn_shell_client(Path::new(env!("CARGO_BIN_EXE_bande-shell")))
            .expect("bande-shell");
        let mut frames = Frames::start();
        frames.add_output(SECOND);
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

    fn click(&mut self, (x, y): (i32, i32)) {
        self.server
            .move_pointer_to(Point::from((f64::from(x), f64::from(y))), 0);
        self.pass(HELD);
        self.server.press_button(BTN_LEFT, true, 0);
        self.pass(HELD);
        self.server.press_button(BTN_LEFT, false, 0);
    }

    fn tap(&mut self, (x, y): (i32, i32)) {
        let touch = || Touch(Point::from((f64::from(x), f64::from(y))));
        self.touch(InputEvent::TouchDown { event: touch() });
        self.pass(HELD);
        self.touch(InputEvent::TouchUp { event: touch() });
    }

    fn touch(&mut self, event: InputEvent<Finger>) {
        let frame = InputEvent::<Finger>::TouchFrame {
            event: Touch(Point::default()),
        };
        self.server.handle_input(event).expect("a touch");
        self.server.handle_input(frame).expect("a touch frame");
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

fn differs(empty: &Snapshot, shown: &Snapshot, (x, y, w, h): (i32, i32, i32, i32)) -> bool {
    let changed = (y..y + h)
        .flat_map(|y| (x..x + w).map(move |x| (x, y)))
        .filter(|&at| pixel(empty, at) != pixel(shown, at))
        .count();
    changed * 2 > (w * h) as usize
}

#[test]
fn a_click_on_what_opened_a_panel_closes_it() {
    let mut session = Session::open();
    let displays = [session.server.main_output(), SECOND.to_owned()];
    let picture = |session: &mut Session| {
        session.pass(SETTLED);
        displays
            .each_ref()
            .map(|output| session.frames.snapshot_output(output))
    };
    let empty = picture(&mut session);
    let open = |shown: &[Snapshot; 2], area| {
        empty
            .iter()
            .zip(shown)
            .any(|(empty, shown)| differs(empty, shown, area))
    };
    let (nothing, search, panel) = ((false, false), (true, false), (false, true));
    let steps = [
        ("the logo", LOGO, search),
        ("the logo again", LOGO, nothing),
        ("the logo", LOGO, search),
        ("Quick Settings while Search is open", QUICK_SETTINGS, panel),
        ("Quick Settings again", QUICK_SETTINGS, nothing),
        ("the clock", CLOCK, panel),
        ("the clock again", CLOCK, nothing),
        ("the clock", CLOCK, panel),
        ("the logo while Notifications are open", LOGO, search),
        ("the desktop", DESKTOP, nothing),
    ];
    let presses: [(&str, i32, Press); 3] = [
        ("a click", 0, Session::click),
        ("a click on the second display", WIDTH, Session::click),
        ("a tap", 0, Session::tap),
    ];
    for (press, left, act) in presses {
        for (what, (x, y), expected) in steps {
            act(&mut session, (left + x, y));
            let shown = picture(&mut session);
            let found = (open(&shown, SEARCH_AREA), open(&shown, PANEL_AREA));
            assert_eq!(found, expected, "(Search, a panel) after {press} on {what}");
        }
    }
    for shows in [search, nothing] {
        chord(&mut session.server, &[KEY_LEFTMETA]);
        let shown = picture(&mut session);
        let found = (open(&shown, SEARCH_AREA), open(&shown, PANEL_AREA));
        assert_eq!(found, shows, "(Search, a panel) after a tap of Super");
    }
}
