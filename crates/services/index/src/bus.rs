use std::sync::mpsc::{Sender, channel};

use bande_search::{BUS_NAME, Donation, Found, OBJECT_PATH};
use zbus::blocking::Connection;
use zbus::blocking::connection::Builder;

use crate::service::Message;

struct Search {
    messages: Sender<Message>,
}

#[zbus::interface(name = "org.bande.Search")]
impl Search {
    fn find(&self, text: String, within: String, limit: u32) -> Vec<Found> {
        let (reply, answer) = channel();
        let asked = Message::Find {
            text,
            within,
            limit: limit as usize,
            reply,
        };
        if self.messages.send(asked).is_err() {
            return Vec::new();
        }
        answer.recv().unwrap_or_default()
    }

    fn donate(&self, domain: String, items: Vec<Donation>, cursor: String) {
        let _ = self.messages.send(Message::Donate {
            domain,
            items,
            cursor,
        });
    }

    fn cursor(&self, domain: String) -> String {
        let (reply, answer) = channel();
        if self
            .messages
            .send(Message::Cursor { domain, reply })
            .is_err()
        {
            return String::new();
        }
        answer.recv().unwrap_or_default()
    }

    fn forget_all(&self, domain: String) {
        let _ = self.messages.send(Message::ForgetAll { domain });
    }

    fn forget(&self, domain: String, ids: Vec<String>) {
        let _ = self.messages.send(Message::Forget { domain, ids });
    }
}

pub fn start(messages: Sender<Message>) -> zbus::Result<Connection> {
    Builder::session()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, Search { messages })?
        .build()
}
