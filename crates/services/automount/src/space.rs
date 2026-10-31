use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use unbound_fs::format_size;
use unbound_notify::Urgency;
use unbound_notify::client::{self, Request};
use zbus::blocking::{Connection, MessageIterator};
use zbus::{MatchRule, message};

use crate::L10N;

pub const CHECK_EVERY: Duration = Duration::from_secs(10 * 60);
const GIGABYTE: u64 = 1_000_000_000;
const ALWAYS_LOW: u64 = GIGABYTE;
const NEVER_LOW: u64 = 10 * GIGABYTE;
const LOW_SHARE: u64 = 20;
const MANAGE_STORAGE: &str = "manage-storage";
const SETTINGS: &str = "org.unbound.Settings";

pub fn is_low(total: u64, available: u64) -> bool {
    total > 0
        && (available < ALWAYS_LOW || (available < NEVER_LOW && available * LOW_SHARE < total))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Space {
    pub total: u64,
    pub available: u64,
}

pub fn free_space(path: &Path) -> Option<Space> {
    let stats = rustix::fs::statvfs(path).ok()?;
    Some(Space {
        total: stats.f_blocks * stats.f_frsize,
        available: stats.f_bavail * stats.f_frsize,
    })
}

struct Watched {
    path: PathBuf,
    warned: bool,
}

type Measure = Box<dyn Fn(&Path) -> Option<Space> + Send + Sync>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct SpaceWatch {
    session: Connection,
    measure: Measure,
    watched: Mutex<Vec<Watched>>,
    shown: Mutex<Vec<u32>>,
}

impl SpaceWatch {
    pub fn new(
        session: &Connection,
        paths: &[PathBuf],
        measure: impl Fn(&Path) -> Option<Space> + Send + Sync + 'static,
    ) -> Self {
        let mut devices = Vec::new();
        let mut watched = Vec::new();
        for path in paths {
            let Ok(metadata) = path.metadata() else {
                continue;
            };
            if !devices.contains(&metadata.dev()) {
                devices.push(metadata.dev());
                watched.push(Watched {
                    path: path.clone(),
                    warned: false,
                });
            }
        }
        Self {
            session: session.clone(),
            measure: Box::new(measure),
            watched: Mutex::new(watched),
            shown: Mutex::default(),
        }
    }

    pub fn check(&self) {
        let mut watched = lock(&self.watched);
        for volume in watched.iter_mut() {
            let Some(space) = (self.measure)(&volume.path) else {
                continue;
            };
            let low = is_low(space.total, space.available);
            if low && !volume.warned {
                self.warn(space.available);
            }
            volume.warned = low;
        }
    }

    fn warn(&self, available: u64) {
        let manage = L10N.tr("manage-storage");
        let posted = client::notify(
            &self.session,
            &Request {
                app_name: &L10N.tr("storage"),
                replaces_id: 0,
                app_icon: "drive-harddisk",
                summary: &L10N.tr("disk-almost-full"),
                body: &L10N.tr_args(
                    "disk-almost-full-body",
                    [("free", format_size(available).into())],
                ),
                actions: &[(MANAGE_STORAGE, &manage)],
                urgency: Urgency::Normal,
                sound: None,
            },
        );
        match posted {
            Ok(id) => lock(&self.shown).push(id),
            Err(error) => tracing::warn!("cannot warn that the disk is almost full: {error}"),
        }
    }

    fn notice_signals(&self) -> zbus::Result<MessageIterator> {
        let rule = MatchRule::builder()
            .msg_type(message::Type::Signal)
            .interface("org.freedesktop.Notifications")?
            .build();
        MessageIterator::for_match_rule(rule, &self.session, None)
    }

    fn follow_notices(&self, signals: MessageIterator, open: &dyn Fn()) {
        for signal in signals.flatten() {
            let header = signal.header();
            let body = signal.body();
            match header.member().map(|member| member.as_str()) {
                Some("ActionInvoked") => {
                    if let Ok((id, key)) = body.deserialize::<(u32, String)>()
                        && key == MANAGE_STORAGE
                        && lock(&self.shown).contains(&id)
                    {
                        open();
                    }
                }
                Some("NotificationClosed") => {
                    if let Ok((id, _)) = body.deserialize::<(u32, u32)>() {
                        lock(&self.shown).retain(|shown| *shown != id);
                    }
                }
                _ => {}
            }
        }
    }
}

pub fn open_storage_settings() {
    let command = ["unbound-settings".to_owned(), "storage".to_owned()];
    if let Err(error) = unbound_xdg::spawn_detached(SETTINGS, &command, None, None) {
        tracing::warn!("cannot open Storage settings: {error}");
    }
}

pub fn watch_space(watch: SpaceWatch, open: impl Fn() + Send + 'static) -> Arc<SpaceWatch> {
    let watch = Arc::new(watch);
    match watch.notice_signals() {
        Ok(signals) => {
            let notices = Arc::clone(&watch);
            thread::spawn(move || notices.follow_notices(signals, &open));
        }
        Err(error) => tracing::warn!("Manage Storage cannot be clicked: {error}"),
    }
    let checks = Arc::clone(&watch);
    thread::spawn(move || {
        loop {
            checks.check();
            thread::sleep(CHECK_EVERY);
        }
    });
    watch
}
