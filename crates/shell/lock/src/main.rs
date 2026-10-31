use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::sync::mpsc::Sender;
use std::thread;

use accesskit::TreeUpdate;
use unbound_config::client::ConfigProxy;
use unbound_datetime::{DateFormats, MinuteTimer};
use unbound_keyboard::Layouts;
use unbound_lock::{
    BackdropSource, Inbox, LockScreen, Look, PAM_SERVICE, User, Variant, authenticate,
    current_user, load_backdrop,
};
use unbound_status::Update;
use unbound_status::devices::{Control, follow_devices};
use unbound_toolkit::ui::frame_time;
use unbound_toolkit::{
    App, Image, Role, Scene, Scheme, TextSystem, Window, WindowError, WindowEvent,
};
use zeroize::Zeroizing;

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
    minutes: MinuteTimer,
    backdrop: Option<Inbox<(Look, Image)>>,
    backdrop_requested: bool,
    devices: Option<(Inbox<Update>, Sender<Control>)>,
    layouts: Layouts,
    unlocked: bool,
}

impl Locker {
    fn check(&mut self, password: Zeroizing<String>) {
        let Ok(mut sender) = self.verdict_sender.try_clone() else {
            self.screen.reject(frame_time().on_screen());
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
        if !self.backdrop_requested && width > 0 && height > 0 {
            self.backdrop_requested = true;
            match load_backdrop(
                BackdropSource::Session,
                variant_of(window.appearance().scheme),
                (width, height),
                vec![Look::Blurred],
            ) {
                Ok(inbox) => self.backdrop = Some(inbox),
                Err(error) => tracing::warn!("cannot load the wallpaper: {error}"),
            }
        }
        self.screen.advance(frame_time().presentation);
        self.screen.set_scale(window.scale());
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
        let mut sources = vec![self.verdicts.as_fd(), self.minutes.as_fd()];
        sources.extend(self.devices.as_ref().map(|(inbox, _)| inbox.fd()));
        sources.extend(self.backdrop.as_ref().map(Inbox::fd));
        sources
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        if index == 1 {
            if let Err(error) = self.minutes.wait() {
                tracing::warn!("cannot follow the clock: {error}");
            }
            self.screen.set_clock(&jiff::Zoned::now());
            window.request_redraw();
            return;
        }
        if index == 2
            && let Some((inbox, _)) = self.devices.as_mut()
        {
            let latest = inbox
                .drain()
                .into_iter()
                .rev()
                .find_map(|update| match update {
                    Update::Devices((battery, wifi, _)) => Some((battery, wifi)),
                    _ => None,
                });
            if let Some((battery, wifi)) = latest
                && self.screen.set_devices(battery, wifi.as_ref())
            {
                window.request_redraw();
            }
            return;
        }
        if index >= 2 {
            if let Some((_, image)) = self
                .backdrop
                .take()
                .and_then(|mut inbox| inbox.drain().pop())
            {
                self.screen.set_backdrop(image, frame_time().on_screen());
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
            self.screen.reject(frame_time().on_screen());
            window.request_redraw();
        }
    }

    fn animating(&self) -> bool {
        self.screen.is_animating()
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
    ConfigProxy::builder(&session)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()?
        .clock_format()
}

fn follow_status() -> Option<(Inbox<Update>, Sender<Control>)> {
    let system = zbus::blocking::Connection::system()
        .inspect_err(|error| tracing::warn!("no system bus: {error}"))
        .ok()?;
    let inbox = Inbox::new()
        .inspect_err(|error| tracing::warn!("cannot follow the battery and Wi-Fi: {error}"))
        .ok()?;
    let controls = follow_devices(&system, inbox.outbox());
    Some((inbox, controls))
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
        minutes: MinuteTimer::new()?,
        backdrop: None,
        backdrop_requested: false,
        devices: follow_status(),
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
