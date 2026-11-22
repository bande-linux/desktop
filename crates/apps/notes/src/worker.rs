use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use bande_inbox::Outbox;
use bande_keyring::Keyring;
use bande_l10n::tr;
use bande_mail_core::accounts::{Account, AccountId};
use bande_mail_core::error::{Error, Result};
use bande_mail_core::imap::Session;
use bande_mail_core::net::{Closer, Network};
use bande_mail_core::{KEYRING, keyring_key};

use crate::store::Store;
use crate::sync::{find_tree, sync_account};

const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
const FULL_SYNC_INTERVAL: Duration = Duration::from_secs(15 * 60);
const IDLE_RENEW: Duration = Duration::from_secs(25 * 60);
const FIRST_RETRY: Duration = Duration::from_secs(5);
const LAST_RETRY: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Sync,
    Password(String),
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Changed(AccountId),
    Online(AccountId),
    Offline(AccountId, String),
    LoginFailed(AccountId, String),
    NoFolder(AccountId),
    AccountsChanged,
    Found(Box<bande_mail_core::setup::Found>),
    Checked(Box<bande_mail_core::setup::Outcome>),
}

pub struct Worker {
    pub account: AccountId,
    commands: Sender<Command>,
    stopping: Arc<AtomicBool>,
    idle: Arc<Mutex<Option<Closer>>>,
}

impl Worker {
    pub fn spawn(
        account: Account,
        password: Option<String>,
        store: PathBuf,
        network: Network,
        events: Outbox<Event>,
    ) -> Self {
        let (commands, received) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        let idle = Arc::new(Mutex::new(None));
        let worker = Self {
            account: account.id,
            commands: commands.clone(),
            stopping: Arc::clone(&stopping),
            idle: Arc::clone(&idle),
        };
        let name = format!("notes-{}", account.id);
        let spawned = thread::Builder::new().name(name).spawn(move || {
            let store = match Store::open(&store) {
                Ok(store) => store,
                Err(error) => {
                    events.send(Event::Offline(account.id, error.to_string()));
                    return;
                }
            };
            Runner {
                account,
                store,
                network,
                events,
                password,
                session: None,
                retry: FIRST_RETRY,
                commands: commands.clone(),
                stopping,
                idle,
                idling: Arc::new(AtomicBool::new(false)),
            }
            .run(&received);
        });
        if let Err(error) = spawned {
            tracing::warn!("cannot start syncing notes: {error}");
        }
        worker
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        let _ = self.commands.send(Command::Stop);
        if let Ok(idle) = self.idle.lock()
            && let Some(closer) = idle.as_ref()
        {
            closer.close();
        }
    }
}

struct Runner {
    account: Account,
    store: Store,
    network: Network,
    events: Outbox<Event>,
    password: Option<String>,
    session: Option<Session>,
    retry: Duration,
    commands: Sender<Command>,
    stopping: Arc<AtomicBool>,
    idle: Arc<Mutex<Option<Closer>>>,
    idling: Arc<AtomicBool>,
}

fn connect(account: &Account, password: &str, network: &Network) -> Result<Session> {
    let mut session = Session::connect(&account.imap, network)?;
    session.login(&account.user, password)?;
    Ok(session)
}

impl Runner {
    fn emit(&self, event: Event) {
        self.events.send(event);
    }

    fn password(&mut self) -> Result<String> {
        if self.password.is_none() {
            let key = keyring_key(&self.account.email, &self.account.imap.host);
            self.password = Keyring::open(KEYRING)?.password(&key)?;
        }
        self.password
            .clone()
            .ok_or_else(|| Error::Login(tr!("no password is saved for this account")))
    }

    fn sync(&mut self) -> Result<bool> {
        if self.session.is_none() {
            let password = self.password()?;
            self.session = Some(connect(&self.account, &password, &self.network)?);
        }
        let Some(session) = self.session.as_mut() else {
            return Err(Error::Closed);
        };
        let Some(changed) = sync_account(session, &mut self.store, &self.account)? else {
            return Ok(false);
        };
        let watch = session.has("IDLE");
        self.emit(Event::Online(self.account.id));
        if changed {
            self.emit(Event::Changed(self.account.id));
        }
        if watch && !self.idling.load(Ordering::Relaxed) {
            self.start_idle();
        }
        Ok(true)
    }

    fn attempt(&mut self) -> bool {
        let mut result = self.sync();
        if matches!(&result, Err(error) if error.is_connection()) && self.session.is_some() {
            self.session = None;
            result = self.sync();
        }
        match result {
            Ok(true) => {
                self.retry = FIRST_RETRY;
                true
            }
            Ok(false) => {
                self.emit(Event::NoFolder(self.account.id));
                false
            }
            Err(Error::Login(text)) => {
                self.password = None;
                self.session = None;
                self.emit(Event::LoginFailed(self.account.id, text));
                false
            }
            Err(error) => {
                tracing::warn!("{}: {error}", self.account.email);
                self.session = None;
                self.retry = (self.retry * 2).min(LAST_RETRY);
                self.emit(Event::Offline(self.account.id, error.to_string()));
                true
            }
        }
    }

    fn run(mut self, commands: &mpsc::Receiver<Command>) {
        let mut waiting = !self.attempt();
        while !self.stopping.load(Ordering::Relaxed) {
            let wait = match (self.session.is_some(), self.idling.load(Ordering::Relaxed)) {
                (false, _) => self.retry,
                (true, false) => POLL_INTERVAL,
                (true, true) => FULL_SYNC_INTERVAL,
            };
            let command = match waiting {
                true => commands.recv().map_err(|_| RecvTimeoutError::Disconnected),
                false => commands.recv_timeout(wait),
            };
            match command {
                Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(Command::Password(password)) => {
                    self.password = Some(password);
                    self.session = None;
                }
                Ok(Command::Sync) | Err(RecvTimeoutError::Timeout) => {}
            }
            while let Ok(queued) = commands.try_recv() {
                match queued {
                    Command::Stop => return,
                    Command::Password(password) => self.password = Some(password),
                    Command::Sync => {}
                }
            }
            waiting = !self.attempt();
        }
        if let Some(session) = self.session.take() {
            session.logout();
        }
    }

    fn start_idle(&mut self) {
        let Some(password) = self.password.clone() else {
            return;
        };
        self.idling.store(true, Ordering::Relaxed);
        let idling = Arc::clone(&self.idling);
        let account = self.account.clone();
        let network = self.network.clone();
        let commands = self.commands.clone();
        let stopping = Arc::clone(&self.stopping);
        let closer = Arc::clone(&self.idle);
        let spawned = thread::Builder::new()
            .name(format!("notes-idle-{}", account.id))
            .spawn(move || {
                let watched = connect(&account, &password, &network).and_then(|mut session| {
                    let Some(tree) = find_tree(&mut session)? else {
                        return Ok(());
                    };
                    session.select(&tree.root)?;
                    if let Ok(mut closer) = closer.lock() {
                        *closer = session.closer().ok();
                    }
                    while !stopping.load(Ordering::Relaxed) {
                        if session.idle(IDLE_RENEW)? && commands.send(Command::Sync).is_err() {
                            break;
                        }
                    }
                    Ok(())
                });
                if let Err(error) = watched {
                    tracing::debug!("{} stopped watching notes: {error}", account.email);
                }
                idling.store(false, Ordering::Relaxed);
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot watch for changed notes: {error}");
            self.idling.store(false, Ordering::Relaxed);
        }
    }
}
