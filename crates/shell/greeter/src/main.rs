use std::fs::File;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;
use bande_config::{
    Accessibility, Appearance as Look, CARRIED_ACCESSIBILITY, Config, LOGIN_PICTURES, WallpaperFit,
    carry, login_defaults,
};
use bande_datetime::daylight::{Sky, place_of, sky};
use bande_datetime::{DateFormats, MinuteTimer};
use bande_greeter::{
    FAILED_SESSIONS, Login, Request, Step, account_users, failed_session, last_user_file,
    login_accessibility, login_look, login_picture, login_users, read_message, remember_user,
    remembered_user, write_message,
};
use bande_keyboard::Layouts;
use bande_l10n::tr;
use bande_lock::{
    BackdropSource, Corner, Devices, Fit, Inbox, LockScreen, Look as Drawn, PICTURE_PIXELS, Peer,
    PowerAction, Prompt, Purpose, Refusal, User, Variant, load_backdrop, load_pictures, night,
};
use bande_status::devices::{powers, use_power};
use bande_status::session::Ending;
use bande_toolkit::ui::{Event, Key, frame_time};
use bande_toolkit::{
    Appearance, Contrast, Edges, Image, Item, KeyboardFocus, LayerKind, LayerOptions, Rect, Role,
    Scene, Shape, Surfaces, TextSize, TextSystem, Window, WindowEvent,
};
use rustix::process::{DumpableBehavior, set_dumpable_behavior};
use zbus::blocking::Connection;
use zeroize::Zeroizing;

const SHOWN_PEERS: usize = 6;
const BACKDROP_NAMESPACE: &str = "greeter-backdrop";
const SLEEPS_AFTER: Duration = Duration::from_secs(10 * 60);

struct Backdrop {
    output: String,
    image: Option<Image>,
    loading: Option<Inbox<(Drawn, Image)>>,
    size: Option<(u32, u32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Minutes,
    Devices,
    Backdrop,
    Sharp,
    Pictures,
    Greetd,
    Powers,
    OtherBackdrop(usize),
}

struct Wallpaper {
    picture: Option<Arc<File>>,
    fit: Fit,
    variant: Variant,
}

struct Greeter {
    windows: Vec<Window>,
    backdrops: Vec<Backdrop>,
    users: Vec<User>,
    pictures: Vec<Option<Image>>,
    picture_inbox: Option<Inbox<(usize, Image)>>,
    shown: usize,
    other: Option<String>,
    peers: Vec<Option<usize>>,
    screen: LockScreen,
    greetd: Option<UnixStream>,
    login: Option<Login>,
    awaiting: bool,
    asked: bool,
    cancel_queued: bool,
    next: Option<String>,
    refusal: Option<Refusal>,
    pending: Option<Zeroizing<String>>,
    started: bool,
    caps_lock: bool,
    minutes: MinuteTimer,
    backdrop: Option<Inbox<(Drawn, Image)>>,
    backdrop_size: Option<(u32, u32)>,
    sharp: Option<Inbox<(Drawn, Image)>>,
    wallpaper: Wallpaper,
    blurred: Option<Image>,
    look: Option<Config>,
    last_user: Option<PathBuf>,
    layouts: Layouts,
    input_source: Option<String>,
    sources: Vec<String>,
    active_source: usize,
    text: TextSystem,
    system: Option<Connection>,
    powers: Vec<PowerAction>,
    power_answers: Option<Inbox<Vec<PowerAction>>>,
    devices: Option<Devices>,
    corner: Corner,
    last_input: Instant,
}

fn variant_of(look: Look) -> Variant {
    match look {
        Look::Light => Variant::Light,
        Look::Dark => Variant::Dark,
        Look::Automatic => {
            let now = bande_datetime::now();
            match sky(&now, place_of(now.time_zone())) {
                Some((Sky::Day, _)) | None => Variant::Light,
                Some((Sky::Night, _)) => Variant::Dark,
            }
        }
    }
}

fn appearance_of(login: &Accessibility, own: Option<&Accessibility>) -> Appearance {
    let own = own.copied().unwrap_or_default();
    Appearance {
        contrast: match login.increase_contrast || own.increase_contrast {
            true => Contrast::Increased,
            false => Contrast::Standard,
        },
        reduce_motion: login.reduce_motion || own.reduce_motion,
        reduce_transparency: login.reduce_transparency || own.reduce_transparency,
        text_size: TextSize::from_percent(login.text_size.max(own.text_size)),
        ..Appearance::default()
    }
}

fn other_user(name: Option<&str>) -> User {
    User {
        uid: u32::MAX,
        login: name.unwrap_or_default().to_owned(),
        full_name: name.map_or_else(|| tr!("Other User"), str::to_owned),
    }
}

fn backdrop_role() -> Role {
    Role::Layer(LayerOptions {
        layer: LayerKind::Overlay,
        anchor: Edges::ALL,
        exclusive_zone: -1,
        keyboard: KeyboardFocus::None,
        namespace: BACKDROP_NAMESPACE.to_owned(),
    })
}

fn ask_powers(system: &Connection) -> Option<Inbox<Vec<PowerAction>>> {
    let answers = Inbox::new()
        .inspect_err(|error| tracing::warn!("cannot ask logind about power: {error}"))
        .ok()?;
    let outbox = answers.outbox();
    let system = system.clone();
    let asked = thread::Builder::new()
        .name("powers".to_owned())
        .spawn(move || match powers(&system) {
            Ok(powers) => {
                outbox.send(powers);
            }
            Err(error) => tracing::warn!("cannot ask logind about power: {error}"),
        });
    match asked {
        Ok(_) => Some(answers),
        Err(error) => {
            tracing::warn!("cannot ask logind about power: {error}");
            None
        }
    }
}

impl Greeter {
    fn current(&self) -> User {
        self.users
            .get(self.shown)
            .cloned()
            .unwrap_or_else(|| other_user(self.other.as_deref()))
    }

