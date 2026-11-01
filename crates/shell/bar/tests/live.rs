#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::fd::BorrowedFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Once;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bande_bar::L10N;
use bande_bar::surfaces::{self, Display, Services};
use bande_compositor::{Compositor, FrameClock, FrameScheduler, REFRESH, Renderer, SessionRole};
use bande_config::Config;
use bande_controlcenter::ControlCenter;
use bande_launcher::LauncherShell;
use bande_notifications::Notifications;
use bande_test_wayland::{RED, ShmImage, TestClient};
use bande_toolkit::{App, Scene, Surfaces, TextSystem, Window, WindowEvent};
use smithay::utils::{Point, Size};
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (1024, 720);
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const KEY_ESC: u32 = 1;
const KEY_DOWN: u32 = 108;
const KEY_LEFTMETA: u32 = 125;
const KEY_F2: u32 = 60;
const KEY_F3: u32 = 61;
const WINDOW: i32 = 200;
const EDITOR: &str = "org.example.Editor";
const KEPT: usize = 16;
const HOLD: Duration = Duration::from_millis(700);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Notifications,
    Bar,
    ControlCenter,
    Launcher,
}

type Targets = Vec<(String, (f64, f64, f64, f64))>;

struct Shell {
    windows: Vec<Window>,
    parts: Vec<(Part, Box<dyn App>)>,
    events: Sender<(Part, WindowEvent)>,
    targets: Sender<Targets>,
    _text: TextSystem,
}

impl Shell {
    fn report_targets(&self, index: usize) {
        let Some(tree) = self.parts[index].1.accessibility() else {
            return;
        };
        let targets = tree
            .nodes
            .iter()
            .filter_map(|(_, node)| {
                let label = node.label()?.to_owned();
                let bounds = node.bounds()?;
                Some((label, (bounds.x0, bounds.y0, bounds.x1, bounds.y1)))
            })
            .collect();
        let _ = self.targets.send(targets);
    }
}

impl Surfaces for Shell {
    fn windows(&self) -> &[Window] {
        &self.windows
    }

    fn windows_mut(&mut self) -> &mut [Window] {
        &mut self.windows
    }

    fn scene(&mut self, index: usize) -> Scene {
        let scene = self.parts[index].1.scene(&mut self.windows[index]);
        if index == 1 {
            self.report_targets(index);
        }
        scene
    }

    fn event(&mut self, index: usize, event: WindowEvent) {
        let _ = self.events.send((self.parts[index].0, event.clone()));
        self.parts[index].1.event(&mut self.windows[index], event);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.parts
            .iter()
            .flat_map(|(_, app)| app.sources())
            .collect()
    }

    fn source_ready(&mut self, mut index: usize) {
        for (part, (_, app)) in self.parts.iter_mut().enumerate() {
            let count = app.sources().len();
            if index < count {
                app.source_ready(&mut self.windows[part], index);
                return;
            }
            index -= count;
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.parts
            .iter()
            .filter_map(|(_, app)| app.deadline())
            .min()
    }

    fn tick(&mut self, now: Instant) {
        for ((_, app), window) in self.parts.iter_mut().zip(&mut self.windows) {
            if app.deadline().is_some_and(|deadline| deadline <= now) {
                app.tick(window, now);
            }
        }
    }

    fn animating(&self, index: usize) -> bool {
        self.parts[index].1.animating()
    }
}

fn app_id(index: usize) -> String {
    format!("org.example.App{index}")
}

fn data_home() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("bar-live")
}

fn install_apps() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        let home = data_home();
        let applications = home.join("apps/applications");
        std::fs::create_dir_all(&applications).expect("applications folder");
        let mut entries: Vec<(String, String)> = (0..KEPT)
            .map(|index| (app_id(index), format!("App {index}")))
            .collect();
        entries.push((EDITOR.to_owned(), "Editor".to_owned()));
        for (id, name) in entries {
            std::fs::write(
                applications.join(format!("{id}.desktop")),
                format!(
                    "[Desktop Entry]\nType=Application\nName={name}\nExec=true\nIcon=org.bande.Files\n"
                ),
            )
            .expect("desktop entry");
        }
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data");
        let system = std::env::var_os("XDG_DATA_DIRS")
            .filter(|dirs| !dirs.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        let dirs = std::env::join_paths(
            [home.join("apps"), repository]
                .into_iter()
                .chain(std::env::split_paths(&system)),
        )
        .expect("data dirs");
        unsafe {
            std::env::set_var("XDG_DATA_HOME", home.join("home"));
            std::env::set_var("XDG_CONFIG_HOME", home.join("config"));
            std::env::set_var("XDG_DATA_DIRS", dirs);
        }
    });
}

