use std::collections::HashSet;
use std::sync::mpsc;
use std::sync::{Condvar, Mutex, PoisonError};

use bande_inbox::Outbox;

use crate::agent::{Answer, Conversation, Notice, Request};
use crate::dialog::{Message, Prompt};

#[derive(Default)]
struct Turns {
    active: Option<Request>,
    waiting: HashSet<String>,
    withdrawn: HashSet<String>,
}

pub struct Bridge {
    outbox: Outbox<Message>,
    turns: Mutex<Turns>,
    released: Condvar,
}

impl Bridge {
    pub fn new(outbox: Outbox<Message>) -> Self {
        Self {
            outbox,
            turns: Mutex::new(Turns::default()),
            released: Condvar::new(),
        }
    }
}

impl Conversation for Bridge {
    fn respond(&self, request: &Request, user: &str, prompt: &str, _echo: bool) -> Answer {
        {
            let mut turns = self.turns.lock().unwrap_or_else(PoisonError::into_inner);
            turns.waiting.insert(request.cookie.clone());
            while turns
                .active
                .as_ref()
                .is_some_and(|active| active.cookie != request.cookie)
                && !turns.withdrawn.contains(&request.cookie)
            {
                turns = self
                    .released
                    .wait(turns)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            turns.waiting.remove(&request.cookie);
            if turns.withdrawn.remove(&request.cookie) {
                return Answer::Cancel;
            }
            turns.active = Some(request.clone());
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
        let turns = self.turns.lock().unwrap_or_else(PoisonError::into_inner);
        if turns
            .active
            .as_ref()
            .is_some_and(|active| active.cookie == request.cookie)
        {
            self.outbox.send(Message::Notice(notice));
        }
    }

    fn close(&self, request: &Request) {
        let mut turns = self.turns.lock().unwrap_or_else(PoisonError::into_inner);
        if turns
            .active
            .as_ref()
            .is_some_and(|active| active.cookie == request.cookie)
        {
            turns.active = None;
            self.outbox.send(Message::Close);
        } else if turns.waiting.contains(&request.cookie) {
            turns.withdrawn.insert(request.cookie.clone());
        }
        self.released.notify_all();
    }
}