    fn current_login(&self) -> Option<String> {
        match self.users.get(self.shown) {
            Some(user) => Some(user.login.clone()),
            None => self.other.clone(),
        }
    }

    fn choose_look(&mut self) {
        let user = self.users.get(self.shown);
        self.look = user.and_then(|user| login_look(Path::new(LOGIN_PICTURES), user.uid));
        let variant = self
            .look
            .as_ref()
            .map_or(Variant::Dark, |look| variant_of(look.appearance));
        let fit = match self.look.as_ref().map(|look| look.wallpaper.fit) {
            Some(WallpaperFit::Fit) => Fit::Fit,
            _ => Fit::Fill,
        };
        let picture = user
            .and_then(|user| {
                login_picture(
                    Path::new(LOGIN_PICTURES),
                    user.uid,
                    variant == Variant::Light,
                )
            })
            .map(Arc::new);
        self.wallpaper = Wallpaper {
            picture,
            fit,
            variant,
        };
    }

    fn appearance(&self) -> Appearance {
        appearance_of(
            &login_accessibility(),
            self.look.as_ref().map(|look| &look.accessibility),
        )
    }

    fn peer_list(&self) -> (Vec<Option<usize>>, Vec<Peer>) {
        let mut listed: Vec<Option<usize>> =
            (0..self.users.len().min(SHOWN_PEERS)).map(Some).collect();
        if self.shown >= SHOWN_PEERS
            && self.shown < self.users.len()
            && let Some(last) = listed.last_mut()
        {
            *last = Some(self.shown);
        }
        listed.push(None);
        let peers = listed
            .iter()
            .map(|index| match index {
                Some(index) => {
                    let user = &self.users[*index];
                    Peer {
                        name: user.full_name.clone(),
                        initial: user
                            .full_name
                            .chars()
                            .next()
                            .map(|initial| initial.to_uppercase().collect())
                            .unwrap_or_default(),
                        picture: self.pictures.get(*index).cloned().flatten(),
                    }
                }
                None => Peer {
                    name: tr!("Other…"),
                    initial: "…".to_owned(),
                    picture: None,
                },
            })
            .collect();
        (listed, peers)
    }

