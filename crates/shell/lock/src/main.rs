use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;
use unbound_lock::{LockScreen, PAM_SERVICE, User, authenticate, current_user};
use unbound_toolkit::{App, Role, Scene, TextSystem, Window, WindowError, WindowEvent};

const CLOCK_TICK: Duration = Duration::from_secs(1);
const ACCEPTED: u8 = 1;
const REJECTED: u8 = 0;

struct Locker {
    screen: LockScreen,
    user: User,
    verdicts: UnixStream,
    verdict_sender: UnixStream,
    next_clock: Instant,
    unlocked: bool,
}

impl Locker {
    fn check(&mut self, password: String) {
        let Ok(mut sender) = self.verdict_sender.try_clone() else {
            self.screen.reject(Instant::now());
            return;
        };
        let login = self.user.login.clone();
        thread::spawn(move || {
            let verdict = match authenticate(PAM_SERVICE, &login, &password) {
                Ok(()) => ACCEPTED,
                Err(error) => {
                    tracing::warn!("{error}");
                    REJECTED
                }
            };
            let _ = sender.write_all(&[verdict]);
        });
    }
}

impl App for Locker {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.screen.advance(Instant::now());
        self.screen.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let changed = match event {
            WindowEvent::Input(input) => self.screen.handle(&input),
            WindowEvent::Accessibility(request) => self.screen.perform(&request),
            WindowEvent::AppearanceChanged(appearance) => {
                self.screen.set_palette(appearance.palette());
                false
            }
            _ => false,
        };
        if changed {
            window.request_redraw();
            if let Some(password) = self.screen.take_password() {
                self.check(password);
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
        if verdict[0] == ACCEPTED {
            match window.unlock() {
                Ok(()) => self.unlocked = true,
                Err(error) => tracing::warn!("cannot unlock: {error}"),
            }
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
        self.unlocked
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.screen.accessibility())
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let user = current_user()?;
    let (verdicts, verdict_sender) = UnixStream::pair()?;
    let mut window = Window::open(Role::Lock, (0, 0))?;
    let mut locker = Locker {
        screen: LockScreen::new(
            &user,
            &jiff::Zoned::now(),
            window.appearance().palette(),
            TextSystem::new(),
        ),
        user,
        verdicts,
        verdict_sender,
        next_clock: Instant::now() + CLOCK_TICK,
        unlocked: false,
    };
    unbound_toolkit::run(&mut window, &mut locker).map_err(|error: WindowError| error.into())
}

fn main() -> ExitCode {
    unbound_log::init("unbound-lock");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
