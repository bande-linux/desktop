use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;

use bande_graphics::{Event, Modifiers};
use bande_protocols::client::bande_app_v1::{self, BandeAppV1};
use bande_protocols::client::xdg_session_manager_v1::{self, XdgSessionManagerV1};
use bande_protocols::client::xdg_session_v1::{self, XdgSessionV1};
use wayland_client::backend::WaylandError;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use xkbcommon::xkb::Keysym;

use crate::seat::key_for;
use crate::window::{Role, Window, WindowError};

const KEEP_RUNNING_SINCE: u32 = 2;
const APP_VERSION: u32 = 10;

#[derive(Debug, Clone, PartialEq)]
pub enum AppRequest {
    Reopen(String),
    Key(Event),
    SessionEnding,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInSession {
    pub(crate) session: XdgSessionV1,
    pub(crate) name: String,
    pub(crate) restoring: bool,
}

#[derive(Default)]
struct Link {
    global: Option<(WlRegistry, u32, u32)>,
    app: Option<BandeAppV1>,
    session_manager: Option<XdgSessionManagerV1>,
    session: Option<XdgSessionV1>,
    session_id: Option<String>,
    requests: Vec<AppRequest>,
}

pub struct AppConnection {
    connection: Connection,
    queue: EventQueue<Link>,
    link: Link,
    app_id: String,
    kept: bool,
}

fn modifiers(bits: u32) -> Modifiers {
    Modifiers {
        shift: bits & 1 != 0,
        control: bits & 2 != 0,
        alt: bits & 4 != 0,
        logo: bits & 8 != 0,
    }
}

pub(crate) fn modifier_bits(modifiers: Modifiers) -> u32 {
    [
        modifiers.shift,
        modifiers.control,
        modifiers.alt,
        modifiers.logo,
    ]
    .into_iter()
    .enumerate()
    .filter(|(_, held)| *held)
    .fold(0, |bits, (bit, _)| bits | 1 << bit)
}

impl AppConnection {
    pub fn connect(app_id: &str) -> Result<Self, WindowError> {
        Self::on(
            Connection::connect_to_env().map_err(WindowError::Connect)?,
            app_id,
        )
    }

    pub fn connect_on(stream: UnixStream, app_id: &str) -> Result<Self, WindowError> {
        Self::on(
            Connection::from_socket(stream).map_err(WindowError::Connect)?,
            app_id,
        )
    }

    fn on(connection: Connection, app_id: &str) -> Result<Self, WindowError> {
        let mut queue = connection.new_event_queue();
        connection.display().get_registry(&queue.handle(), ());
        let mut link = Link::default();
        queue.roundtrip(&mut link)?;
        Ok(Self {
            connection,
            queue,
            link,
            app_id: app_id.to_owned(),
            kept: false,
        })
    }

    pub fn keeps_running(&self) -> bool {
        self.link.app.is_some()
    }

    pub fn is_kept(&self) -> bool {
        self.kept
    }

    pub fn keep_running(&mut self) {
        if let Some(app) = self.link.app.as_ref().filter(|_| !self.kept) {
            app.keep_running(self.app_id.clone());
            self.kept = true;
        }
    }

    pub fn run_in_background(&mut self) {
        if !std::mem::take(&mut self.kept) {
            return;
        }
        if let Some(app) = self.link.app.take() {
            app.destroy();
        }
        self.link.app = self.link.global.as_ref().map(|(registry, name, version)| {
            registry.bind(*name, *version, &self.queue.handle(), ())
        });
    }

    pub fn join_session(&mut self, saved: Option<&str>) -> Result<Option<String>, WindowError> {
        let Some(manager) = &self.link.session_manager else {
            return Ok(None);
        };
        let reason = match saved {
            Some(_) => xdg_session_manager_v1::Reason::SessionRestore,
            None => xdg_session_manager_v1::Reason::Launch,
        };
        self.link.session_id = saved.map(str::to_owned);
        self.link.session =
            Some(manager.get_session(reason, saved.map(str::to_owned), &self.queue.handle(), ()));
        self.queue.roundtrip(&mut self.link)?;
        Ok(self.link.session_id.clone())
    }

    pub fn window_in_session(&self, name: String, restoring: bool) -> Option<WindowInSession> {
        Some(WindowInSession {
            session: self.link.session.clone()?,
            name,
            restoring,
        })
    }

    pub fn leave_session(&self, name: &str) {
        if let Some(session) = &self.link.session {
            session.remove_toplevel(name.to_owned());
        }
    }

    pub fn end_session(&mut self) {
        if let Some(session) = self.link.session.take() {
            session.remove();
        }
    }

    pub fn open(&self, role: Role, size: (u32, u32)) -> Result<Window, WindowError> {
        Window::on(
            self.connection.clone(),
            role,
            size,
            None,
            crate::lock::wanted_output(),
        )
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.connection.as_fd()
    }

    pub fn flush(&self) -> Result<(), WindowError> {
        self.connection.flush().map_err(WindowError::from)
    }

    pub fn take_requests(&mut self) -> Result<Vec<AppRequest>, WindowError> {
        self.queue.dispatch_pending(&mut self.link)?;
        Ok(std::mem::take(&mut self.link.requests))
    }

    pub fn read(&mut self) -> Result<Vec<AppRequest>, WindowError> {
        if let Some(guard) = self.queue.prepare_read() {
            match guard.read() {
                Ok(_) => {}
                Err(WaylandError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            }
        }
        self.take_requests()
    }
}

impl Dispatch<WlRegistry, ()> for Link {
    fn event(
        link: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "bande_app_v1" if version >= KEEP_RUNNING_SINCE => {
                let version = version.min(APP_VERSION);
                link.app = Some(registry.bind(name, version, handle, ()));
                link.global = Some((registry.clone(), name, version));
            }
            "xdg_session_manager_v1" => {
                link.session_manager = Some(registry.bind(name, 1, handle, ()));
            }
            _ => {}
        }
    }
}

impl Dispatch<BandeAppV1, ()> for Link {
    fn event(
        link: &mut Self,
        _: &BandeAppV1,
        event: bande_app_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let request = match event {
            bande_app_v1::Event::SessionEnding => AppRequest::SessionEnding,
            bande_app_v1::Event::QuitRequested => AppRequest::Quit,
            bande_app_v1::Event::Reopen { token } => AppRequest::Reopen(token),
            bande_app_v1::Event::Key {
                keysym,
                modifiers: bits,
            } => match key_for(Keysym::new(keysym)) {
                Some(key) => AppRequest::Key(Event::Key {
                    key,
                    modifiers: modifiers(bits),
                }),
                None => return,
            },
            _ => return,
        };
        link.requests.push(request);
    }
}

impl Dispatch<XdgSessionManagerV1, ()> for Link {
    fn event(
        _: &mut Self,
        _: &XdgSessionManagerV1,
        _: xdg_session_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<XdgSessionV1, ()> for Link {
    fn event(
        link: &mut Self,
        session: &XdgSessionV1,
        event: xdg_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_session_v1::Event::Created { session_id } => link.session_id = Some(session_id),
            xdg_session_v1::Event::Replaced => {
                session.destroy();
                link.session = None;
                link.session_id = None;
            }
            _ => {}
        }
    }
}
