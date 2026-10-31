use std::collections::VecDeque;
use std::error::Error;
use std::os::fd::BorrowedFd;
use std::process::ExitCode;
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use accesskit::TreeUpdate;
use bande_inbox::{Inbox, Outbox};
use bande_polkit::agent::{Agent, Answer, Conversation, Notice, Request, Subject};
use bande_polkit::dialog::{Dialog, Message, Prompt};
use bande_polkit::helper::HELPER;
use bande_toolkit::{
    App, Edges, KeyboardFocus, LayerKind, LayerOptions, Role, Scene, TextSystem, Window,
    WindowError, WindowEvent, run,
};
use rustix::event::{PollFd, PollFlags, poll};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedObjectPath;

const LOGIN: &str = "org.freedesktop.login1";
const NAMESPACE: &str = "polkit-authentication";

struct Bridge {
    outbox: Outbox<Message>,
    active: Mutex<Option<Request>>,
    released: Condvar,
}

impl Conversation for Bridge {
    fn respond(&self, request: &Request, user: &str, prompt: &str, _echo: bool) -> Answer {
        {
            let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
            while active.as_ref().is_some_and(|active| active != request) {
                active = self
                    .released
                    .wait(active)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            *active = Some(request.clone());
        }
        tracing::debug!("asking for {} ({prompt})", request.action_id);
        let (reply, answer) = mpsc::channel();
        let prompt = Prompt {
            message: request.message.clone(),
            identities: request.identities.clone(),
            user: user.to_owned(),
        };
        if !self.outbox.send(Message::Prompt { prompt, reply }) {
            return Answer::Cancel;
        }
        answer.recv().unwrap_or(Answer::Cancel)
    }

    fn notice(&self, request: &Request, notice: Notice) {
        let active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        if active.as_ref() == Some(request) {
            self.outbox.send(Message::Notice(notice));
        }
    }

    fn close(&self, request: &Request) {
        let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        if active.as_ref() == Some(request) {
            *active = None;
            self.outbox.send(Message::Close);
            self.released.notify_all();
        }
    }
}

struct Asking<'a> {
    dialog: Dialog,
    inbox: &'a mut Inbox<Message>,
    pending: &'a mut VecDeque<Message>,
}

impl App for Asking<'_> {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.dialog.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        if let Err(error) = window.drive(self.dialog.ui_mut(), event) {
            tracing::warn!("cannot handle the event: {error}");
        }
        if self.dialog.settle_choices() {
            window.request_redraw();
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.inbox.fd()]
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) {
        for message in self.inbox.drain() {
            if self.dialog.finished() || self.dialog.closing() {
                self.pending.push_back(message);
            } else {
                self.dialog.receive(message);
            }
        }
        window.request_redraw();
    }

    fn deadline(&self) -> Option<std::time::Instant> {
        self.dialog.ui().deadline()
    }

    fn tick(&mut self, window: &mut Window, _now: std::time::Instant) {
        window.request_redraw();
    }

    fn finished(&self) -> bool {
        self.dialog.finished()
    }

    fn should_close(&mut self, _window: &mut Window) -> bool {
        self.dialog.cancel();
        true
    }

    fn animating(&self) -> bool {
        self.dialog.ui().is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.dialog.ui().accessibility())
    }
}

fn ask(
    prompt: Prompt,
    reply: mpsc::Sender<Answer>,
    inbox: &mut Inbox<Message>,
    pending: &mut VecDeque<Message>,
) -> Result<bool, WindowError> {
    let mut window = Window::open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Overlay,
            anchor: Edges::ALL,
            exclusive_zone: -1,
            keyboard: KeyboardFocus::Exclusive,
            namespace: NAMESPACE.to_owned(),
        }),
        (0, 0),
    )?;
    let dialog = Dialog::new(prompt, reply, TextSystem::new(), window.appearance());
    let mut asking = Asking {
        dialog,
        inbox,
        pending,
    };
    run(&mut window, &mut asking)?;
    Ok(asking.dialog.declined())
}

fn session(system: &Connection) -> zbus::Result<String> {
    let reply = system.call_method(
        Some(LOGIN),
        "/org/freedesktop/login1",
        Some("org.freedesktop.login1.Manager"),
        "GetSession",
        &("auto",),
    )?;
    let path: OwnedObjectPath = reply.body().deserialize()?;
    Proxy::new(system, LOGIN, path, "org.freedesktop.login1.Session")?.get_property("Id")
}

fn subject(system: &Connection) -> std::io::Result<Subject> {
    match session(system) {
        Ok(id) => Ok(Subject::Session(id)),
        Err(error) => match std::env::var("XDG_SESSION_ID") {
            Ok(id) => Ok(Subject::Session(id)),
            Err(_) => {
                tracing::warn!("no logind session ({error}), authenticating for this process");
                Subject::process(std::process::id())
            }
        },
    }
}

fn next(inbox: &mut Inbox<Message>, pending: &mut VecDeque<Message>) -> std::io::Result<Message> {
    loop {
        if let Some(message) = pending.pop_front() {
            return Ok(message);
        }
        {
            let fd = inbox.fd();
            poll(&mut [PollFd::new(&fd, PollFlags::IN)], None)?;
        }
        pending.extend(inbox.drain());
    }
}

fn serve() -> Result<(), Box<dyn Error>> {
    let system = Connection::system()?;
    let mut inbox = Inbox::new()?;
    let locale = std::env::var("LANG").unwrap_or_else(|_| "C".to_owned());
    let _agent = Agent::register(
        &system,
        subject(&system)?,
        &locale,
        HELPER.into(),
        Arc::new(Bridge {
            outbox: inbox.outbox(),
            active: Mutex::new(None),
            released: Condvar::new(),
        }),
    )?;
    let mut pending = VecDeque::new();
    let mut declined = false;
    loop {
        match next(&mut inbox, &mut pending)? {
            Message::Prompt { reply, .. } if declined => {
                let _ = reply.send(Answer::Cancel);
            }
            Message::Prompt { prompt, reply } => {
                declined = ask(prompt, reply, &mut inbox, &mut pending)?;
            }
            Message::Close => declined = false,
            Message::Notice(_) => {}
        }
    }
}

fn main() -> ExitCode {
    bande_log::init("bande-polkit-agent");
    match serve() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
