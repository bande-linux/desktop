use std::fs::File;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;
use bande_config::LOGIN_PICTURES;
use bande_datetime::{DateFormats, MinuteTimer};
use bande_greeter::{
    L10N, Login, Rejection, Request, Step, last_user_file, login_picture, login_users,
    read_message, remember_user, remembered_user, write_message,
};
use bande_keyboard::Layouts;
use bande_lock::{
    BackdropSource, Devices, Inbox, LockScreen, Look, PowerAction, User, Variant, load_backdrop,
};
use bande_status::devices::{powers, use_power};
use bande_status::status::{Battery, Wifi};
use bande_toolkit::ui::{Event, Key, frame_time};
use bande_toolkit::{
    App, Appearance, Edges, Image, KeyboardFocus, LayerKind, LayerOptions, Role, Scene, TextSystem,
    Window, WindowEvent,
};
use zeroize::Zeroizing;

struct Conversation {
    login: Login,
    greetd: UnixStream,
    asking: bool,
    explained: bool,
    started: bool,
    rejection: Option<Rejection>,
}

impl Conversation {
    fn open(login: &str, password: Zeroizing<String>) -> std::io::Result<Self> {
        let socket = std::env::var_os("GREETD_SOCK")
            .ok_or_else(|| std::io::Error::other("GREETD_SOCK is not set"))?;
        let mut greetd = UnixStream::connect(socket)?;
        let (login, request) = Login::begin(login, password);
        write_message(&mut greetd, &request)?;
        Ok(Self {
            login,
            greetd,
            asking: false,
            explained: false,
            started: false,
            rejection: None,
        })
    }

    fn send(&mut self, request: &Request) -> std::io::Result<()> {
        write_message(&mut self.greetd, request)
    }
}

struct Greeter {
    users: Vec<User>,
    user: usize,
    screen: LockScreen,
    conversation: Option<Conversation>,
    caps_lock: bool,
    minutes: MinuteTimer,
    backdrop: Option<Inbox<(Look, Image)>>,
    backdrop_size: Option<(u32, u32)>,
    sharp: Option<Inbox<(Look, Image)>>,
    picture: Option<Arc<File>>,
    wallpaper: Option<Image>,
    last_user: Option<PathBuf>,
    layouts: Layouts,
    input_source: Option<String>,
    started: bool,
    text: TextSystem,
    system: Option<zbus::blocking::Connection>,
    powers: Vec<PowerAction>,
    devices: Option<Devices>,
    corner: (Option<Battery>, Option<Wifi>),
}

impl Greeter {
    fn screen_for(user: &User, text: TextSystem) -> LockScreen {
        LockScreen::new(
            user,
            DateFormats::from_env(),
            &bande_datetime::now(),
            Appearance::default(),
            text,
        )
    }

    fn switch_user(&mut self, forward: bool) -> bool {
        let count = self.users.len();
        if count < 2 || self.conversation.is_some() {
            return false;
        }
        self.user = if forward {
            (self.user + 1) % count
        } else {
            (self.user + count - 1) % count
        };
        self.screen = Self::screen_for(&self.users[self.user], self.text.sharing_fonts());
        self.screen.set_position(self.user, count);
        self.screen.offer_power(&self.powers);
        self.screen.set_input_source(self.input_source.clone());
        self.screen.set_caps_lock(self.caps_lock);
        self.screen
            .set_devices(self.corner.0, self.corner.1.as_ref());
        self.picture =
            login_picture(Path::new(LOGIN_PICTURES), self.users[self.user].uid).map(Arc::new);
        self.backdrop = None;
        self.backdrop_size = None;
        self.sharp = None;
        if let Some(wallpaper) = &self.wallpaper {
            self.screen
                .set_backdrop(wallpaper.clone(), Instant::now() - Duration::from_secs(1));
        }
        true
    }

    fn request_backdrop(
        &mut self,
        size: (u32, u32),
        looks: Vec<Look>,
    ) -> Option<Inbox<(Look, Image)>> {
        let picture = BackdropSource::Picture(self.picture.clone());
        load_backdrop(picture, Variant::Dark, size, looks)
            .inspect_err(|error| tracing::warn!("cannot load the wallpaper: {error}"))
            .ok()
    }

    fn show_backdrop(&mut self, drawn: Vec<(Look, Image)>) {
        for (look, image) in drawn {
            match look {
                Look::Blurred => {
                    self.screen
                        .set_backdrop(image.clone(), frame_time().on_screen());
                    self.wallpaper = Some(image);
                }
                Look::Sharp => self.screen.set_wallpaper(image),
            }
        }
    }

