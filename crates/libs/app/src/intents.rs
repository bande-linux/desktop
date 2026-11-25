use std::os::fd::BorrowedFd;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bande_inbox::{Inbox, Outbox};
use bande_intents::{Owner, Serving};
use rustix::event::{PollFd, PollFlags, poll};
use zbus::blocking::Connection;

use crate::Error;
use crate::instance::{Instance, Request};

const IDLE: Duration = Duration::from_secs(60);
const LATE_CALLS: Duration = Duration::from_millis(200);
const ANSWERED: Duration = Duration::from_secs(2);

struct Question {
    kind: String,
    answer: mpsc::Sender<Vec<String>>,
}

pub(crate) struct Intents {
    serving: Arc<Serving>,
    questions: Inbox<Question>,
    ended: Inbox<()>,
}

fn ask(questions: &Mutex<Outbox<Question>>, kind: &str) -> Vec<String> {
    let (answer, answered) = mpsc::channel();
    let question = Question {
        kind: kind.to_owned(),
        answer,
    };
    questions
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .send(question);
    answered.recv_timeout(ANSWERED).unwrap_or_default()
}

impl Intents {
    pub(crate) fn serve(owner: Owner, session: &Connection) -> Result<Self, Error> {
        let questions = Inbox::new()?;
        let outbox = Mutex::new(questions.outbox());
        let serving = owner
            .current(Arc::new(move |kind| ask(&outbox, kind)))
            .serve(session)?;
        Ok(Self {
            serving: Arc::new(serving),
            questions,
            ended: Inbox::new()?,
        })
    }

    pub(crate) fn fd(&self) -> BorrowedFd<'_> {
        self.questions.fd()
    }

    pub(crate) fn answer(&mut self, current: impl Fn(&str) -> Vec<String>) {
        for question in self.questions.drain() {
            let _ = question.answer.send(current(&question.kind));
        }
    }

    pub(crate) fn wait_for_launch(
        &mut self,
        session: &Connection,
        id: &str,
        instance: &mut Instance,
    ) -> Result<Option<Request>, Error> {
        loop {
            let serving = Arc::clone(&self.serving);
            let ended = self.ended.outbox();
            std::thread::Builder::new()
                .name("idle".to_owned())
                .spawn(move || {
                    serving.wait_until_idle(IDLE);
                    ended.send(());
                })?;
            while self.ended.drain().is_empty() {
                if let Some(request) = instance.take_requests().into_iter().next() {
                    return Ok(Some(request));
                }
                self.answer(|_| Vec::new());
                let mut fds = [instance.fd(), self.questions.fd(), self.ended.fd()]
                    .map(|fd| PollFd::from_borrowed_fd(fd, PollFlags::IN));
                match poll(&mut fds, None) {
                    Ok(_) | Err(rustix::io::Errno::INTR) => {}
                    Err(error) => return Err(std::io::Error::from(error).into()),
                }
            }
            session.release_name(id)?;
            std::thread::sleep(LATE_CALLS);
            if let Some(request) = instance.take_requests().into_iter().next() {
                session.request_name(id)?;
                return Ok(Some(request));
            }
            if !self.serving.is_busy() {
                return Ok(None);
            }
            session.request_name(id)?;
        }
    }

    pub(crate) fn is_working(&self) -> bool {
        self.serving.is_working()
    }

    pub(crate) fn finish(&self, session_ending: bool) {
        match session_ending {
            true => self.serving.stop(),
            false => self.serving.finish(),
        }
    }
}
