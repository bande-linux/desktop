use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;
use unbound_greeter::{
    Outcome, last_user_file, log_in, login_picture, login_users, remember_user, remembered_user,
};
use unbound_keyboard::Layouts;
use unbound_lock::{BackdropSource, DateFormats, Inbox, LockScreen, User, Variant, load_backdrop};
use unbound_toolkit::ui::{Event, Key};
use unbound_toolkit::{
    App, Edges, Image, KeyboardFocus, LayerKind, LayerOptions, Palette, Role, Scene, Scheme,
    TextSystem, Window, WindowEvent,
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
    backdrop: Option<Inbox<Image>>,
    picture: Option<Option<PathBuf>>,
    wallpaper: Option<Image>,
    last_user: Option<PathBuf>,
    layouts: Layouts,
    input_source: Option<String>,
    started: bool,
}

impl Greeter {
    fn screen_for(user: &User) -> LockScreen {
        LockScreen::new(
            user,
            DateFormats::from_env(),
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
        self.screen.set_input_source(self.input_source.clone());
        let picture = login_picture(&self.users[self.user].login);
        if self.picture.as_ref() != Some(&picture) {
            self.picture = Some(picture);
            self.backdrop = None;
        }
        if let Some(wallpaper) = &self.wallpaper {
            self.screen
                .set_backdrop(wallpaper.clone(), Instant::now() - Duration::from_secs(1));
        }
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
                    tracing::info!("{login} not signed in: {reason}");
                    REJECTED
                }
                Err(error) => {
                    tracing::warn!("greetd: {error}");
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
        if self.backdrop.is_none() && width > 0 && height > 0 {
            let picture = self.picture.clone().flatten();
            match load_backdrop(
                BackdropSource::Picture(picture),
                Variant::Dark,
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
            WindowEvent::Input(Event::Key {
                key: key @ (Key::Up | Key::Down),
                ..
            }) => self.switch_user(key == Key::Down),
            WindowEvent::Input(input) => self.screen.handle(&input),
            WindowEvent::Accessibility(request) => self.screen.perform(&request),
            WindowEvent::InputSources { sources, active } => {
                self.input_source = sources
                    .get(active)
                    .filter(|_| sources.len() > 1)
                    .map(|code| self.layouts.name(code));
                self.screen.set_input_source(self.input_source.clone())
            }
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
        let mut sources = vec![self.verdicts.as_fd()];
        sources.extend(self.backdrop.as_ref().map(Inbox::fd));
        sources
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        if index == 1 {
            if let Some(image) = self.backdrop.as_ref().and_then(|inbox| inbox.drain().pop()) {
                self.screen.set_backdrop(image.clone(), Instant::now());
                self.wallpaper = Some(image);
                window.request_redraw();
            }
            return;
        }
        let mut verdict = [REJECTED];
        if self.verdicts.read_exact(&mut verdict).is_err() {
            return;
        }
        if verdict[0] == STARTED {
            self.started = true;
            if let Some(file) = &self.last_user
                && let Err(error) = remember_user(file, &self.users[self.user].login)
            {
                tracing::info!("cannot remember who signed in last: {error}");
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
        self.started
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.screen.accessibility())
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let users = login_users(&std::fs::read_to_string("/etc/passwd")?);
    let last_user = last_user_file(
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .as_deref(),
        std::env::home_dir().as_deref(),
    );
    let remembered = last_user.as_deref().and_then(remembered_user);
    let user = users
        .iter()
        .position(|user| Some(&user.login) == remembered.as_ref())
        .unwrap_or(0);
    let chosen = users.get(user).ok_or("no one can sign in")?;
    let screen = Greeter::screen_for(chosen);
    let picture = login_picture(&chosen.login);
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
        user,
        screen,
        verdicts,
        verdict_sender,
        next_clock: Instant::now() + CLOCK_TICK,
        backdrop: None,
        picture: Some(picture),
        wallpaper: None,
        last_user,
        layouts: Layouts::load(),
        input_source: None,
        started: false,
    };
    unbound_toolkit::run(&mut window, &mut greeter)?;
    Ok(())
}

fn main() -> ExitCode {
    unbound_log::init("unbound-greeter");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
