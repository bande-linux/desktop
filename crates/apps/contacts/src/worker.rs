use std::sync::mpsc::{Sender, channel};

use bande_address_book::store::Book;
use bande_address_book::sync::{Outcome, sync_book};
use bande_dav::offer::{self, Offer};
use bande_dav::{Client, Credentials, Error, FoundAddressBooks, Service};
use bande_discovery::Resolver;
use bande_inbox::Outbox;
use bande_keyring::Passwords;

const KEYRING: &str = "org.bande.Contacts";

#[derive(Debug)]
pub enum Job {
    Sync(Vec<(String, String, Book)>),
    Find {
        setup: u64,
        address: String,
    },
    SignIn {
        setup: u64,
        server: String,
        credentials: Credentials,
    },
    SetPassword {
        account: String,
        user: String,
        password: String,
    },
    Forget(String),
    Adopt,
    Offer {
        offer: Offer,
        password: String,
    },
}

#[derive(Debug)]
pub enum Reply {
    Synced(Outcome),
    Failed {
        account: String,
        error: Error,
    },
    Finished,
    Found {
        setup: u64,
        server: Option<String>,
    },
    Signed {
        setup: u64,
        result: Result<FoundAddressBooks, Error>,
    },
    Adopted {
        offer: Offer,
        credentials: Credentials,
        result: Result<FoundAddressBooks, Error>,
    },
    OfferFailed(String),
}

fn run(job: Job, passwords: &mut Passwords, outbox: &Outbox<Reply>) {
    let reply = |reply: Reply| {
        outbox.send(reply);
    };
    match job {
        Job::Sync(books) => {
            for (account, user, book) in books {
                let credentials = passwords
                    .get(&account)
                    .map(|password| Credentials { user, password });
                let client = Client::new(Service::CardDav, credentials.as_ref());
                match sync_book(&client, &book) {
                    Ok(outcome) => reply(Reply::Synced(outcome)),
                    Err(error) => reply(Reply::Failed { account, error }),
                }
            }
            reply(Reply::Finished);
        }
        Job::Find { setup, address } => reply(Reply::Found {
            setup,
            server: Client::find(Service::CardDav, &address, &Resolver::system()),
        }),
        Job::SignIn {
            setup,
            server,
            credentials,
        } => reply(Reply::Signed {
            setup,
            result: Client::new(Service::CardDav, Some(&credentials))
                .discover_address_books(&server),
        }),
        Job::SetPassword {
            account,
            user,
            password,
        } => passwords.set(&account, &format!("Contacts: {user}"), &password),
        Job::Forget(account) => passwords.forget(&account),
        Job::Adopt => {
            let answered = offer::adopt(Service::CardDav, |client, server| {
                client.discover_address_books(server)
            });
            for (offer, credentials, result) in answered {
                reply(Reply::Adopted {
                    offer,
                    credentials,
                    result,
                });
            }
        }
        Job::Offer { offer, password } => {
            if let Err(error) = offer::make(Service::CalDav, &offer, &password) {
                reply(Reply::OfferFailed(error));
            }
        }
    }
}

pub struct Worker {
    jobs: Sender<Job>,
    outbox: Outbox<Reply>,
}

impl Worker {
    pub fn start(outbox: Outbox<Reply>) -> std::io::Result<Self> {
        let (jobs, inbox) = channel::<Job>();
        let replies = outbox.clone();
        std::thread::Builder::new()
            .name("sync".to_owned())
            .spawn(move || {
                let mut passwords = Passwords::new(KEYRING);
                for job in inbox {
                    run(job, &mut passwords, &replies);
                }
            })?;
        Ok(Self { jobs, outbox })
    }

    /// Runs `job`: a lookup or sign-in of the account sheet on a thread of its own, so a long
    /// sync never holds it up; everything else in order on the sync thread.
    pub fn send(&self, job: Job) {
        if matches!(job, Job::Find { .. } | Job::SignIn { .. }) {
            let outbox = self.outbox.clone();
            let spawned = std::thread::Builder::new()
                .name("account-setup".to_owned())
                .spawn(move || run(job, &mut Passwords::new(KEYRING), &outbox));
            if let Err(error) = spawned {
                tracing::warn!("cannot look up the account: {error}");
            }
            return;
        }
        if self.jobs.send(job).is_err() {
            tracing::warn!("the sync thread stopped");
        }
    }
}
