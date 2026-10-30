use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;
use unbound_datetime::DateFormats;
use unbound_keyboard::Layouts;
use unbound_lock::{
    BackdropSource, Inbox, LockScreen, PAM_SERVICE, User, Variant, authenticate, current_user,
    load_backdrop,
};
use unbound_toolkit::{
    App, Image, Role, Scene, Scheme, TextSystem, Window, WindowError, WindowEvent,
};

const CLOCK_TICK: Duration = Duration::from_secs(1);
const ACCEPTED: u8 = 1;
const REJECTED: u8 = 0;

fn variant_of(scheme: Scheme) -> Variant {
    match scheme {
        Scheme::Light => Variant::Light,
        Scheme::Dark => Variant::Dark,
    }
}

struct Locker {
    screen: LockScreen,
    user: User,
    verdicts: UnixStream,
    verdict_sender: UnixStream,
    next_clock: Instant,
    backdrop: Option<Inbox<Image>>,
    layouts: Layouts,
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
        if self.backdrop.is_none() && width > 0 && height > 0 {
            match load_backdrop(
                BackdropSource::Session,
                variant_of(window.appearance().scheme),
                (width, height),
            ) {
                Ok(inbox) => self.backdrop = Some(inbox),
                Err(error) => tracing::warn!("cannot load the wallpaper: {error}"),
            }
        }
        self.screen.advance(Instant::now());
        self.screen.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let changed = match event {
            WindowEvent::Input(input) => self.screen.handle(&input),
            WindowEvent::Accessibility(request) => self.screen.perform(&request),
            WindowEvent::AppearanceChanged(appearance) => {
                self.screen.set_appearance(appearance);
                false
            }
            WindowEvent::CapsLock(on) => self.screen.set_caps_lock(on),
            WindowEvent::InputSources { sources, active } => {
                let name = sources
                    .get(active)
                    .filter(|_| sources.len() > 1)
                    .map(|code| self.layouts.name(code));
                self.screen.set_input_source(name)
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
        let mut sources = vec![self.verdicts.as_fd()];
        sources.extend(self.backdrop.as_ref().map(Inbox::fd));
        sources
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        if index == 1 {
            if let Some(image) = self.backdrop.as_ref().and_then(|inbox| inbox.drain().pop()) {
                self.screen.set_backdrop(image, Instant::now());
                window.request_redraw();
            }
            return;
        }
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
        let now = Instant::now();
        let fading = self
            .screen
            .is_fading(now)
            .then(|| now + Duration::from_millis(8));
        [Some(self.next_clock), self.screen.shaking_until(), fading]
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

fn clock_format() -> zbus::Result<String> {
    let session = zbus::blocking::Connection::session()?;
    zbus::blocking::Proxy::new(
        &session,
        "org.unbound.Config",
        "/org/unbound/Config",
        "org.unbound.Config",
    )?
    .get_property("ClockFormat")
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let user = current_user()?;
    let (verdicts, verdict_sender) = UnixStream::pair()?;
    let mut window = Window::open(Role::Lock, (0, 0))?;
    let mut locker = Locker {
        screen: LockScreen::new(
            &user,
            DateFormats::from_env().with_clock_format(
                &clock_format()
                    .inspect_err(|error| tracing::warn!("no clock format: {error}"))
                    .unwrap_or_default(),
            ),
            &jiff::Zoned::now(),
            window.appearance(),
            TextSystem::new(),
        ),
        user,
        verdicts,
        verdict_sender,
        next_clock: Instant::now() + CLOCK_TICK,
        backdrop: None,
        layouts: Layouts::load(),
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
