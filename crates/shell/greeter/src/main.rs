use std::fs::File;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;
use unbound_config::LOGIN_PICTURES;
use unbound_greeter::{
    L10N, Login, Rejection, Request, Step, last_user_file, login_picture, login_users, opened_path,
    read_message, remember_user, remembered_user, write_message,
};
use unbound_keyboard::Layouts;
use unbound_lock::{
    BackdropSource, DateFormats, Inbox, LockScreen, Look, User, Variant, load_backdrop,
};
use unbound_toolkit::ui::{Event, Key, frame_time};
use unbound_toolkit::{
    App, Appearance, Edges, Image, KeyboardFocus, LayerKind, LayerOptions, Role, Scene, TextSystem,
    Window, WindowEvent,
};
use zeroize::Zeroizing;

const CLOCK_TICK: Duration = Duration::from_secs(1);

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
    next_clock: Instant,
    backdrop: Option<Inbox<(Look, Image)>>,
    backdrop_requested: bool,
    picture: Option<File>,
    wallpaper: Option<Image>,
    last_user: Option<PathBuf>,
    layouts: Layouts,
    input_source: Option<String>,
    started: bool,
    text: TextSystem,
}

impl Greeter {
    fn screen_for(user: &User, text: TextSystem) -> LockScreen {
        LockScreen::new(
            user,
            DateFormats::from_env(),
            &jiff::Zoned::now(),
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
        self.screen.set_input_source(self.input_source.clone());
        self.screen.set_caps_lock(self.caps_lock);
        self.picture = login_picture(Path::new(LOGIN_PICTURES), self.users[self.user].uid);
        self.backdrop = None;
        self.backdrop_requested = false;
        if let Some(wallpaper) = &self.wallpaper {
            self.screen
                .set_backdrop(wallpaper.clone(), Instant::now() - Duration::from_secs(1));
        }
        true
    }

    fn sign_in(&mut self, password: Zeroizing<String>) {
        self.screen.set_message(None);
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
        if !self.backdrop_requested && width > 0 && height > 0 {
            self.backdrop_requested = true;
            let picture = self.picture.as_ref().map(opened_path);
            match load_backdrop(
                BackdropSource::Picture(picture),
                Variant::Dark,
                (width, height),
                vec![Look::Blurred, Look::Sharp],
            ) {
                Ok(inbox) => self.backdrop = Some(inbox),
                Err(error) => tracing::warn!("cannot load the wallpaper: {error}"),
            }
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
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.backdrop
            .as_ref()
            .map(Inbox::fd)
            .into_iter()
            .chain(
                self.conversation
                    .as_ref()
                    .map(|conversation| conversation.greetd.as_fd()),
            )
            .collect()
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        match self.backdrop.as_mut().filter(|_| index == 0) {
            Some(inbox) => {
                for (look, image) in inbox.drain() {
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
            None => self.converse(),
        }
        window.request_redraw();
    }

    fn deadline(&self) -> Option<Instant> {
        match self.finished() {
            true => Some(Instant::now()),
            false => Some(self.next_clock),
        }
    }

    fn animating(&self) -> bool {
        self.screen.is_animating()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        if now >= self.next_clock {
            self.screen.set_clock(&jiff::Zoned::now());
            self.next_clock = now + CLOCK_TICK;
        }
        window.request_redraw();
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
    let picture = login_picture(Path::new(LOGIN_PICTURES), chosen.uid);
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
        conversation: None,
        caps_lock: false,
        next_clock: Instant::now() + CLOCK_TICK,
        backdrop: None,
        backdrop_requested: false,
        picture,
        wallpaper: None,
        last_user,
        layouts: Layouts::load(),
        input_source: None,
        started: false,
        text,
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
