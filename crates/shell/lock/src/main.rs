use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::thread;
use std::time::Instant;

use accesskit::TreeUpdate;
use bande_bus::logind::ManagerProxyBlocking;
use bande_datetime::{DateFormats, MinuteTimer};
use bande_keyboard::Layouts;
use bande_lock::{
    BackdropSource, Devices, Inbox, LockScreen, Look, PAM_SERVICE, PICTURE_PIXELS, User, Variant,
    authenticate, current_user, is_deliberate, load_backdrop, load_picture,
};
use bande_toolkit::ui::frame_time;
use bande_toolkit::{
    App, Image, Role, Scene, Scheme, TextSystem, Window, WindowError, WindowEvent,
};
use rustix::process::{DumpableBehavior, set_dumpable_behavior};
use zeroize::Zeroizing;

const ACCEPTED: u8 = 1;
const REJECTED: u8 = 0;

fn variant_of(scheme: Scheme) -> Variant {
    match scheme {
        Scheme::Light => Variant::Light,
        Scheme::Dark => Variant::Dark,
    }
}

#[zbus::proxy(
    interface = "org.bande.Power",
    default_service = "org.bande.Power",
    default_path = "/org/bande/Power"
)]
trait Power {
    #[zbus(property)]
    fn password_required(&self) -> zbus::Result<bool>;
}

struct Locker {
    screen: LockScreen,
    user: User,
    verdicts: UnixStream,
    verdict_sender: UnixStream,
    minutes: MinuteTimer,
    backdrop: Option<Inbox<(Look, Image)>>,
    backdrop_requested: bool,
    picture: Option<Inbox<Image>>,
    devices: Option<Devices>,
    layouts: Layouts,
    wakes: Option<Inbox<()>>,
    power: Option<PowerProxyBlocking<'static>>,
    unlocked: bool,
}

impl Locker {
    fn password_required(&self) -> bool {
        self.power
            .as_ref()
            .and_then(|power| power.cached_password_required().ok().flatten())
            .unwrap_or(true)
    }

    fn unlock(&mut self, window: &mut Window) {
        match window.unlock() {
            Ok(()) => self.unlocked = true,
            Err(error) => tracing::warn!("cannot unlock: {error}"),
        }
    }

    fn check(&mut self, password: Zeroizing<String>) {
        let Ok(mut sender) = self.verdict_sender.try_clone() else {
            self.screen.reject_password(frame_time().on_screen());
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
        if let WindowEvent::Input(input) = &event
            && is_deliberate(input)
        {
            if !self.password_required() {
                self.unlock(window);
                return;
            }
            if !self.screen.wake_on(input, Instant::now()) {
                window.request_redraw();
                return;
            }
        }
        let changed = match event {
            WindowEvent::Input(input) => self.screen.handle(&input),
            WindowEvent::Accessibility(request) => {
                self.screen.wake(Instant::now());
                window.request_redraw();
                self.screen.perform(&request)
            }
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
        sources.extend(self.devices.as_ref().map(Devices::fd));
        sources.extend(self.backdrop.as_ref().map(Inbox::fd));
        sources.extend(self.picture.as_ref().map(Inbox::fd));
        sources.extend(self.wakes.as_ref().map(Inbox::fd));
        sources
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        if index == 1 {
            if let Err(error) = self.minutes.wait() {
                tracing::warn!("cannot follow the clock: {error}");
            }
            self.screen.set_clock(&bande_datetime::now());
            window.request_redraw();
            return;
        }
        let backdrop_at = 2 + usize::from(self.devices.is_some());
        let picture_at = backdrop_at + usize::from(self.backdrop.is_some());
        if index == 2
            && let Some(devices) = self.devices.as_mut()
        {
            if let Some((battery, wifi)) = devices.latest()
                && self.screen.set_devices(battery, wifi.as_ref())
            {
                window.request_redraw();
            }
            return;
        }
        if index == backdrop_at && self.backdrop.is_some() {
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
        let wakes_at = picture_at + usize::from(self.picture.is_some());
        if index == wakes_at
            && let Some(wakes) = self.wakes.as_mut()
        {
            wakes.drain();
            self.screen.wake(Instant::now());
            window.request_redraw();
            return;
        }
        if index == picture_at {
            if let Some(picture) = self
                .picture
                .take()
                .and_then(|mut inbox| inbox.drain().pop())
            {
                self.screen.set_picture(picture);
                window.request_redraw();
            }
            return;
        }
        let mut verdict = [REJECTED];
        if self.verdicts.read_exact(&mut verdict).is_err() {
            return;
        }
        if verdict[0] == ACCEPTED {
            self.unlock(window);
        } else {
            self.screen.reject_password(frame_time().on_screen());
            window.request_redraw();
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.screen.rests_at()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        if self.screen.rest_when_due(now) {
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

fn follow_wakes(system: &zbus::blocking::Connection) -> zbus::Result<Inbox<()>> {
    let inbox = Inbox::new().map_err(|error| zbus::Error::InputOutput(error.into()))?;
    let outbox = inbox.outbox();
    let signals = ManagerProxyBlocking::new(system)?.receive_prepare_for_sleep()?;
    thread::spawn(move || {
        for signal in signals {
            if signal.args().is_ok_and(|arguments| !*arguments.start()) && !outbox.send(()) {
                return;
            }
        }
    });
    Ok(inbox)
}

fn config_text(property: &str) -> String {
    bande_bus::session()
        .and_then(|session| {
            zbus::blocking::Proxy::new(
                &session,
                "org.bande.Config",
                "/org/bande/Config",
                "org.bande.Config",
            )?
            .get_property(property)
        })
        .inspect_err(|error| tracing::warn!("no {property}: {error}"))
        .unwrap_or_default()
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    set_dumpable_behavior(DumpableBehavior::NotDumpable)?;
    let user = current_user()?;
    let picture = load_picture(user.login.clone(), PICTURE_PIXELS)
        .inspect_err(|error| tracing::warn!("cannot load the account picture: {error}"))
        .ok();
    let (verdicts, verdict_sender) = UnixStream::pair()?;
    let mut window = Window::open(Role::Lock, (0, 0))?;
    let system = bande_bus::system()
        .inspect_err(|error| tracing::warn!("no system bus: {error}"))
        .ok();
    let mut locker = Locker {
        screen: LockScreen::new(
            &user,
            DateFormats::from_env().with_clock_format(&config_text("ClockFormat")),
            &bande_datetime::now(),
            window.appearance(),
            TextSystem::new(),
        ),
        user,
        verdicts,
        verdict_sender,
        minutes: MinuteTimer::new()?,
        backdrop: None,
        backdrop_requested: false,
        picture,
        devices: system.as_ref().and_then(Devices::follow),
        layouts: Layouts::load(),
        wakes: system.as_ref().and_then(|system| {
            follow_wakes(system)
                .inspect_err(|error| tracing::warn!("cannot follow waking: {error}"))
                .ok()
        }),
        power: bande_bus::session()
            .and_then(|session| PowerProxyBlocking::new(&session))
            .inspect_err(|error| tracing::warn!("the password is always required: {error}"))
            .ok(),
        unlocked: false,
    };
    locker.screen.set_note(&config_text("LockMessage"));
    locker.screen.start_resting();
    bande_toolkit::run(&mut window, &mut locker).map_err(|error: WindowError| error.into())
}

fn main() -> ExitCode {
    bande_toolkit::skip_absent_vulkan_drivers();
    bande_log::init("bande-lock");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
