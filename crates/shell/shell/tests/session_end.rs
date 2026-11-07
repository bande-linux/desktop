#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::time::{Duration, Instant};

use bande_compositor::Compositor;
use bande_protocols::client::bande_shell_v1::SessionEnd;
use bande_test_bus::SessionBus;
use bande_test_bus::drives::{FakeLogind, FakeSystemd};
use bande_test_wayland::keys::{KEY_ENTER, KEY_ESC};
use bande_test_wayland::{Frames, SeparateApp, TestClient, act_as_separate_app, chord, headless};

const DIALOG: &str = "org.bande.MenuBar";
const PATIENCE: Duration = Duration::from_secs(40);

struct Scene {
    server: Compositor,
    frames: Frames,
    shell: TestClient,
    started: Instant,
}

impl Scene {
    fn open() -> Self {
        let mut server = headless(900, 900);
        server.set_session_dialog(Path::new(env!("CARGO_BIN_EXE_bande-shell")));
        let mut shell = TestClient::connect_as_shell(&mut server);
        shell.listen_as_shell(&mut server);
        Self {
            server,
            frames: Frames::start(),
            shell,
            started: Instant::now(),
        }
    }

    fn turn(&mut self) {
        self.server.keep_session_clients_running();
        self.server.watchdog_tick(self.started.elapsed());
        let mut first = true;
        self.frames
            .run_until(&mut self.server, || !std::mem::take(&mut first));
    }

    fn wait(&mut self, what: &str, mut done: impl FnMut(&Compositor) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !done(&self.server) {
            assert!(Instant::now() < deadline, "waited in vain: {what}");
            self.turn();
        }
    }

    fn pass(&mut self, time: Duration) {
        let until = Instant::now() + time;
        while Instant::now() < until {
            self.turn();
        }
    }

    fn dialog_shown(server: &Compositor) -> bool {
        server
            .windows()
            .iter()
            .any(|window| window.app_id() == DIALOG && window.size().is_some())
    }

    fn confirm(&mut self, ending: SessionEnd) {
        self.shell.confirm_session_end(&mut self.server, ending);
        self.wait("the confirmation opens", Self::dialog_shown);
        self.pass(Duration::from_millis(200));
        chord(&mut self.server, &[KEY_ENTER]);
    }
}

#[test]
fn a_confirmed_ending_goes_through_and_something_in_the_way_is_shown() {
    if act_as_separate_app() {
        return;
    }
    let state = tempfile::tempdir().expect("state dir");
    let bus = SessionBus::start().expect("private bus");
    unsafe {
        std::env::set_var("DBUS_SYSTEM_BUS_ADDRESS", bus.address());
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
        std::env::set_var("XDG_STATE_HOME", state.path());
    }
    let logind = FakeLogind::start(&bus).expect("fake logind");
    let systemd = FakeSystemd::start(&bus).expect("fake systemd");

    let mut scene = Scene::open();
    scene.confirm(SessionEnd::ShutDown);
    scene.wait("the computer is asked to power off", |_| {
        logind.ended() == ["poweroff"]
    });
    assert!(
        Scene::dialog_shown(&scene.server),
        "the dialog stays while the computer goes down"
    );
    drop(scene);

    let mut scene = Scene::open();
    let mut stuck = SeparateApp::launch(
        &mut scene.server,
        "a_confirmed_ending_goes_through_and_something_in_the_way_is_shown",
    );
    stuck.show_window_of(&mut scene.server, 64, "org.example.Stuck");
    scene.confirm(SessionEnd::Restart);
    scene.wait("a hung app does not stop the restart", |_| {
        logind.ended() == ["poweroff", "reboot"]
    });
    assert!(
        scene
            .server
            .windows()
            .iter()
            .any(|window| window.app_id() == "org.example.Stuck")
    );
    drop(scene);
    drop(stuck);

    logind.inhibit("Software Update", "Installing updates");
    let mut scene = Scene::open();
    scene.confirm(SessionEnd::ShutDown);
    scene.pass(Duration::from_secs(2));
    assert_eq!(logind.ended(), ["poweroff", "reboot"]);
    assert!(
        Scene::dialog_shown(&scene.server),
        "the dialog says what is in the way"
    );
    chord(&mut scene.server, &[KEY_ESC]);
    scene.wait("Escape dismisses it", |server| !Scene::dialog_shown(server));
    assert_eq!(logind.ended(), ["poweroff", "reboot"]);
    drop(scene);

    let mut scene = Scene::open();
    scene.confirm(SessionEnd::LogOut);
    scene.wait("Log Out stops the compositor's service", |_| {
        systemd.stopped() == ["bande-compositor.service"]
    });
}