fn shell(stream: UnixStream, events: Sender<(Part, WindowEvent)>, targets: Sender<Targets>) {
    let text = TextSystem::new();
    let mut root =
        Window::open_on(stream, bande_notifications::role(), (0, 0)).expect("notifications");
    root.share_fonts(&text);
    let notifications =
        Notifications::new(&mut root, text.sharing_fonts(), None).expect("notifications");
    let mut config = Config::default();
    config.bar.apps = [EDITOR.to_owned()]
        .into_iter()
        .chain((0..KEPT).map(app_id))
        .collect();
    let services = Services {
        text: &text,
        session: None,
        system: None,
        config_home: None,
        home: None,
    };
    let display = Display {
        output: "HEADLESS-1".to_owned(),
        main: true,
    };
    let bar = surfaces::open(&root, &display, &config, &services).expect("bar");
    let mut control_center = root
        .open_beside(bande_controlcenter::role(), (0, 0))
        .expect("control center");
    control_center.share_fonts(&text);
    let control =
        ControlCenter::new(&mut control_center, text.sharing_fonts(), None, None).expect("panel");
    let mut launcher = root
        .open_beside(bande_launcher::role(), (0, 0))
        .expect("launcher");
    launcher.share_fonts(&text);
    let search = LauncherShell::new(&mut launcher, text.sharing_fonts(), None).expect("launcher");
    let mut windows = vec![root];
    let mut parts: Vec<(Part, Box<dyn App>)> = vec![(Part::Notifications, Box::new(notifications))];
    for (window, app) in bar.surfaces {
        windows.push(window);
        parts.push((Part::Bar, app));
    }
    windows.extend([control_center, launcher]);
    parts.push((Part::ControlCenter, Box::new(control)));
    parts.push((Part::Launcher, Box::new(search)));
    let mut shell = Shell {
        windows,
        parts,
        events,
        targets,
        _text: text,
    };
    bande_toolkit::run_surfaces(&mut shell).expect("run loop");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Mark,
    Header,
    KeptApp,
    RunningApp,
    More,
    ControlCenter,
    Clock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Press {
    Primary,
    Secondary,
    Hold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Menu,
    Launcher,
    ControlCenter,
    NotificationCenter,
    Nothing,
}

struct Desktop {
    server: Compositor,
    renderer: Renderer,
    scheduler: FrameScheduler,
    start: Instant,
    events: Receiver<(Part, WindowEvent)>,
    seen: Vec<(Part, WindowEvent)>,
    target_updates: Receiver<Targets>,
    targets: Targets,
    client: Option<JoinHandle<()>>,
    editor: Option<TestClient>,
    time: u32,
}

impl Desktop {
    fn new() -> Self {
        install_apps();
        let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
        let (client_end, server_end) = UnixStream::pair().expect("socket pair");
        server
            .insert_session_client(server_end, SessionRole::Shell)
            .expect("insert the shell");
        let (events, received) = mpsc::channel();
        let (targets, target_updates) = mpsc::channel();
        let client = thread::spawn(move || shell(client_end, events, targets));
        let mut desktop = Self {
            server,
            renderer: Renderer::new().expect("renderer"),
            scheduler: FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH)),
            start: Instant::now(),
            events: received,
            seen: Vec::new(),
            target_updates,
            targets: Vec::new(),
            client: Some(client),
            editor: None,
            time: 0,
        };
        desktop.wait_until(|desktop| desktop.bar_width().is_some());
        desktop.settle();
        desktop.open_editor();
        desktop
    }

    fn open_editor(&mut self) {
        let mut client = TestClient::connect(&mut self.server);
        client.use_seat(&mut self.server);
        let pixels = RED.repeat((WINDOW * WINDOW) as usize);
        client.show_window(
            &mut self.server,
            &ShmImage {
                width: WINDOW,
                height: WINDOW,
                stride: WINDOW * 4,
                format: Format::Xrgb8888,
                bytes: &pixels,
            },
        );
        client.set_app_id(&mut self.server, 0, EDITOR);
        self.editor = Some(client);
        self.wait_until(|desktop| {
            desktop.seen.iter().any(
                |(_, event)| matches!(event, WindowEvent::ActiveApp(active) if active == EDITOR),
            ) && desktop.editor_has_keyboard()
        });
        self.settle();
    }

    fn bar_width(&self) -> Option<i32> {
        Some(self.server.usable_area().loc.x).filter(|x| *x > 0)
    }

    fn editor_has_keyboard(&self) -> bool {
        self.editor
            .as_ref()
            .is_some_and(|editor| editor.keyboard_focus_on(0))
    }

    fn pump(&mut self) {
        self.server.dispatch().expect("server dispatch");
        let start = self.start;
        self.scheduler
            .poll(|| start.elapsed(), &mut self.server, &mut self.renderer)
            .expect("compositor frame");
        if let Some(editor) = &mut self.editor {
            editor.exchange(&mut self.server);
        }
        self.seen.extend(self.events.try_iter());
        if let Some(targets) = self.target_updates.try_iter().last() {
            self.targets = targets;
        }
        assert!(
            self.client
                .as_ref()
                .is_some_and(|client| !client.is_finished()),
            "the shell stopped"
        );
        thread::sleep(Duration::from_micros(300));
    }

    fn wait_until(&mut self, until: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !until(self) {
            self.pump();
            assert!(
                Instant::now() < deadline,
                "timed out with {} popups after {:?}",
                self.server.popup_count(),
                self.seen
            );
        }
    }

    fn pump_for(&mut self, span: Duration) {
        let until = Instant::now() + span;
        while Instant::now() < until {
            self.pump();
        }
    }

    fn settle(&mut self) {
        self.pump_for(Duration::from_millis(450));
    }

    fn tick(&mut self) -> u32 {
        self.time += 10;
        self.time
    }

    fn point(&mut self, (x, y): (f64, f64)) {
        let time = self.tick();
        self.server.move_pointer_to(Point::from((x, y)), time);
        self.pump();
    }

    fn button(&mut self, button: u32, pressed: bool) {
        let time = self.tick();
        self.server.press_button(button, pressed, time);
        self.pump();
    }

    fn key(&mut self, key: u32) {
        let time = self.tick();
        self.server.key(key, true, time);
        self.pump();
        let time = self.tick();
        self.server.key(key, false, time);
        self.pump();
    }

    fn chord(&mut self, modifier: u32, key: u32) {
        let time = self.tick();
        self.server.key(modifier, true, time);
        self.key(key);
        let time = self.tick();
        self.server.key(modifier, false, time);
        self.settle();
    }

    fn label(&self, target: Target) -> Option<String> {
        let more = |count: i64| L10N.tr_args("more-apps-name", [("count", count.into())]);
        let wanted = match target {
            Target::Mark => L10N.tr("launcher"),
            Target::Header => L10N.tr_args("app-menus", [("app", "Editor".into())]),
            Target::KeptApp => "App 1".to_owned(),
            Target::RunningApp => "Editor".to_owned(),
            Target::ControlCenter => L10N.tr("control-center"),
            Target::More => {
                return (1..=KEPT as i64)
                    .map(more)
                    .find(|label| self.targets.iter().any(|(known, _)| known == label));
            }
            Target::Clock => {
                return self
                    .targets
                    .iter()
                    .max_by(|a, b| a.1.1.total_cmp(&b.1.1))
                    .map(|(label, _)| label.clone());
            }
        };
        Some(wanted)
    }

    fn at(&self, target: Target) -> (f64, f64) {
        let label = self.label(target);
        let mut found = self
            .targets
            .iter()
            .filter(|(known, _)| Some(known) == label.as_ref());
        let (_, (x0, y0, x1, y1)) = found
            .next()
            .unwrap_or_else(|| panic!("no {target:?} ({label:?}) in {:?}", self.targets));
        ((x0 + x1) / 2.0, (y0 + y1) / 2.0)
    }

    fn press(&mut self, target: Target, press: Press) {
        let at = self.at(target);
        self.point(at);
        match press {
            Press::Primary => {
                self.button(BTN_LEFT, true);
                self.button(BTN_LEFT, false);
            }
            Press::Secondary => {
                self.button(BTN_RIGHT, true);
                self.button(BTN_RIGHT, false);
            }
            Press::Hold => {
                self.button(BTN_LEFT, true);
                self.pump_for(HOLD);
                self.button(BTN_LEFT, false);
            }
        }
        self.settle();
    }

    fn await_outcome(&mut self, expected: Outcome) {
        let until = Instant::now() + Duration::from_secs(3);
        while self.outcome() != expected && Instant::now() < until {
            self.pump();
        }
    }

    fn focused(&self) -> Option<Part> {
        let mut holding: Vec<Part> = Vec::new();
        for (part, event) in &self.seen {
            match event {
                WindowEvent::Focus(true) => {
                    holding.retain(|known| known != part);
                    holding.push(*part);
                }
                WindowEvent::Focus(false) => holding.retain(|known| known != part),
                _ => {}
            }
        }
        holding.last().copied()
    }

    fn outcome(&self) -> Outcome {
        if self.server.popup_count() > 0 {
            return Outcome::Menu;
        }
        match self.focused() {
            Some(Part::Launcher) => Outcome::Launcher,
            Some(Part::ControlCenter) => Outcome::ControlCenter,
            Some(Part::Notifications) => Outcome::NotificationCenter,
            _ => Outcome::Nothing,
        }
    }

    fn calm(&self) -> bool {
        self.server.popup_count() == 0
            && self.editor_has_keyboard()
            && !matches!(
                self.focused(),
                Some(Part::Launcher | Part::ControlCenter | Part::Notifications)
            )
    }

    fn await_calm(&mut self) -> bool {
        let until = Instant::now() + Duration::from_secs(2);
        while !self.calm() && Instant::now() < until {
            self.pump();
        }
        self.calm()
    }

    fn reset(&mut self) -> Option<String> {
        for _ in 0..3 {
            if self.await_calm() {
                return None;
            }
            self.key(KEY_ESC);
        }
        Some(format!(
            "Escape leaves {:?} with {} popups, the editor has the keyboard: {}",
            self.outcome(),
            self.server.popup_count(),
            self.editor_has_keyboard()
        ))
    }

    fn shot(&mut self, name: &str) {
        self.renderer.render(&mut self.server).expect("frame");
        let snapshot = self.renderer.snapshot(&self.server).expect("read back");
        let image = image::RgbaImage::from_raw(
            snapshot.size.w as u32,
            snapshot.size.h as u32,
            snapshot.rgba,
        )
        .expect("frame");
        let folder = std::env::var_os("BANDE_REPORT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")));
        std::fs::create_dir_all(&folder).expect("report folder");
        image::DynamicImage::ImageRgba8(image)
            .to_rgb8()
            .save(folder.join(format!("{name}.png")))
            .expect("save the frame");
    }
}

