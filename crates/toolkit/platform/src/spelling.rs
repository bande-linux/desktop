use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::thread;
use std::time::Duration;

use bande_widgets::{SpellReply, SpellRequest, install_spelling, receive_spelling};
use zbus::blocking::{Connection, Proxy};

const BUS_NAME: &str = "org.bande.Spell";
const OBJECT_PATH: &str = "/org/bande/Spell";
const SUGGESTIONS_WAIT: Duration = Duration::from_secs(1);

struct Delivery {
    replies: Mutex<Receiver<SpellReply>>,
    woken: UnixStream,
}

static DELIVERY: OnceLock<Delivery> = OnceLock::new();

#[derive(Clone)]
struct Replier {
    replies: Sender<SpellReply>,
    wake: std::sync::Arc<UnixStream>,
}

impl Replier {
    fn send(&self, reply: SpellReply) {
        if self.replies.send(reply).is_ok() {
            let _ = (&*self.wake).write(&[1]);
        }
    }
}

pub(crate) fn start() {
    if DELIVERY.get().is_some() {
        return;
    }
    let Ok((wake, woken)) = UnixStream::pair() else {
        return;
    };
    if woken.set_nonblocking(true).is_err() || wake.set_nonblocking(true).is_err() {
        return;
    }
    let (replies, received) = channel();
    let delivery = Delivery {
        replies: Mutex::new(received),
        woken,
    };
    if DELIVERY.set(delivery).is_err() {
        return;
    }
    let replier = Replier {
        replies,
        wake: std::sync::Arc::new(wake),
    };
    let worker: Mutex<Option<Sender<SpellRequest>>> = Mutex::new(None);
    install_spelling(move |request| {
        let mut worker = worker.lock().unwrap_or_else(PoisonError::into_inner);
        let sender = worker.get_or_insert_with(|| {
            let (sender, requests) = channel();
            let replier = replier.clone();
            thread::spawn(move || serve(&requests, &replier));
            sender
        });
        let _ = sender.send(request);
    });
}

pub(crate) fn fd() -> Option<BorrowedFd<'static>> {
    DELIVERY.get().map(|delivery| delivery.woken.as_fd())
}

pub(crate) fn deliver() {
    let Some(delivery) = DELIVERY.get() else {
        return;
    };
    let mut drained = [0; 64];
    while matches!((&delivery.woken).read(&mut drained), Ok(read) if read > 0) {}
    let replies = delivery
        .replies
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    for reply in replies.try_iter() {
        receive_spelling(reply);
    }
}

fn is_absent(error: zbus::Error) -> bool {
    matches!(
        zbus::fdo::Error::from(error),
        zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::NameHasNoOwner(_)
    )
}

fn follow_changes(connection: Connection, replier: Replier) {
    let Ok(proxy) = Proxy::new(&connection, BUS_NAME, OBJECT_PATH, BUS_NAME) else {
        return;
    };
    let Ok(changes) = proxy.receive_signal("Changed") else {
        return;
    };
    for _ in changes {
        replier.send(SpellReply::Changed);
    }
}

fn suggest(connection: Connection, word: String, replier: Replier) {
    let (answer, answered) = channel();
    let asked = word.clone();
    thread::spawn(move || {
        let found = Proxy::new(&connection, BUS_NAME, OBJECT_PATH, BUS_NAME)
            .and_then(|proxy| proxy.call::<_, _, Vec<String>>("Suggest", &(&asked,)));
        let _ = answer.send(found.unwrap_or_default());
    });
    let suggestions = answered.recv_timeout(SUGGESTIONS_WAIT).unwrap_or_default();
    replier.send(SpellReply::Suggested(word, suggestions));
}

fn serve(requests: &Receiver<SpellRequest>, replier: &Replier) {
    let connected = bande_bus::session().and_then(|connection| {
        let proxy = Proxy::new(&connection, BUS_NAME, OBJECT_PATH, BUS_NAME)?;
        Ok((connection, proxy))
    });
    let Ok((connection, proxy)) = connected else {
        replier.send(SpellReply::Absent);
        return;
    };
    let follower = (connection.clone(), replier.clone());
    thread::spawn(move || follow_changes(follower.0, follower.1));
    for request in requests {
        match request {
            SpellRequest::Check(words) => {
                match proxy.call::<_, _, Vec<bool>>("Check", &(&words,)) {
                    Ok(verdicts) => replier.send(SpellReply::Checked(
                        words.into_iter().zip(verdicts).collect(),
                    )),
                    Err(error) => {
                        if is_absent(error) {
                            replier.send(SpellReply::Absent);
                            return;
                        }
                        replier.send(SpellReply::Failed(words));
                    }
                }
            }
            SpellRequest::Suggest(word) => {
                let (connection, replier) = (connection.clone(), replier.clone());
                thread::spawn(move || suggest(connection, word, replier));
            }
            SpellRequest::Correct(word) => {
                let found = proxy
                    .call::<_, _, String>("Correction", &(&word,))
                    .unwrap_or_default();
                replier.send(SpellReply::Corrected(word, found));
            }
            SpellRequest::Rules => {
                if let Ok((rules, replacements)) = proxy.call("Rules", &()) {
                    replier.send(SpellReply::Rules(rules, replacements));
                }
            }
            SpellRequest::Learn(word) => {
                if let Err(error) = proxy.call::<_, _, ()>("Learn", &(&word,)) {
                    tracing::warn!("cannot learn a word: {error}");
                }
            }
        }
    }
}