    fn rebuild_screen(&mut self) {
        self.choose_look();
        let user = self.current();
        let mut screen = LockScreen::new(
            &user,
            DateFormats::from_env(),
            &bande_datetime::now(),
            self.appearance(),
            self.text.sharing_fonts(),
            Purpose::Login,
        );
        screen.set_position(self.shown, self.users.len() + 1);
        screen.offer_power(&self.powers);
        screen.set_input_source(self.input_source.clone());
        screen.set_caps_lock(self.caps_lock);
        screen.set_devices(
            self.corner.battery,
            self.corner.wifi.as_ref(),
            self.corner.wired,
        );
        if !self.users.is_empty() {
            let (listed, peers) = self.peer_list();
            let selected = listed
                .iter()
                .position(|index| *index == self.users.get(self.shown).map(|_| self.shown))
                .unwrap_or(listed.len() - 1);
            screen.set_peers(peers, selected);
            self.peers = listed;
        }
        if let Some(picture) = self.pictures.get(self.shown).cloned().flatten() {
            screen.set_picture(picture);
        }
        if let Some(user) = self.users.get(self.shown) {
            screen.set_message(failed_session(Path::new(FAILED_SESSIONS), user.uid));
        }
        if let Some(blurred) = &self.blurred {
            screen.set_backdrop(blurred.clone(), Instant::now() - Duration::from_secs(1));
        }
        screen.wake(Instant::now());
        self.screen = screen;
        self.backdrop = None;
        self.backdrop_size = None;
        self.sharp = None;
        for backdrop in &mut self.backdrops {
            backdrop.loading = None;
            backdrop.size = None;
        }
    }

    fn switch_to(&mut self, index: usize) {
        if self.started || index > self.users.len() {
            return;
        }
        self.shown = index;
        self.other = None;
        self.pending = None;
        self.rebuild_screen();
        self.converse();
    }

    fn step_user(&mut self, forward: bool) {
        let count = self.users.len() + 1;
        let next = match forward {
            true => (self.shown + 1) % count,
            false => (self.shown + count - 1) % count,
        };
        self.switch_to(next);
    }

    fn request_backdrop(
        &self,
        size: (u32, u32),
        looks: Vec<Drawn>,
    ) -> Option<Inbox<(Drawn, Image)>> {
        let source = BackdropSource::Picture(self.wallpaper.picture.clone(), self.wallpaper.fit);
        load_backdrop(source, self.wallpaper.variant, size, looks)
            .inspect_err(|error| tracing::warn!("cannot load the wallpaper: {error}"))
            .ok()
    }

    fn show_backdrop(&mut self, drawn: Vec<(Drawn, Image)>) {
        for (look, image) in drawn {
            match look {
                Drawn::Blurred => {
                    self.screen
                        .set_backdrop(image.clone(), frame_time().on_screen());
                    self.blurred = Some(image);
                }
                Drawn::Sharp => self.screen.set_wallpaper(image),
            }
        }
    }

    fn unavailable(&mut self, error: &std::io::Error) {
        tracing::warn!("greetd: {error}");
        self.greetd = None;
        self.login = None;
        self.awaiting = false;
        self.cancel_queued = false;
        self.next = None;
        self.asked = false;
        self.screen.ask(None);
        self.screen
            .refuse(Refusal::Unavailable, frame_time().on_screen());
    }

    fn send(&mut self, request: &Request) {
        let connected = match self.greetd.take() {
            Some(greetd) => Ok(greetd),
            None => std::env::var_os("GREETD_SOCK")
                .ok_or_else(|| std::io::Error::other("GREETD_SOCK is not set"))
                .and_then(UnixStream::connect),
        };
        match connected.and_then(|mut greetd| write_message(&mut greetd, request).map(|()| greetd))
        {
            Ok(greetd) => {
                self.greetd = Some(greetd);
                self.awaiting = true;
            }
            Err(error) => self.unavailable(&error),
        }
    }

    fn converse(&mut self) {
        self.asked = false;
        self.refusal = None;
        let Some(login) = self.current_login() else {
            self.screen.ask(Some(&Prompt {
                text: tr!("User Name"),
                visible: true,
            }));
            if let Some(current) = self.login.as_mut().filter(|login| !login.is_idle()) {
                let cancel = current.cancel();
                match self.awaiting {
                    true => self.cancel_queued = true,
                    false => self.send(&cancel),
                }
            }
            return;
        };
        match self.login.as_mut() {
            Some(current) if !current.is_idle() => {
                self.next = Some(login);
                if !current.is_cancelling() {
                    let cancel = current.cancel();
                    match self.awaiting {
                        true => self.cancel_queued = true,
                        false => self.send(&cancel),
                    }
                }
            }
            _ => {
                self.next = None;
                let (login, request) = Login::begin(&login);
                self.login = Some(login);
                self.send(&request);
            }
        }
    }

