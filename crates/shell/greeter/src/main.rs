use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;
use unbound_greeter::{Outcome, log_in, login_users};
use unbound_lock::{LockScreen, User};
use unbound_toolkit::ui::{Event, Key};
use unbound_toolkit::{
    App, Edges, KeyboardFocus, LayerKind, LayerOptions, Palette, Role, Scene, Scheme, TextSystem,
    Window, WindowEvent,
};

const CLOCK_TICK: Duration = Duration::from_secs(1);
const SESSION: &str = "unbound-compositor";
const STARTED: u8 = 1;
const REJECTED: u8 = 0;

struct Greeter {
    users: Vec<User>,
    user: usize,
    screen: LockScreen,
    verdicts: UnixStream,
    verdict_sender: UnixStream,
    next_clock: Instant,
    started: bool,
}

impl Greeter {
    fn screen_for(user: &User) -> LockScreen {
        LockScreen::new(
            user,
            &jiff::Zoned::now(),
            Palette::for_scheme(Scheme::Dark),
            TextSystem::new(),
        )
    }

    fn switch_user(&mut self, forward: bool) -> bool {
        let count = self.users.len();
        if count < 2 {
            return false;
        }
        self.user = if forward {
            (self.user + 1) % count
        } else {
            (self.user + count - 1) % count
        };
        self.screen = Self::screen_for(&self.users[self.user]);
        true
    }

    fn sign_in(&mut self, password: String) {
        let Ok(mut sender) = self.verdict_sender.try_clone() else {
            self.screen.reject(Instant::now());
            return;
        };
        let login = self.users[self.user].login.clone();
        thread::spawn(move || {
            let outcome = std::env::var_os("GREETD_SOCK")
                .ok_or_else(|| std::io::Error::other("GREETD_SOCK is not set"))
                .and_then(UnixStream::connect)
                .and_then(|mut greetd| {
                    log_in(&mut greetd, &login, password, vec![SESSION.to_owned()])
                });
            let verdict = match outcome {
                Ok(Outcome::Started) => STARTED,
                Ok(Outcome::Rejected(reason)) => {
                    eprintln!("unbound-greeter: {login} not signed in: {reason}");
                    REJECTED
                }
                Err(error) => {
                    eprintln!("unbound-greeter: greetd: {error}");
                    REJECTED
                }
            };
            let _ = sender.write_all(&[verdict]);
        });
    }
}

impl App for Greeter {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.screen.advance(Instant::now());
        self.screen.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let changed = match event {
            WindowEvent::Input(Event::Key {
                key: key @ (Key::Up | Key::Down),
                ..
            }) => self.switch_user(key == Key::Down),
            WindowEvent::Input(input) => self.screen.handle(&input),
            WindowEvent::Accessibility(request) => self.screen.perform(&request),
            _ => false,
        };
        if changed {
            window.request_redraw();
            if let Some(password) = self.screen.take_password() {
                self.sign_in(password);
            }
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.verdicts.as_fd()]
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) {
        let mut verdict = [REJECTED];
        if self.verdicts.read_exact(&mut verdict).is_err() {
            return;
        }
        if verdict[0] == STARTED {
            self.started = true;
        } else {
            self.screen.reject(Instant::now());
            window.request_redraw();
        }
    }

    fn deadline(&self) -> Option<Instant> {
        [Some(self.next_clock), self.screen.shaking_until()]
            .into_iter()
            .flatten()
            .min()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        if now >= self.next_clock {
            self.screen.set_clock(&jiff::Zoned::now());
            self.next_clock = now + CLOCK_TICK;
        }
        window.request_redraw();
    }

    fn finished(&self) -> bool {
        self.started
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.screen.accessibility())
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let users = login_users(&std::fs::read_to_string("/etc/passwd")?);
    let first = users.first().ok_or("no one can sign in")?;
    let screen = Greeter::screen_for(first);
    let (verdicts, verdict_sender) = UnixStream::pair()?;
    let mut window = Window::open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Overlay,
            anchor: Edges::ALL,
            exclusive_zone: -1,
            keyboard: KeyboardFocus::Exclusive,
            namespace: "greeter".to_owned(),
        }),
        (0, 0),
    )?;
    let mut greeter = Greeter {
        users,
        user: 0,
        screen,
        verdicts,
        verdict_sender,
        next_clock: Instant::now() + CLOCK_TICK,
        started: false,
    };
    unbound_toolkit::run(&mut window, &mut greeter)?;
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("unbound-greeter: {error}");
            ExitCode::FAILURE
        }
    }
}
