use std::io;
use std::os::fd::BorrowedFd;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use bande_inbox::{Inbox, Outbox};
use bande_pam::{Code, Conversation, REFRESH_CRED};
use zeroize::Zeroizing;

use crate::conversation::{Prompt, Refusal, clip_message};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accepted {
    Current,
    PasswordExpired,
}

#[derive(Debug)]
pub enum Said {
    Ask(Prompt),
    Show(String),
    Done(Result<Accepted, Refusal>),
}

struct Talk {
    said: Outbox<Said>,
    answers: Receiver<Zeroizing<String>>,
}

impl Conversation for Talk {
    fn ask(&mut self, prompt: &str, visible: bool) -> Option<Zeroizing<String>> {
        let prompt = Prompt {
            text: clip_message(prompt),
            visible,
        };
        match self.said.send(Said::Ask(prompt)) {
            true => self.answers.recv().ok(),
            false => None,
        }
    }

    fn show(&mut self, message: &str, _error: bool) {
        self.said.send(Said::Show(clip_message(message)));
    }
}

pub struct Transaction {
    said: Inbox<Said>,
    answers: Sender<Zeroizing<String>>,
}

impl Transaction {
    pub fn begin(service: &str, user: &str) -> io::Result<Self> {
        let service = service.to_owned();
        let user = user.to_owned();
        let said = Inbox::new()?;
        let (answers, received) = channel();
        let done = said.outbox();
        let mut talk = Talk {
            said: said.outbox(),
            answers: received,
        };
        thread::Builder::new()
            .name("pam".to_owned())
            .spawn(move || {
                let outcome = authenticate(&service, &user, &mut talk);
                done.send(Said::Done(outcome));
            })?;
        Ok(Self { said, answers })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.said.fd()
    }

    pub fn drain(&mut self) -> Vec<Said> {
        self.said.drain()
    }

    pub fn answer(&self, text: Zeroizing<String>) -> bool {
        self.answers.send(text).is_ok()
    }
}

fn authenticate(service: &str, user: &str, talk: &mut Talk) -> Result<Accepted, Refusal> {
    let mut transaction = bande_pam::Transaction::start(service, user, talk).map_err(|error| {
        tracing::warn!("cannot ask PAM: {error}");
        Refusal::Unavailable
    })?;
    let refused = |code: Code| {
        tracing::info!("PAM refused authentication ({code})");
        Refusal::from_pam(code)
    };
    transaction.authenticate().map_err(refused)?;
    let accepted = match transaction.check_account() {
        Ok(()) => Accepted::Current,
        Err(Code::NEW_AUTHTOK_REQD) => Accepted::PasswordExpired,
        Err(code) => return Err(refused(code)),
    };
    if let Err(code) = transaction.credentials(REFRESH_CRED) {
        tracing::info!("cannot refresh the credentials ({code})");
    }
    Ok(accepted)
}