struct Case {
    target: Target,
    press: Press,
    outcome: Outcome,
}

const fn case(target: Target, press: Press, outcome: Outcome) -> Case {
    Case {
        target,
        press,
        outcome,
    }
}

fn cases() -> Vec<Case> {
    use Outcome::{Launcher, Menu, NotificationCenter};
    use Press::*;
    use Target::*;
    vec![
        case(Mark, Primary, Launcher),
        case(Mark, Hold, Launcher),
        case(Mark, Secondary, Menu),
        case(Header, Primary, Menu),
        case(Header, Secondary, Menu),
        case(Header, Hold, Menu),
        case(KeptApp, Secondary, Menu),
        case(KeptApp, Hold, Menu),
        case(RunningApp, Secondary, Menu),
        case(RunningApp, Hold, Menu),
        case(More, Primary, Menu),
        case(More, Secondary, Menu),
        case(More, Hold, Menu),
        case(ControlCenter, Primary, Outcome::ControlCenter),
        case(ControlCenter, Secondary, Outcome::ControlCenter),
        case(ControlCenter, Hold, Outcome::ControlCenter),
        case(Clock, Primary, NotificationCenter),
        case(Clock, Secondary, Menu),
        case(Clock, Hold, NotificationCenter),
    ]
}

fn another(outcome: Outcome) -> (Target, Press, Outcome) {
    match outcome {
        Outcome::Launcher => (Target::ControlCenter, Press::Primary, Outcome::Nothing),
        Outcome::ControlCenter => (Target::Mark, Press::Secondary, Outcome::Menu),
        _ => (
            Target::ControlCenter,
            Press::Primary,
            Outcome::ControlCenter,
        ),
    }
}

