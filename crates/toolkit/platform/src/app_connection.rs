use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;

use bande_graphics::{Event, Modifiers};
use bande_protocols::client::bande_app_v1::{self, BandeAppV1};
use wayland_client::backend::WaylandError;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use xkbcommon::xkb::Keysym;

use crate::seat::key_for;
use crate::window::{Role, Window, WindowError};

const KEEP_RUNNING_SINCE: u32 = 2;

#[derive(Debug, Clone, PartialEq)]
pub enum AppRequest {
    Reopen(String),
    Key(Event),
    Quit,
}

#[derive(Default)]
struct Link {
    app: Option<BandeAppV1>,
    requests: Vec<AppRequest>,
}

pub struct AppConnection {
    connection: Connection,
    queue: EventQueue<Link>,
    link: Link,
}

fn modifiers(bits: u32) -> Modifiers {
    Modifiers {
        shift: bits & 1 != 0,
        control: bits & 2 != 0,
        alt: bits & 4 != 0,
        logo: bits & 8 != 0,
    }
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
        if let Some(app) = &link.app {
            app.keep_running(app_id.to_owned());
        }
        connection.flush()?;
        Ok(Self {
            connection,
            queue,
            link,
        })
    }

    pub fn keeps_running(&self) -> bool {
        self.link.app.is_some()
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
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
            && interface == "bande_app_v1"
            && version >= KEEP_RUNNING_SINCE
        {
            link.app = Some(registry.bind(name, KEEP_RUNNING_SINCE, handle, ()));
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