    fn carried_environment(&self) -> Vec<String> {
        let before = match self.look {
            Some(_) => login_defaults(),
            None => Accessibility::default(),
        };
        let words = carry(&login_accessibility(), &before);
        match words.is_empty() {
            true => Vec::new(),
            false => vec![format!("{CARRIED_ACCESSIBILITY}={words}")],
        }
    }

    fn answer(&mut self, answer: Zeroizing<String>) {
        if self.sharp.is_none()
            && let Some(size) = self.backdrop_size
        {
            self.sharp = self.request_backdrop(size, vec![Drawn::Sharp]);
        }
        if self.shown >= self.users.len() && self.other.is_none() {
            let name = answer.trim();
            if !name.is_empty() {
                self.other = Some(name.to_owned());
                self.rebuild_screen();
                self.screen.ask(None);
                self.converse();
            }
            return;
        }
        if self.login.is_none() {
            self.pending = Some(answer);
            self.converse();
            return;
        }
        if !self.asked || self.awaiting {
            self.pending = Some(answer);
            return;
        }
        self.asked = false;
        if let Some(request) = self.login.as_ref().map(|login| login.answer(answer)) {
            self.send(&request);
        }
    }

    fn follow(&mut self) {
        let Some(greetd) = self.greetd.as_mut() else {
            return;
        };
        let response = match read_message(greetd) {
            Ok(response) => response,
            Err(error) => {
                self.unavailable(&error);
                return;
            }
        };
        self.awaiting = false;
        if std::mem::take(&mut self.cancel_queued) {
            self.send(&Request::CancelSession);
            return;
        }
        let steps = self
            .login
            .as_mut()
            .map(|login| login.respond(response))
            .unwrap_or_default();
        for step in steps {
            match step {
                Step::Send(request) => self.send(&request),
                Step::Ask(prompt) => {
                    self.asked = true;
                    match self.pending.take() {
                        Some(answer) if !prompt.visible => self.answer(answer),
                        _ => self.screen.ask(Some(&prompt)),
                    }
                }
                Step::Show(message) => self.screen.set_message(Some(message)),
                Step::Refused(refusal) => self.refusal = Some(refusal),
                Step::Authenticated => {
                    let environment = self.carried_environment();
                    if let Some(request) =
                        self.login.as_mut().map(|login| login.start(&environment))
                    {
                        self.send(&request);
                    }
                }
                Step::Started => self.started = true,
            }
        }
        if !self.login.as_ref().is_some_and(Login::is_idle) {
            return;
        }
        if self.started {
            self.leave();
            return;
        }
        if let Some(next) = self.next.take() {
            let (login, request) = Login::begin(&next);
            self.login = Some(login);
            self.send(&request);
            return;
        }
        if let Some(refusal) = self.refusal.take() {
            self.pending = None;
            if self.shown >= self.users.len() {
                self.other = None;
                self.rebuild_screen();
            }
            self.screen.refuse(refusal, frame_time().on_screen());
            self.converse();
        }
    }

    fn leave(&mut self) {
        tracing::info!("signed in; handing the screen to the session");
        self.screen.accepted();
        self.screen.leave(frame_time().on_screen());
        if let Some(file) = &self.last_user
            && let Some(login) = self.current_login()
            && let Err(error) = remember_user(file, &login)
        {
            tracing::info!("cannot remember who signed in last: {error}");
        }
    }

    fn after_input(&mut self) {
        if let Some(chosen) = self.screen.take_chosen_peer()
            && let Some(index) = self.peers.get(chosen).copied()
        {
            self.switch_to(index.unwrap_or(self.users.len()));
            return;
        }
        if self.screen.take_cancel() {
            self.pending = None;
            if self.shown >= self.users.len() && self.other.is_some() {
                self.other = None;
                self.rebuild_screen();
            }
            self.converse();
        }
        if let Some(answer) = self.screen.take_answer() {
            self.answer(answer);
        }
        if self.screen.take_next_input_source() {
            self.next_input_source();
        }
        if let Some(power) = self.screen.take_power()
            && let Some(system) = &self.system
            && let Err(error) = use_power(system, power)
        {
            tracing::warn!("cannot {power:?}: {error}");
            self.screen.set_message(power.ending().map(Ending::failed));
        }
    }