fn expect(desktop: &mut Desktop, what: &str, expected: Outcome) -> Option<String> {
    desktop.await_outcome(expected);
    let got = desktop.outcome();
    if got != expected {
        return Some(format!("{what}: expected {expected:?}, got {got:?}"));
    }
    desktop.pump_for(Duration::from_millis(300));
    let later = desktop.outcome();
    (later != expected).then(|| format!("{what}: {expected:?} turned into {later:?}"))
}

fn restore(desktop: &mut Desktop, failures: &mut Vec<String>, name: &str) {
    if let Some(stuck) = desktop.reset() {
        failures.push(format!("{name}: {stuck}"));
        *desktop = Desktop::new();
    }
}

fn sweep(cases: Vec<Case>) {
    let mut desktop = Desktop::new();
    let mut failures = Vec::new();
    for Case {
        target,
        press,
        outcome,
    } in cases
    {
        let name = format!("{target:?} {press:?}");
        desktop.press(target, press);
        failures.extend(expect(&mut desktop, &name, outcome));
        if press == Press::Primary || (press == Press::Secondary && outcome == Outcome::Menu) {
            desktop.shot(&format!("{target:?}-{press:?}").to_lowercase());
        }
        if outcome == Outcome::Nothing {
            restore(&mut desktop, &mut failures, &name);
            continue;
        }
        desktop.key(KEY_ESC);
        let escaped = format!("{name}, then Escape");
        failures.extend(expect(&mut desktop, &escaped, Outcome::Nothing));
        restore(&mut desktop, &mut failures, &escaped);

        desktop.press(target, press);
        desktop.await_outcome(outcome);
        let again = if press == Press::Secondary {
            Press::Secondary
        } else {
            Press::Primary
        };
        desktop.press(target, again);
        let twice = format!("{name}, then {again:?} again");
        failures.extend(expect(&mut desktop, &twice, Outcome::Nothing));
        restore(&mut desktop, &mut failures, &twice);

        desktop.press(target, press);
        desktop.await_outcome(outcome);
        let (next, next_press, next_outcome) = another(outcome);
        desktop.press(next, next_press);
        let moved = format!("{name}, then {next:?} {next_press:?}");
        failures.extend(expect(&mut desktop, &moved, next_outcome));
        restore(&mut desktop, &mut failures, &moved);

        if outcome == Outcome::Menu {
            desktop.press(target, press);
            desktop.await_outcome(outcome);
            desktop.key(KEY_DOWN);
            desktop.key(KEY_DOWN);
            let walked = format!("{name}, then Down Down");
            failures.extend(expect(&mut desktop, &walked, Outcome::Menu));
            restore(&mut desktop, &mut failures, &walked);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_bar_target_answers_every_press() {
    sweep(cases());
}

#[test]
fn the_keyboard_opens_the_app_menus_and_walks_the_apps_and_gives_the_keyboard_back() {
    let mut desktop = Desktop::new();
    desktop.chord(KEY_LEFTMETA, KEY_F2);
    assert_eq!(desktop.outcome(), Outcome::Menu);
    desktop.shot("keyboard-app-menus");
    desktop.pump_for(Duration::from_millis(300));
    assert_eq!(desktop.outcome(), Outcome::Menu);
    desktop.key(KEY_ESC);
    desktop.settle();
    assert!(desktop.calm());
    desktop.chord(KEY_LEFTMETA, KEY_F3);
    desktop.key(KEY_DOWN);
    desktop.settle();
    assert_eq!(desktop.focused(), Some(Part::Bar));
    desktop.key(KEY_ESC);
    desktop.settle();
    assert!(desktop.calm());
}

#[test]
fn the_clock_over_control_center_closes_it() {
    let mut desktop = Desktop::new();
    desktop.press(Target::ControlCenter, Press::Primary);
    desktop.await_outcome(Outcome::ControlCenter);
    desktop.press(Target::Clock, Press::Primary);
    assert_eq!(
        expect(
            &mut desktop,
            "the clock while Control Center is open",
            Outcome::NotificationCenter
        ),
        None
    );
    desktop.key(KEY_ESC);
    assert!(desktop.await_calm(), "Control Center closed for the center");
}

#[test]
fn control_center_over_the_notification_center_closes_it() {
    let mut desktop = Desktop::new();
    desktop.press(Target::Clock, Press::Primary);
    desktop.await_outcome(Outcome::NotificationCenter);
    desktop.press(Target::ControlCenter, Press::Primary);
    assert_eq!(
        expect(
            &mut desktop,
            "Control Center while the center is open",
            Outcome::ControlCenter
        ),
        None
    );
    desktop.key(KEY_ESC);
    assert!(desktop.await_calm(), "the center closed for Control Center");
}

#[test]
fn a_bar_menu_opened_over_control_center_closes_it() {
    let mut desktop = Desktop::new();
    desktop.press(Target::ControlCenter, Press::Primary);
    desktop.await_outcome(Outcome::ControlCenter);
    desktop.press(Target::Mark, Press::Secondary);
    assert_eq!(
        expect(
            &mut desktop,
            "a bar menu while Control Center is open",
            Outcome::Menu
        ),
        None
    );
    desktop.key(KEY_ESC);
    assert!(
        desktop.await_calm(),
        "Control Center closed when the menu took the keyboard"
    );
}
