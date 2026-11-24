#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use bande_terminal::{GridSize, Profile, Session, Terminal, Theme};
use bande_toolkit::Appearance;

const TIMEOUT: Duration = Duration::from_secs(20);

fn read_until(session: &mut Session, theme: &Theme, what: &str, done: impl Fn(&Session) -> bool) {
    let started = Instant::now();
    while !done(session) {
        assert!(started.elapsed() < TIMEOUT, "{what}");
        session.read(theme);
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_tab_whose_programs_were_killed_stays_open_until_return() {
    let grid = GridSize {
        columns: 80,
        lines: 24,
    };
    let theme = Theme::new(Appearance::default(), Profile::default());
    let shell = ("/bin/sleep".to_owned(), vec!["60".to_owned()]);
    let mut session = Session::spawn(Terminal::new(grid), Some(shell), None).expect("a shell");
    let shell = session.id();
    let started = || {
        std::fs::read_to_string(format!("/proc/{shell}/comm")).is_ok_and(|name| name == "sleep\n")
    };
    read_until(&mut session, &theme, "the shell was left stopped", |_| {
        started()
    });

    let killed = Command::new("kill")
        .args(["-KILL", &shell.to_string()])
        .status();
    assert!(killed.expect("kill").success());
    read_until(&mut session, &theme, "no notice in the tab", |session| {
        session.terminal.all_text().contains("memory ran low")
    });
    assert!(!session.exited(), "the tab closed without telling why");

    session.send(b"a");
    assert!(!session.exited(), "any key closed the tab");
    session.send(b"\r");
    assert!(session.exited(), "Return did not close the tab");
}