    fn next_input_source(&mut self) {
        if self.sources.len() < 2 {
            return;
        }
        let next = (self.active_source + 1) % self.sources.len();
        if let Err(error) = self.windows[0].select_input_source(next) {
            tracing::warn!("cannot switch the input source: {error}");
        }
    }

    fn noticed(&mut self, now: Instant) {
        self.last_input = now;
        if self.screen.is_resting()
            && let Some(system) = &self.system
        {
            self.power_answers = ask_powers(system);
        }
        self.screen.wake(now);
    }

    fn sleeps_at(&self) -> Option<Instant> {
        let on_battery = self
            .corner
            .battery
            .is_some_and(|battery| !battery.on_adapter());
        (on_battery && self.powers.contains(&PowerAction::Sleep) && !self.started)
            .then(|| self.last_input + SLEEPS_AFTER)
    }

    fn show_displays(&mut self, names: &[String]) {
        let wanted = names.get(1..).unwrap_or_default();
        let mut index = 0;
        while index < self.backdrops.len() {
            if wanted.contains(&self.backdrops[index].output) {
                index += 1;
            } else {
                self.backdrops.remove(index);
                self.windows.remove(index + 1);
            }
        }
        for output in wanted {
            if self
                .backdrops
                .iter()
                .any(|backdrop| &backdrop.output == output)
            {
                continue;
            }
            match self.windows[0].open_beside_on(output, backdrop_role(), (0, 0)) {
                Ok(window) => {
                    self.windows.push(window);
                    self.backdrops.push(Backdrop {
                        output: output.clone(),
                        image: None,
                        loading: None,
                        size: None,
                    });
                }
                Err(error) => tracing::warn!("cannot cover {output}: {error}"),
            }
        }
    }

    fn sources_in_order(&self) -> Vec<(Source, BorrowedFd<'_>)> {
        let mut sources = vec![(Source::Minutes, self.minutes.as_fd())];
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
        sources.extend(self.sharp.as_ref().map(|inbox| (Source::Sharp, inbox.fd())));
        sources.extend(
            self.picture_inbox
                .as_ref()
                .map(|inbox| (Source::Pictures, inbox.fd())),
        );
        sources.extend(
            self.greetd
                .as_ref()
                .map(|greetd| (Source::Greetd, greetd.as_fd())),
        );
        sources.extend(
            self.power_answers
                .as_ref()
                .map(|inbox| (Source::Powers, inbox.fd())),
        );
        sources.extend(
            self.backdrops
                .iter()
                .enumerate()
                .filter_map(|(index, backdrop)| {
                    backdrop
                        .loading
                        .as_ref()
                        .map(|inbox| (Source::OtherBackdrop(index), inbox.fd()))
                }),
        );
        sources
    }

    fn main_event(&mut self, event: WindowEvent) {
        if let WindowEvent::Input(_) | WindowEvent::Accessibility(_) = &event {
            self.noticed(Instant::now());
        }
        let changed = match event {
            WindowEvent::Input(Event::Key {
                key: key @ (Key::Up | Key::Down),
                ..
            }) if !self.started => {
                self.step_user(key == Key::Down);
                true
            }
            WindowEvent::Input(input) => self.screen.handle(&input),
            WindowEvent::CapsLock(on) => {
                self.caps_lock = on;
                self.screen.set_caps_lock(on)
            }
            WindowEvent::Accessibility(request) => self.screen.perform(&request),
            WindowEvent::InputSources { sources, active } => {
                self.input_source = sources
                    .get(active)
                    .filter(|_| sources.len() > 1)
                    .map(|code| self.layouts.name(code));
                self.sources = sources;
                self.active_source = active;
                self.screen.set_input_source(self.input_source.clone())
            }
            WindowEvent::Displays(names) => {
                self.show_displays(&names);
                false
            }
            _ => false,
        };
        if changed {
            self.windows[0].request_redraw();
            self.after_input();
        }
    }
}

impl Surfaces for Greeter {
    fn windows(&self) -> &[Window] {
        &self.windows
    }

