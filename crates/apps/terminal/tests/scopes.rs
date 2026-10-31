#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;
use std::time::{Duration, Instant};

use alacritty_terminal::tty::Shell;
use unbound_term::{GridSize, Session, Terminal};
use unbound_test_bus::SessionBus;
use unbound_test_bus::systemd::{FakeSystemd, StartedUnit};
use zbus::blocking::Connection;

const MESSAGE: &str = "This tab was closed because memory ran low";

fn spawn(bus: Connection) -> Session {
    Session::spawn(
        Terminal::new(GridSize {
            columns: 60,
            lines: 6,
        }),
        Some(Shell::new(
            "/bin/sh".to_owned(),
            vec!["-c".to_owned(), "printf ready; sleep 30".to_owned()],
        )),
        None,
        Some(bus),
    )
    .expect("session")
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn scope_of(systemd: &FakeSystemd, session: &Session) -> String {
    let unit = StartedUnit {
        name: format!("app-unbound-org.unbound.Terminal-tab{}.scope", session.id()),
        pids: vec![session.id()],
        slice: "app.slice".to_owned(),
        collect_mode: None,
    };
    wait_until("the tab gets its own scope", || {
        systemd.started().contains(&unit)
    });
    unit.name
}

fn kill(session: &Session) {
    let killed = Command::new("kill")
        .args(["-KILL", &session.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
}

fn screen(session: &Session) -> String {
    session.terminal.screen_text().join("\n")
}

#[test]
fn a_tab_killed_for_memory_says_so_and_stays_open() {
    let bus = SessionBus::start().unwrap();
    let systemd = FakeSystemd::start(&bus).unwrap();
    let mut session = spawn(bus.connect().unwrap());
    let unit = scope_of(&systemd, &session);
    wait_until("the shell starts", || {
        session.read();
        screen(&session).contains("ready")
    });

    systemd.fail(&unit, "oom-kill").unwrap();
    kill(&session);
    wait_until("the tab tells why it ended", || {
        session.read();
        screen(&session).contains(MESSAGE)
    });

    assert!(!session.exited());
    assert!(session.fd().is_none());
    assert!(screen(&session).contains("ready"));
    assert_eq!(systemd.reset(), [unit]);
}

#[test]
fn a_tab_whose_shell_is_killed_otherwise_closes() {
    let bus = SessionBus::start().unwrap();
    let systemd = FakeSystemd::start(&bus).unwrap();
    let mut session = spawn(bus.connect().unwrap());
    let unit = scope_of(&systemd, &session);

    systemd.end(&unit).unwrap();
    kill(&session);
    wait_until("the tab closes", || {
        session.read();
        session.exited()
    });

    assert!(!screen(&session).contains(MESSAGE));
}

#[test]
fn without_a_user_manager_tabs_still_close_when_their_shell_ends() {
    let bus = SessionBus::start().unwrap();
    let mut session = spawn(bus.connect().unwrap());

    kill(&session);
    wait_until("the tab closes", || {
        session.read();
        session.exited()
    });
}
