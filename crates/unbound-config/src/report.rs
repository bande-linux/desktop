use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::Value;
use zbus::{MatchRule, message};

use crate::ReloadError;

const NOTIFICATIONS: &str = "org.freedesktop.Notifications";
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";
pub const EDIT_ACTION: &str = "edit";

pub struct ErrorNotifier {
    connection: Connection,
    path: PathBuf,
    shown: Arc<Mutex<Option<u32>>>,
}

fn lock(shown: &Mutex<Option<u32>>) -> MutexGuard<'_, Option<u32>> {
    shown.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ErrorNotifier {
    pub fn start(
        connection: &Connection,
        path: &Path,
        open: impl Fn(&Path) + Send + 'static,
    ) -> zbus::Result<Self> {
        let rule = MatchRule::builder()
            .msg_type(message::Type::Signal)
            .interface(NOTIFICATIONS)?
            .path(NOTIFICATIONS_PATH)?
            .build();
        let signals = MessageIterator::for_match_rule(rule, connection, None)?;
        let shown = Arc::new(Mutex::new(None));
        let watched = Arc::clone(&shown);
        let opened = path.to_path_buf();
        thread::spawn(move || {
            for signal in signals.flatten() {
                let header = signal.header();
                let body = signal.body();
                match header.member().map(|member| member.as_str()) {
                    Some("ActionInvoked") => {
                        if let Ok((id, key)) = body.deserialize::<(u32, String)>()
                            && key == EDIT_ACTION
                            && *lock(&watched) == Some(id)
                        {
                            open(&opened);
                        }
                    }
                    Some("NotificationClosed") => {
                        if let Ok((id, _)) = body.deserialize::<(u32, u32)>() {
                            let mut shown = lock(&watched);
                            if *shown == Some(id) {
                                *shown = None;
                            }
                        }
                    }
                    _ => {}
                }
            }
        });
        Ok(Self {
            connection: connection.clone(),
            path: path.to_path_buf(),
            shown,
        })
    }

    pub fn report(&self, result: &Result<(), ReloadError>) {
        let mut shown = lock(&self.shown);
        let file = self
            .path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let (summary, detail) = match result {
            Ok(()) => {
                if let Some(id) = shown.take() {
                    let _ = self.connection.call_method(
                        Some(NOTIFICATIONS),
                        NOTIFICATIONS_PATH,
                        Some(NOTIFICATIONS),
                        "CloseNotification",
                        &id,
                    );
                }
                return;
            }
            Err(ReloadError::Invalid(error)) => (
                format!("{file} has an error"),
                format!(
                    "Line {}, column {}: {}.",
                    error.line, error.column, error.message
                ),
            ),
            Err(ReloadError::Read(error)) => {
                (format!("{file} cannot be read"), format!("{error}."))
            }
        };
        let body = format!("{detail} The previous settings stay in effect.");
        let hints: HashMap<&str, Value<'_>> = HashMap::new();
        let reply = self.connection.call_method(
            Some(NOTIFICATIONS),
            NOTIFICATIONS_PATH,
            Some(NOTIFICATIONS),
            "Notify",
            &(
                "Unbound",
                shown.unwrap_or(0),
                "preferences-system",
                summary,
                body,
                vec![EDIT_ACTION, "Open in Editor"],
                hints,
                -1i32,
            ),
        );
        match reply.and_then(|reply| reply.body().deserialize::<u32>()) {
            Ok(id) => *shown = Some(id),
            Err(error) => eprintln!("unbound-config: cannot post the error notification: {error}"),
        }
    }
}
