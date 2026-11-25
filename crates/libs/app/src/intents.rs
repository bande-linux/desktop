use std::os::fd::BorrowedFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use bande_inbox::{Inbox, Outbox};
use bande_intents::{Owner, Refusal, Serving};
use bande_l10n::tr;
use rustix::event::{PollFd, PollFlags, poll};
use zbus::blocking::Connection;

use crate::instance::{Instance, Request};
use crate::{Cx, Error, Program};

const IDLE: Duration = Duration::from_secs(60);
const LATE_CALLS: Duration = Duration::from_millis(200);
const ANSWERED: Duration = Duration::from_secs(2);

type Errand<P> = Box<dyn FnOnce(&mut P, &mut Cx<P>) + Send>;

enum Message<P: Program> {
    Errand(Errand<P>),
    Launch(Vec<String>),
}

/// Reaches the program and its windows from the threads its intents are performed on.
pub struct MainLoop<P: Program> {
    messages: Outbox<Message<P>>,
    running: Arc<AtomicBool>,
}

impl<P: Program> Clone for MainLoop<P> {
    fn clone(&self) -> Self {
        Self {
            messages: self.messages.clone(),
            running: Arc::clone(&self.running),
        }
    }
}

impl<P: Program> MainLoop<P> {
    /// Runs `errand` on the program's own thread and waits for its answer: `None` when the process
    /// only serves intents, so nothing is open, and `Unavailable` when the program is busy, in which
    /// case `errand` is never run.
    pub fn run<T: Send + 'static>(
        &self,
        errand: impl FnOnce(&mut P, &mut Cx<P>) -> T + Send + 'static,
    ) -> Result<Option<T>, Refusal> {
        if !self.running.load(Ordering::Acquire) {
            return Ok(None);
        }
        let taken = Arc::new(AtomicBool::new(false));
        let mine = Arc::clone(&taken);
        let (answer, answered) = mpsc::channel();
        let sent = self
            .messages
            .send(Message::Errand(Box::new(move |program, cx| {
                if !mine.swap(true, Ordering::AcqRel) {
                    let _ = answer.send(errand(program, cx));
                }
            })));
        if !sent {
            return Ok(None);
        }
        match answered.recv_timeout(ANSWERED) {
            Ok(answer) => Ok(Some(answer)),
            Err(RecvTimeoutError::Disconnected) => Ok(None),
            Err(RecvTimeoutError::Timeout) => match taken.swap(true, Ordering::AcqRel) {
                true => Ok(answered.recv().ok()),
                false => Err(Refusal::unavailable(tr!(
                    "The app is busy. Try again in a moment."
                ))),
            },
        }
    }

    /// Opens `arguments` as a launch with them does, for an intent that shows something.
    pub fn launch(&self, arguments: Vec<String>) {
        self.messages.send(Message::Launch(arguments));
    }
}

pub(crate) struct Intents<P: Program> {
    serving: OnceLock<Arc<Serving>>,
    messages: Inbox<Message<P>>,
    ended: Inbox<()>,
    main_loop: MainLoop<P>,
}

impl<P: Program> Intents<P> {
    pub(crate) fn new() -> Result<Self, Error> {
        let messages = Inbox::new()?;
        let main_loop = MainLoop {
            messages: messages.outbox(),
            running: Arc::default(),
        };
        Ok(Self {
            serving: OnceLock::new(),
            messages,
            ended: Inbox::new()?,
            main_loop,
        })
    }

    pub(crate) fn owner(&self) -> Option<Owner> {
        P::intents(&self.main_loop)
    }

    pub(crate) fn serve(session: &Connection) -> Result<Option<Self>, Error> {
        let intents = Self::new()?;
        let Some(owner) = intents.owner() else {
            return Ok(None);
        };
        let main_loop = intents.main_loop.clone();
        let current = move |kind: &str| {
            let kind = kind.to_owned();
            main_loop
                .run(move |program, cx| program.current(cx, &kind))
                .inspect_err(|refusal| tracing::warn!("what is open is unknown: {refusal}"))
                .ok()
                .flatten()
                .unwrap_or_default()
        };
        let serving = owner.current(Arc::new(current)).serve(session)?;
        intents.serving.get_or_init(|| Arc::new(serving));
        Ok(Some(intents))
    }

    pub(crate) fn fd(&self) -> BorrowedFd<'_> {
        self.messages.fd()
    }

    /// From now on errands wait for the program.
    pub(crate) fn begin(&self) {
        self.main_loop.running.store(true, Ordering::Release);
    }

    /// Runs the errands, and gives the launches that were asked for.
    pub(crate) fn run_errands(&mut self, program: &mut P, cx: &mut Cx<P>) -> Vec<Request> {
        let mut launches = Vec::new();
        for message in self.messages.drain() {
            match message {
                Message::Errand(errand) => errand(program, cx),
                Message::Launch(arguments) => launches.push(Request::opening(arguments)),
            }
        }
        launches
    }

    fn take_launch(&mut self, instance: &mut Instance) -> Option<Request> {
        for message in self.messages.drain() {
            if let Message::Launch(arguments) = message {
                instance.add_request(Request::opening(arguments));
            }
        }
        instance.take_first_request()
    }

    pub(crate) fn wait_for_launch(
        &mut self,
        session: &Connection,
        id: &str,
        instance: &mut Instance,
    ) -> Result<Option<Request>, Error> {
        loop {
            let serving = self.serving.get().map(Arc::clone);
            let ended = self.ended.outbox();
            std::thread::Builder::new()
                .name("idle".to_owned())
                .spawn(move || {
                    if let Some(serving) = serving {
                        serving.wait_until_idle(IDLE);
                    }
                    ended.send(());
                })?;
            while self.ended.drain().is_empty() {
                if let Some(request) = self.take_launch(instance) {
                    return Ok(Some(request));
                }
                let mut fds = [instance.fd(), self.messages.fd(), self.ended.fd()]
                    .map(|fd| PollFd::from_borrowed_fd(fd, PollFlags::IN));
                match poll(&mut fds, None) {
                    Ok(_) | Err(rustix::io::Errno::INTR) => {}
                    Err(error) => return Err(std::io::Error::from(error).into()),
                }
            }
            session.release_name(id)?;
            std::thread::sleep(LATE_CALLS);
            if let Some(request) = self.take_launch(instance) {
                session.request_name(id)?;
                return Ok(Some(request));
            }
            if !self.serving.get().is_some_and(|serving| serving.is_busy()) {
                return Ok(None);
            }
            session.request_name(id)?;
        }
    }

    pub(crate) fn is_working(&self) -> bool {
        self.serving
            .get()
            .is_some_and(|serving| serving.is_working())
    }

    /// Waits for the jobs, for which nothing is open any more.
    pub(crate) fn finish(&mut self, session_ending: bool) {
        self.main_loop.running.store(false, Ordering::Release);
        self.messages.drain();
        if let Some(serving) = self.serving.get() {
            match session_ending {
                true => serving.stop(),
                false => serving.finish(),
            }
        }
    }
}

impl<P: Program> Drop for Intents<P> {
    fn drop(&mut self) {
        if self.is_working() {
            self.finish(false);
        }
    }
}
