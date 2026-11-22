use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender, channel};

use bande_keyring::Passwords;

use crate::caldav::{Availability, Principal, availability, scheduling, search_principals};
use crate::ical::freebusy::Period;
use crate::store::{AccountKind, Calendar};
use crate::sync::{Outcome, sync_calendar};
use bande_dav::{Client, Credentials, Error, RemoteCalendar, Service};

const KEYRING: &str = "org.bande.Calendar";

#[derive(Debug)]
pub enum Job {
    Sync(Vec<(AccountKind, String, Calendar)>),
    Forget(String),
    SetPassword {
        account: String,
        user: String,
        password: String,
    },
    Collection {
        account: String,
        user: String,
        url: String,
        change: Change,
    },
    Availability {
        account: String,
        user: String,
        home: String,
        organizer: String,
        attendees: Vec<String>,
        window: Period,
    },
    Directory {
        account: String,
        user: String,
        home: String,
        text: String,
    },
    OpenCalendars {
        account: String,
        user: String,
        home: String,
        owner: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Make { name: String, color: String },
    Rename(String),
    Color(String),
    Delete,
}

#[derive(Debug)]
pub enum Reply {
    Synced(Outcome),
    Refreshed {
        calendar: String,
        data: String,
    },
    Failed {
        calendar: String,
        error: String,
    },
    SignInFailed {
        account: String,
    },
    Finished,
    Made {
        account: String,
        url: String,
        name: String,
        color: String,
    },
    ChangeFailed(String),
    Availability(Result<Vec<(String, Availability)>, String>),
    Directory(Result<Vec<Principal>, String>),
    Opened {
        account: String,
        owner: String,
        calendars: Result<Vec<RemoteCalendar>, String>,
    },
}

pub struct Worker {
    jobs: Sender<Job>,
    replies: Receiver<Reply>,
    wake: UnixStream,
}

pub fn http_url(url: &str) -> String {
    let url = url.trim();
    match url.split_once("://") {
        Some((scheme, rest))
            if scheme.eq_ignore_ascii_case("webcal") || scheme.eq_ignore_ascii_case("webcals") =>
        {
            format!("https://{rest}")
        }
        Some(_) => url.to_owned(),
        None => format!("https://{url}"),
    }
}

fn signed_in(passwords: &mut Passwords, account: &str, user: String) -> Client {
    let credentials = passwords
        .get(account)
        .map(|password| Credentials { user, password });
    Client::new(Service::CalDav, credentials.as_ref())
}

fn run(job: Job, passwords: &mut Passwords, reply: &mut dyn FnMut(Reply)) {
    match job {
        Job::Sync(calendars) => {
            for (kind, account, calendar) in calendars {
                let result = match &kind {
                    AccountKind::CalDav { user, .. } => {
                        let credentials = passwords.get(&account).map(|password| Credentials {
                            user: user.clone(),
                            password,
                        });
                        sync_calendar(
                            &Client::new(Service::CalDav, credentials.as_ref()),
                            &calendar,
                        )
                        .map(Reply::Synced)
                    }
                    AccountKind::Subscription { url, .. } => Client::new(Service::CalDav, None)
                        .get(&http_url(url))
                        .map(|resource| Reply::Refreshed {
                            calendar: calendar.info.id.clone(),
                            data: resource.data,
                        }),
                    AccountKind::Local => continue,
                };
                if let Err(Error::Unauthorized) = &result {
                    reply(Reply::SignInFailed {
                        account: account.clone(),
                    });
                }
                reply(result.unwrap_or_else(|error: Error| Reply::Failed {
                    calendar: calendar.info.id.clone(),
                    error: error.to_string(),
                }));
            }
            reply(Reply::Finished);
        }
        Job::SetPassword {
            account,
            user,
            password,
        } => {
            passwords.set(&account, &format!("Calendar: {user}"), &password);
        }
        Job::Collection {
            account,
            user,
            url,
            change,
        } => {
            let client = signed_in(passwords, &account, user);
            let result = match &change {
                Change::Make { name, color } => client.make_calendar(&url, name, color),
                Change::Rename(name) => client.rename(&url, name),
                Change::Color(color) => client.set_color(&url, color),
                Change::Delete => client.delete(&url, None),
            };
            match (result, change) {
                (Ok(()), Change::Make { name, color }) => reply(Reply::Made {
                    account,
                    url,
                    name,
                    color,
                }),
                (Ok(()), _) => {}
                (Err(error), _) => reply(Reply::ChangeFailed(error.to_string())),
            }
        }
        Job::Availability {
            account,
            user,
            home,
            organizer,
            attendees,
            window,
        } => {
            let client = signed_in(passwords, &account, user);
            let answers = scheduling(&client, &home)
                .and_then(|found| availability(&client, &found, &organizer, &attendees, window))
                .map_err(|error| error.to_string());
            reply(Reply::Availability(answers));
        }
        Job::Directory {
            account,
            user,
            home,
            text,
        } => {
            let client = signed_in(passwords, &account, user);
            let found = scheduling(&client, &home)
                .and_then(|found| search_principals(&client, &found.principal, &text))
                .map_err(|error| error.to_string());
            reply(Reply::Directory(found));
        }
        Job::OpenCalendars {
            account,
            user,
            home,
            owner,
        } => {
            let calendars = signed_in(passwords, &account, user)
                .calendars(&home)
                .map_err(|error| error.to_string());
            reply(Reply::Opened {
                account,
                owner,
                calendars,
            });
        }
        Job::Forget(account) => passwords.forget(&account),
    }
}

impl Worker {
    pub fn start() -> std::io::Result<Self> {
        let (jobs, inbox) = channel::<Job>();
        let (outbox, replies) = channel::<Reply>();
        let (wake, mut waker) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        std::thread::Builder::new()
            .name("sync".to_owned())
            .spawn(move || {
                let mut passwords = Passwords::new(KEYRING);
                for job in inbox {
                    run(job, &mut passwords, &mut |reply| {
                        if outbox.send(reply).is_ok() {
                            let _ = waker.write_all(&[1]);
                        }
                    });
                }
            })?;
        Ok(Self {
            jobs,
            replies,
            wake,
        })
    }

    pub fn send(&self, job: Job) {
        if self.jobs.send(job).is_err() {
            tracing::warn!("the sync thread stopped");
        }
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }

    pub fn take(&mut self) -> Vec<Reply> {
        let mut buffer = [0u8; 64];
        while matches!((&self.wake).read(&mut buffer), Ok(read) if read > 0) {}
        self.replies.try_iter().collect()
    }
}
