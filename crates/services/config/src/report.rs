use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use unbound_notify::client::{self, Request};
use unbound_notify::{BUS_NAME as NOTIFICATIONS, OBJECT_PATH as NOTIFICATIONS_PATH, Urgency};
use zbus::blocking::{Connection, MessageIterator};
use zbus::{MatchRule, message};

use crate::{L10N, ReloadError};

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
                    let _ = client::close(&self.connection, id);
                }
                return;
            }
            Err(ReloadError::Invalid(error)) => (
                L10N.tr_args("has-error", [("file", file.as_str().into())]),
                L10N.tr_args(
                    "error-at",
                    [
                        ("line", error.line.into()),
                        ("column", error.column.into()),
                        ("message", error.message.as_str().into()),
                    ],
                ),
            ),
            Err(ReloadError::Read(error)) => (
                L10N.tr_args("cannot-read", [("file", file.as_str().into())]),
                L10N.tr_args("read-error", [("error", error.to_string().into())]),
            ),
        };
        let body = L10N.tr_args("previous-settings-stay", [("detail", detail.into())]);
        let open_in_editor = L10N.tr("open-in-editor");
        let reply = client::notify(
            &self.connection,
            &Request {
                app_name: "Unbound",
                replaces_id: shown.unwrap_or(0),
                app_icon: "preferences-system",
                summary: &summary,
                body: &body,
                actions: &[(EDIT_ACTION, &open_in_editor)],
                urgency: Urgency::Normal,
                sound: None,
            },
        );
        match reply {
            Ok(id) => *shown = Some(id),
            Err(error) => tracing::warn!("cannot post the error notification: {error}"),
        }
    }
}