    fn sign_in(&mut self, password: Zeroizing<String>) {
        self.screen.set_message(None);
        if self.sharp.is_none()
            && let Some(size) = self.backdrop_size
        {
            self.sharp = self.request_backdrop(size, vec![Look::Sharp]);
        }
        let sent = match &mut self.conversation {
            Some(conversation) if conversation.asking => {
                conversation.asking = false;
                let answer = conversation.login.answer(password);
                conversation.send(&answer)
            }
            Some(_) => return,
            None => Conversation::open(&self.users[self.user].login, password)
                .map(|conversation| self.conversation = Some(conversation)),
        };
        if let Err(error) = sent {
            self.unavailable(&error);
        }
    }

    fn unavailable(&mut self, error: &std::io::Error) {
        tracing::warn!("greetd: {error}");
        self.conversation = None;
        self.screen.ask(None);
        self.screen.reject(frame_time().on_screen());
        self.screen.set_message(Some(L10N.tr("unavailable")));
    }

    fn cancel_answer(&mut self) {
        let Some(conversation) = self
            .conversation
            .as_mut()
            .filter(|conversation| conversation.asking)
        else {
            return;
        };
        conversation.asking = false;
        let cancel = conversation.login.cancel();
        if let Err(error) = conversation.send(&cancel) {
            self.unavailable(&error);
        }
    }

    fn converse(&mut self) {
        let Some(conversation) = self.conversation.as_mut() else {
            return;
        };
        if let Err(error) = Self::follow(conversation, &mut self.screen) {
            self.unavailable(&error);
            return;
        }
        if !conversation.login.is_idle() {
            return;
        }
        let Some(finished) = self.conversation.take() else {
            return;
        };
        match finished.rejection {
            None if finished.started => {
                self.started = true;
                tracing::info!("signed in; handing the screen to the session");
                self.screen.leave(frame_time().on_screen());
                if let Some(file) = &self.last_user
                    && let Err(error) = remember_user(file, &self.users[self.user].login)
                {
                    tracing::info!("cannot remember who signed in last: {error}");
                }
            }
            None => self.screen.ask(None),
            Some(rejection) => {
                self.screen.ask(None);
                self.screen.reject(frame_time().on_screen());
                match rejection {
                    Rejection::WrongPassword => {
                        self.screen.set_message(Some(L10N.tr("wrong-password")));
                    }
                    Rejection::Failed(reason) => {
                        tracing::info!("not signed in: {reason}");
                        if !finished.explained {
                            self.screen.set_message(Some(reason));
                        }
                    }
                }
            }
        }
    }

    fn follow(conversation: &mut Conversation, screen: &mut LockScreen) -> std::io::Result<()> {
        let response = read_message(&mut conversation.greetd)?;
        for step in conversation.login.respond(response) {
            match step {
                Step::Send(request) => conversation.send(&request)?,
                Step::Ask(prompt) => {
                    conversation.asking = true;
                    screen.ask(Some(&prompt));
                }
                Step::Show(message) => {
                    conversation.explained = true;
                    screen.set_message(Some(message));
                }
                Step::Rejected(rejection) => conversation.rejection = Some(rejection),
                Step::Started => conversation.started = true,
            }
        }
        Ok(())
    }
}

