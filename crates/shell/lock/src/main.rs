use std::os::fd::{AsFd, BorrowedFd};
use std::process::ExitCode;
use std::thread;
use std::time::Instant;

use accesskit::TreeUpdate;
use bande_bus::logind::ManagerProxyBlocking;
use bande_datetime::{DateFormats, MinuteTimer};
use bande_keyboard::Layouts;
use bande_lock::{
    BackdropSource, Devices, Inbox, LockScreen, Look, PAM_SERVICE, PICTURE_PIXELS, Purpose,
    Refusal, Said, Transaction, User, Variant, current_user, is_deliberate, load_backdrop,
    load_pictures,
};
use bande_toolkit::ui::frame_time;
use bande_toolkit::{
    App, Image, Role, Scene, Scheme, TextSystem, Window, WindowError, WindowEvent,
};
use rustix::process::{DumpableBehavior, set_dumpable_behavior};
use zeroize::Zeroizing;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Minutes,
    Transaction,
    Devices,
    Backdrop,
    Picture,
    Wakes,
}

struct Locker {
    screen: LockScreen,
    user: User,
    transaction: Option<Transaction>,
    asked: bool,
    pending: Option<Zeroizing<String>>,
    minutes: MinuteTimer,
    backdrop: Option<Inbox<(Look, Image)>>,
    backdrop_requested: bool,
    picture: Option<Inbox<(usize, Image)>>,
    devices: Option<Devices>,
    layouts: Layouts,
    sources: Vec<String>,
    active_source: usize,
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

    fn converse(&mut self) {
        if self.transaction.is_some() {
            return;
        }
        self.asked = false;
        match Transaction::begin(PAM_SERVICE, &self.user.login) {
            Ok(transaction) => self.transaction = Some(transaction),
            Err(error) => {
                tracing::warn!("cannot ask PAM: {error}");
                self.screen
                    .refuse(Refusal::Unavailable, frame_time().on_screen());
            }
        }
    }

    fn start_over(&mut self) {
        self.transaction = None;
        self.pending = None;
        self.screen.ask(None);
        self.converse();
    }

    fn answer(&mut self, answer: Zeroizing<String>) {
        if !self.asked {
            self.pending = Some(answer);
            return;
        }
        self.asked = false;
        let sent = self
            .transaction
            .as_ref()
            .is_some_and(|transaction| transaction.answer(answer));
        if !sent {
            self.start_over();
        }
    }

    fn follow(&mut self, window: &mut Window) {
        let said = self
            .transaction
            .as_mut()
            .map(Transaction::drain)
            .unwrap_or_default();
        for said in said {
            match said {
                Said::Ask(prompt) => match self.pending.take() {
                    Some(answer) if !prompt.visible => {
                        self.asked = true;
                        self.answer(answer);
                    }
                    _ => {
                        self.asked = true;
                        self.screen.ask(Some(&prompt));
                    }
                },
                Said::Show(message) => self.screen.set_message(Some(message)),
                Said::Done(Ok(())) => {
                    self.transaction = None;
                    self.screen.accepted();
                    self.unlock(window);
                }
                Said::Done(Err(refusal)) => {
                    self.screen.refuse(refusal, frame_time().on_screen());
                    self.transaction = None;
                    self.pending = None;
                    self.converse();
                }
            }
        }
    }

    fn sources_in_order(&self) -> Vec<(Source, BorrowedFd<'_>)> {
        let mut sources = vec![(Source::Minutes, self.minutes.as_fd())];
        sources.extend(
            self.transaction
                .as_ref()
                .map(|transaction| (Source::Transaction, transaction.fd())),
        );
        sources.extend(
            self.devices
                .as_ref()
                .map(|devices| (Source::Devices, devices.fd())),
        );
        sources.extend(
            self.backdrop
                .as_ref()
                .map(|inbox| (Source::Backdrop, inbox.fd())),
        );
        sources.extend(
            self.picture
                .as_ref()
                .map(|inbox| (Source::Picture, inbox.fd())),
        );
        sources.extend(self.wakes.as_ref().map(|inbox| (Source::Wakes, inbox.fd())));
        sources
    }

    fn after_input(&mut self, window: &mut Window) {
        if self.screen.take_cancel() {
            self.start_over();
        }
        if let Some(answer) = self.screen.take_answer() {
            self.answer(answer);
        }
        if self.screen.take_next_input_source() && self.sources.len() > 1 {
            let next = (self.active_source + 1) % self.sources.len();
            if let Err(error) = window.select_input_source(next) {
                tracing::warn!("cannot switch the input source: {error}");
            }
        }
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
            let typed = self.screen.wake_on(input, Instant::now());
            self.converse();
            if !typed {
                window.request_redraw();
                return;
            }
        }
        let changed = match event {
            WindowEvent::Input(input) => self.screen.handle(&input),
            WindowEvent::Accessibility(request) => {
                self.screen.wake(Instant::now());
                self.converse();
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
                self.sources = sources;
                self.active_source = active;
                self.screen.set_input_source(name)
            }
            _ => false,
        };
        if changed {
            window.request_redraw();
            self.after_input(window);
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.sources_in_order()
            .into_iter()
            .map(|(_, fd)| fd)
            .collect()
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        let Some(source) = self
            .sources_in_order()
            .get(index)
            .map(|(source, _)| *source)
        else {
            return;
        };
        match source {
            Source::Minutes => {
                if let Err(error) = self.minutes.wait() {
                    tracing::warn!("cannot follow the clock: {error}");
                }
                self.screen.set_clock(&bande_datetime::now());
            }
            Source::Transaction => self.follow(window),
            Source::Devices => {
                let Some(corner) = self.devices.as_mut().and_then(Devices::latest) else {
                    return;
                };
                if !self.screen.set_devices(
                    corner.battery,
                    corner.wifi.as_ref(),
                    corner.wired.as_ref(),
                ) {
                    return;
                }
            }
            Source::Backdrop => {
                let Some((_, image)) = self
                    .backdrop
                    .take()
                    .and_then(|mut inbox| inbox.drain().pop())
                else {
                    return;
                };
                self.screen.set_backdrop(image, frame_time().on_screen());
            }
            Source::Picture => {
                let Some((_, picture)) = self
                    .picture
                    .take()
                    .and_then(|mut inbox| inbox.drain().pop())
                else {
                    return;
                };
                self.screen.set_picture(picture);
            }
            Source::Wakes => {
                if let Some(wakes) = self.wakes.as_mut() {
                    wakes.drain();
                }
                self.screen.wake(Instant::now());
                self.converse();
            }
        }
        window.request_redraw();
    }

    fn deadline(&self) -> Option<Instant> {
        self.screen.deadline()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        if self.screen.tick(now) {
            if self.screen.is_resting() {
                self.transaction = None;
                self.pending = None;
            }
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
    let picture = load_pictures(vec![user.login.clone()], PICTURE_PIXELS)
        .inspect_err(|error| tracing::warn!("cannot load the account picture: {error}"))
        .ok();
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
            Purpose::Lock,
        ),
        user,
        transaction: None,
        asked: false,
        pending: None,
        minutes: MinuteTimer::new()?,
        backdrop: None,
        backdrop_requested: false,
        picture,
        devices: system.as_ref().and_then(Devices::follow),
        layouts: Layouts::load(),
        sources: Vec::new(),
        active_source: 0,
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
    bande_log::init("bande-lock");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
