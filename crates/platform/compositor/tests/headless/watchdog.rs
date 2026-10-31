use std::time::Duration;

use crate::support::*;
use bande_compositor::{Compositor, Renderer, UNRESPONSIVE_AFTER};
use bande_test_wayland::{RED, TestClient, roundtrip};
use smithay::utils::Size;

const WINDOW: i32 = 64;
const CENTER: (f64, f64) = (128.0, 96.0);
const SAMPLE: (i32, i32) = (108, 76);
const APP_ID: &str = "org.example.Stuck";

struct Scene {
    server: Compositor,
    renderer: Renderer,
    app: TestClient,
    shell: TestClient,
}

fn scene() -> Scene {
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let app = window_of(&mut server, RED, (WINDOW, WINDOW), APP_ID);
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.join_shell(&mut server);
    Scene {
        server,
        renderer: Renderer::new().expect("renderer"),
        app,
        shell,
    }
}

impl Scene {
    fn interact(&mut self, at: Duration) -> Option<Duration> {
        self.server.move_pointer_to(CENTER.into(), 1);
        let due = self.server.watchdog_tick(at);
        roundtrip(&mut self.server, &mut [&mut self.app, &mut self.shell]);
        due
    }

    fn tick(&mut self, at: Duration) {
        self.server.watchdog_tick(at);
        roundtrip(&mut self.server, &mut [&mut self.app, &mut self.shell]);
    }

    fn window_pixel(&mut self) -> [u8; 4] {
        self.renderer.render(&mut self.server).expect("frame");
        let snapshot = self.renderer.snapshot(&self.server).expect("read back");
        let at = ((SAMPLE.1 * snapshot.size.w + SAMPLE.0) * 4) as usize;
        snapshot.rgba[at..at + 4].try_into().unwrap()
    }
}

#[test]
fn an_app_that_stops_answering_is_reported_veiled_and_released_when_it_answers() {
    let mut scene = scene();
    let start = Duration::from_secs(10);
    let red = scene.window_pixel();
    scene.app.hang();

    let due = scene.interact(start);
    scene.tick(start + UNRESPONSIVE_AFTER - Duration::from_millis(1));
    let still_trusted = scene.shell.app_responding().to_vec();
    scene.tick(start + UNRESPONSIVE_AFTER);
    let veiled = scene.window_pixel();
    scene.app.recover(&mut scene.server);
    scene.shell.exchange(&mut scene.server);
    let clear = scene.window_pixel();

    let pid = std::process::id() as i32;
    assert_eq!(due, Some(start + UNRESPONSIVE_AFTER));
    assert!(still_trusted.is_empty(), "{still_trusted:?}");
    assert_eq!(
        scene.shell.app_responding(),
        [
            (pid, APP_ID.to_owned(), false),
            (pid, APP_ID.to_owned(), true)
        ]
    );
    assert!(veiled[0] < red[0], "{red:?} {veiled:?}");
    assert_eq!(clear, red);
}

#[test]
fn an_app_that_answers_is_never_reported() {
    let mut scene = scene();
    let start = Duration::from_secs(10);

    scene.interact(start);
    scene.tick(start + UNRESPONSIVE_AFTER * 2);

    assert!(scene.shell.app_responding().is_empty());
}

#[test]
fn a_shell_that_binds_later_learns_which_apps_are_not_responding() {
    let mut scene = scene();
    let start = Duration::from_secs(10);
    scene.app.hang();
    scene.interact(start);
    scene.tick(start + UNRESPONSIVE_AFTER);

    let mut late = TestClient::connect_as_shell(&mut scene.server);
    late.join_shell(&mut scene.server);

    assert_eq!(
        late.app_responding(),
        [(std::process::id() as i32, APP_ID.to_owned(), false)]
    );
}

#[test]
fn an_ordinary_app_watching_apps_learns_who_stops_and_starts_answering() {
    let mut scene = scene();
    let start = Duration::from_secs(10);
    let mut monitor = TestClient::connect(&mut scene.server);
    scene.app.hang();
    scene.interact(start);
    scene.tick(start + UNRESPONSIVE_AFTER);

    monitor.join_app_control(&mut scene.server);
    scene.app.recover(&mut scene.server);
    monitor.exchange(&mut scene.server);

    let pid = std::process::id() as i32;
    assert_eq!(
        monitor.app_responding(),
        [
            (pid, APP_ID.to_owned(), false),
            (pid, APP_ID.to_owned(), true)
        ]
    );
}