impl App for Greeter {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        if self.backdrop_size.is_none() && width > 0 && height > 0 {
            self.backdrop_size = Some((width, height));
            self.backdrop = self.request_backdrop((width, height), vec![Look::Blurred]);
        }
        self.screen.advance(frame_time().presentation);
        self.screen.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let changed = match event {
            WindowEvent::Input(Event::Key {
                key: key @ (Key::Up | Key::Down),
                ..
            }) => self.switch_user(key == Key::Down),
            WindowEvent::Input(
                input @ Event::Key {
                    key: Key::Escape, ..
                },
            ) => {
                self.cancel_answer();
                self.screen.handle(&input)
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
                self.screen.set_input_source(self.input_source.clone())
            }
            _ => false,
        };
        if changed {
            window.request_redraw();
            if let Some(password) = self.screen.take_password() {
                self.sign_in(password);
            }
            if let Some(power) = self.screen.take_power()
                && let Some(system) = &self.system
                && let Err(error) = use_power(system, power)
            {
                tracing::warn!("cannot {power:?}: {error}");
            }
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        std::iter::once(self.minutes.as_fd())
            .chain(self.devices.as_ref().map(Devices::fd))
            .chain(self.backdrop.as_ref().map(Inbox::fd))
            .chain(self.sharp.as_ref().map(Inbox::fd))
            .chain(
                self.conversation
                    .as_ref()
                    .map(|conversation| conversation.greetd.as_fd()),
            )
            .collect()
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        if index == 0 {
            if let Err(error) = self.minutes.wait() {
                tracing::warn!("cannot follow the clock: {error}");
            }
            self.screen.set_clock(&bande_datetime::now());
            window.request_redraw();
            return;
        }
        let following = usize::from(self.devices.is_some());
        if following == 1 && index == 1 {
            if let Some(corner) = self.devices.as_mut().and_then(Devices::latest) {
                if self.screen.set_devices(corner.0, corner.1.as_ref()) {
                    window.request_redraw();
                }
                self.corner = corner;
            }
            return;
        }
        let backdrop = following + 1;
        let sharp = backdrop + usize::from(self.backdrop.is_some());
        let drawn = match (&mut self.backdrop, &mut self.sharp) {
            (Some(inbox), _) if index == backdrop => Some(inbox.drain()),
            (_, Some(inbox)) if index == sharp => Some(inbox.drain()),
            _ => None,
        };
        match drawn {
            Some(drawn) => self.show_backdrop(drawn),
            None => self.converse(),
        }
        window.request_redraw();
    }

    fn deadline(&self) -> Option<Instant> {
        self.finished().then(Instant::now)
    }

    fn animating(&self) -> bool {
        self.screen.is_animating()
    }

    fn finished(&self) -> bool {
        self.started && self.screen.has_left()
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
    let text = TextSystem::new();
    let mut screen = Greeter::screen_for(chosen, text.sharing_fonts());
    screen.set_position(user, users.len());
    let system = bande_bus::system()
        .inspect_err(|error| tracing::warn!("no system bus: {error}"))
        .ok();
    let powers = system
        .as_ref()
        .map(|system| {
            powers(system)
                .inspect_err(|error| tracing::warn!("cannot ask logind about power: {error}"))
                .unwrap_or_default()
        })
        .unwrap_or_default();
    screen.offer_power(&powers);
    let picture = login_picture(Path::new(LOGIN_PICTURES), chosen.uid).map(Arc::new);
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
    let devices = system.as_ref().and_then(Devices::follow);
    let mut greeter = Greeter {
        users,
        user,
        screen,
        conversation: None,
        caps_lock: false,
        minutes: MinuteTimer::new()?,
        backdrop: None,
        backdrop_size: None,
        sharp: None,
        picture,
        wallpaper: None,
        last_user,
        layouts: Layouts::load(),
        input_source: None,
        started: false,
        text,
        system,
        powers,
        devices,
        corner: (None, None),
    };
    bande_toolkit::run(&mut window, &mut greeter)?;
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

#[cfg(test)]
mod tests {
    use bande_status::status::Power;

    use super::*;

    fn person(uid: u32, login: &str) -> User {
        User {
            uid,
            login: login.to_owned(),
            full_name: login.to_owned(),
        }
    }

    #[test]
    fn the_battery_and_wifi_stay_in_the_corner_when_another_person_is_chosen() {
        let battery = Battery {
            percent: 12.0,
            power: Power::Battery { remaining: None },
        };
        let wifi = Wifi {
            enabled: true,
            networks: Vec::new(),
            sign_in: false,
        };
        let users = vec![person(1000, "ada"), person(1001, "grace")];
        let text = TextSystem::new();
        let mut screen = Greeter::screen_for(&users[0], text.sharing_fonts());
        screen.set_devices(Some(battery), Some(&wifi));
        let mut greeter = Greeter {
            users,
            user: 0,
            screen,
            conversation: None,
            caps_lock: false,
            minutes: MinuteTimer::new().unwrap(),
            backdrop: None,
            backdrop_size: None,
            sharp: None,
            picture: None,
            wallpaper: None,
            last_user: None,
            layouts: Layouts::load(),
            input_source: None,
            started: false,
            text,
            system: None,
            powers: Vec::new(),
            devices: None,
            corner: (Some(battery), Some(wifi.clone())),
        };

        assert!(greeter.switch_user(true));
        assert!(!greeter.screen.set_devices(Some(battery), Some(&wifi)));
    }
}