    fn windows_mut(&mut self) -> &mut [Window] {
        &mut self.windows
    }

    fn scene(&mut self, index: usize) -> Scene {
        let (width, height) = self.windows[index].size();
        if index > 0 {
            let palette = night(self.appearance());
            let backdrop = &mut self.backdrops[index - 1];
            if backdrop.size != Some((width, height)) && width > 0 && height > 0 {
                backdrop.size = Some((width, height));
                let source =
                    BackdropSource::Picture(self.wallpaper.picture.clone(), self.wallpaper.fit);
                backdrop.loading = load_backdrop(
                    source,
                    self.wallpaper.variant,
                    (width, height),
                    vec![Drawn::Blurred],
                )
                .inspect_err(|error| tracing::warn!("cannot load the wallpaper: {error}"))
                .ok();
            }
            let frame = Rect::new(0.0, 0.0, width as f32, height as f32);
            let items = backdrop.image.as_ref().map_or_else(Vec::new, |image| {
                vec![
                    Item::Image {
                        rect: frame,
                        image: image.clone(),
                    },
                    Item::Fill {
                        shape: Shape::rect(frame),
                        paint: palette.scrim.into(),
                    },
                ]
            });
            return Scene {
                width,
                height,
                background: palette.chassis,
                items,
            };
        }
        if self.backdrop_size.is_none() && width > 0 && height > 0 {
            self.backdrop_size = Some((width, height));
            self.backdrop = self.request_backdrop((width, height), vec![Drawn::Blurred]);
        }
        self.screen.set_scale(self.windows[0].scale());
        self.screen.advance(frame_time().presentation);
        self.screen.scene(width, height)
    }

    fn event(&mut self, index: usize, event: WindowEvent) {
        match index {
            0 => self.main_event(event),
            _ => {
                if let WindowEvent::Input(_) = event {
                    self.noticed(Instant::now());
                    self.windows[0].request_redraw();
                }
            }
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.sources_in_order()
            .into_iter()
            .map(|(_, fd)| fd)
            .collect()
    }

    fn source_ready(&mut self, index: usize) {
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
            Source::Devices => {
                let Some(corner) = self.devices.as_mut().and_then(Devices::latest) else {
                    return;
                };
                self.screen
                    .set_devices(corner.battery, corner.wifi.as_ref(), corner.wired);
                self.corner = corner;
                if let Some(system) = &self.system {
                    self.power_answers = ask_powers(system);
                }
            }
            Source::Backdrop | Source::Sharp => {
                let inbox = match source {
                    Source::Backdrop => self.backdrop.as_mut(),
                    _ => self.sharp.as_mut(),
                };
                let drawn = inbox.map(Inbox::drain).unwrap_or_default();
                self.show_backdrop(drawn);
            }
            Source::Pictures => {
                let drawn = self
                    .picture_inbox
                    .as_mut()
                    .map(Inbox::drain)
                    .unwrap_or_default();
                for (index, picture) in drawn {
                    if let Some(slot) = self.pictures.get_mut(index) {
                        *slot = Some(picture.clone());
                    }
                    if index == self.shown {
                        self.screen.set_picture(picture);
                    }
                }
                let (listed, peers) = self.peer_list();
                if !self.users.is_empty() {
                    let selected = listed
                        .iter()
                        .position(|index| *index == Some(self.shown))
                        .unwrap_or(listed.len() - 1);
                    self.screen.set_peers(peers, selected);
                    self.peers = listed;
                }
            }
            Source::Greetd => self.follow(),
            Source::Powers => {
                let answered = self
                    .power_answers
                    .take()
                    .and_then(|mut inbox| inbox.drain().pop());
                if let Some(powers) = answered
                    && powers != self.powers
                {
                    self.powers = powers;
                    self.screen.offer_power(&self.powers);
                }
            }
            Source::OtherBackdrop(other) => {
                if let Some(backdrop) = self.backdrops.get_mut(other)
                    && let Some((_, image)) = backdrop
                        .loading
                        .take()
                        .and_then(|mut inbox| inbox.drain().pop())
                {
                    backdrop.image = Some(image);
                    if let Some(window) = self.windows.get_mut(other + 1) {
                        window.request_redraw();
                    }
                }
                return;
            }
        }
        self.windows[0].request_redraw();
    }

