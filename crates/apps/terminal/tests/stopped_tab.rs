#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::net::UnixListener;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use bande_terminal::{GridSize, Profile, Session, Terminal, Theme};
use bande_toolkit::Appearance;

const TIMEOUT: Duration = Duration::from_secs(20);
const START: Duration = Duration::from_millis(3500);
const CLOSE: Duration = Duration::from_secs(1);

fn read_until(
    session: &mut Session,
    theme: &Theme,
    limit: Duration,
    what: &str,
    done: impl Fn(&Session) -> bool,
) {
    let started = Instant::now();
    while !done(session) {
        assert!(started.elapsed() < limit, "{what}");
        session.read(theme);
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_tab_closes_at_once_and_starts_beside_a_silent_bus_and_closes_when_its_shell_is_killed() {
    let folder = tempfile::tempdir().expect("a folder");
    let socket = folder.path().join("bus");
    let _silent = UnixListener::bind(&socket).expect("a bus that never answers");
    // SAFETY: the only test of this program sets the variable before it starts a thread.
    unsafe {
        std::env::set_var(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}", socket.display()),
        );
    }
    let grid = GridSize {
        columns: 80,
        lines: 24,
    };
    let theme = Theme::new(Appearance::default(), Profile::default());
    let shell = ("/bin/sleep".to_owned(), vec!["60".to_owned()]);
    let mut unplaced =
        Session::spawn(Terminal::new(grid), Some(shell.clone()), None).expect("a shell");
    let shell_status = format!("/proc/{}/stat", unplaced.id());
    let stopped =
        || std::fs::read_to_string(&shell_status).is_ok_and(|status| status.contains(") T "));
    read_until(
        &mut unplaced,
        &theme,
        START,
        "the shell never waited for its scope",
        |_| stopped(),
    );
    let closing = Instant::now();
    drop(unplaced);
    assert!(closing.elapsed() < CLOSE, "closing a waiting tab blocked");

    let mut session = Session::spawn(Terminal::new(grid), Some(shell), None).expect("a shell");
    let shell = session.id();
    let started = || {
        std::fs::read_to_string(format!("/proc/{shell}/comm")).is_ok_and(|name| name == "sleep\n")
    };
    read_until(
        &mut session,
        &theme,
        START,
        "the shell was left stopped",
        |_| started(),
    );

    // SAFETY: no other thread reads or writes variables: the tab's thread has started the shell.
    unsafe { std::env::set_var("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent") };
    let killed = Command::new("kill")
        .args(["-KILL", &shell.to_string()])
        .status();
    assert!(killed.expect("kill").success());
    read_until(
        &mut session,
        &theme,
        TIMEOUT,
        "the tab stayed open",
        |session| session.exited(),
    );
    assert!(
        !session.terminal.all_text().contains("memory"),
        "a shell killed by hand was blamed on memory"
    );
}