    fn deadline(&self) -> Option<Instant> {
        let finished = (self.started && self.screen.has_left()).then(Instant::now);
        [self.screen.deadline(), self.sleeps_at(), finished]
            .into_iter()
            .flatten()
            .min()
    }

    fn tick(&mut self, now: Instant) {
        if self.screen.tick(now) {
            self.windows[0].request_redraw();
        }
        if self.sleeps_at().is_some_and(|at| now >= at)
            && let Some(system) = &self.system
        {
            self.last_input = now;
            if let Err(error) = use_power(system, PowerAction::Sleep) {
                tracing::warn!("cannot sleep at the login window: {error}");
            }
        }
    }

    fn animating(&self, index: usize) -> bool {
        index == 0 && self.screen.is_animating()
    }

    fn accessibility(&self, index: usize) -> Option<TreeUpdate> {
        (index == 0).then(|| self.screen.accessibility())
    }

    fn closed(&mut self) -> bool {
        self.windows[0].is_closed() || (self.started && self.screen.has_left())
    }
}

fn listed_users(system: Option<&Connection>) -> Vec<User> {
    let listed = system.map(|system| {
        account_users(system)
            .inspect_err(|error| tracing::info!("AccountsService lists no one: {error}"))
    });
    match listed {
        Some(Ok(users)) if !users.is_empty() => users,
        _ => match std::fs::read_to_string("/etc/passwd") {
            Ok(passwd) => login_users(&passwd),
            Err(error) => {
                tracing::warn!("cannot read /etc/passwd: {error}");
                Vec::new()
            }
        },
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    set_dumpable_behavior(DumpableBehavior::NotDumpable)?;
    let system = bande_bus::system()
        .inspect_err(|error| tracing::warn!("no system bus: {error}"))
        .ok();
    let users = listed_users(system.as_ref());
    let last_user = last_user_file();
    let remembered = last_user.as_deref().and_then(remembered_user);
    let shown = users
        .iter()
        .position(|user| Some(&user.login) == remembered.as_ref())
        .unwrap_or(0)
        .min(users.len());
    let powers = system
        .as_ref()
        .map(|system| {
            powers(system)
                .inspect_err(|error| tracing::warn!("cannot ask logind about power: {error}"))
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let window = Window::open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Overlay,
            anchor: Edges::ALL,
            exclusive_zone: -1,
            keyboard: KeyboardFocus::Exclusive,
            namespace: "greeter".to_owned(),
        }),
        (0, 0),
    )?;
    let devices = system.as_ref().and_then(Devices::follow);
    let picture_inbox = load_pictures(
        users.iter().map(|user| user.login.clone()).collect(),
        PICTURE_PIXELS,
    )
    .inspect_err(|error| tracing::warn!("cannot load the account pictures: {error}"))
    .ok();
    let text = TextSystem::new();
    let placeholder = LockScreen::new(
        &other_user(None),
        DateFormats::from_env(),
        &bande_datetime::now(),
        Appearance::default(),
        text.sharing_fonts(),
        Purpose::Login,
    );
    let mut greeter = Greeter {
        windows: vec![window],
        backdrops: Vec::new(),
        pictures: vec![None; users.len()],
        users,
        picture_inbox,
        shown,
        other: None,
        peers: Vec::new(),
        screen: placeholder,
        greetd: None,
        login: None,
        awaiting: false,
        asked: false,
        cancel_queued: false,
        next: None,
        refusal: None,
        pending: None,
        started: false,
        caps_lock: false,
        minutes: MinuteTimer::new()?,
        backdrop: None,
        backdrop_size: None,
        sharp: None,
        wallpaper: Wallpaper {
            picture: None,
            fit: Fit::Fill,
            variant: Variant::Dark,
        },
        blurred: None,
        look: None,
        last_user,
        layouts: Layouts::load(),
        input_source: None,
        sources: Vec::new(),
        active_source: 0,
        text,
        system,
        powers,
        power_answers: None,
        devices,
        corner: Corner::default(),
        last_input: Instant::now(),
    };
    greeter.rebuild_screen();
    greeter.converse();
    bande_toolkit::run_surfaces(&mut greeter)?;
    Ok(())
}

fn main() -> ExitCode {
    bande_log::init("bande-greeter");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
