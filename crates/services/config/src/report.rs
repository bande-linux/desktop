use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use bande_l10n::{tr, trn};
use bande_notify::client::{self, Request};
use bande_notify::{BUS_NAME as NOTIFICATIONS, OBJECT_PATH as NOTIFICATIONS_PATH, Urgency};
use zbus::blocking::{Connection, MessageIterator};
use zbus::{MatchRule, message};

use crate::{ConfigError, ReloadError};

pub const EDIT_ACTION: &str = "edit";

pub struct ErrorNotifier {
    connection: Connection,
    path: PathBuf,
    shown: Arc<Mutex<Option<u32>>>,
    reported: Mutex<Vec<String>>,
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
            reported: Mutex::new(Vec::new()),
        })
    }

    pub fn report(&self, result: Result<&[ConfigError], &ReloadError>) {
        let mut shown = lock(&self.shown);
        let mut reported = self.reported.lock().unwrap_or_else(PoisonError::into_inner);
        let file = self
            .path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let file = file.as_str();
        let (summary, body) = match result {
            Ok([]) => {
                reported.clear();
                if let Some(id) = shown.take() {
                    let _ = client::close(&self.connection, id);
                }
                return;
            }
            Ok(problems @ [first, rest @ ..]) => {
                let messages: Vec<String> = problems
                    .iter()
                    .map(|problem| problem.message.clone())
                    .collect();
                if *reported == messages {
                    return;
                }
                *reported = messages;
                let summary = match rest {
                    [] => tr!("{file} has an error", file = file),
                    _ => tr!(
                        "{file} has {count} errors",
                        file = file,
                        count = problems.len()
                    ),
                };
                let mut detail = located(first);
                if !rest.is_empty() {
                    detail.push(' ');
                    detail.push_str(&trn!(
                        "One more line has an error.",
                        "{count} more lines have errors.",
                        count = rest.len()
                    ));
                }
                (
                    summary,
                    tr!(
                        "{detail} Every other setting is in effect.",
                        detail = detail
                    ),
                )
            }
            Err(error) => {
                reported.clear();
                let (summary, detail) = match error {
                    ReloadError::Invalid(error) => {
                        (tr!("{file} has an error", file = file), located(error))
                    }
                    ReloadError::Read(error) => (
                        tr!("{file} cannot be read", file = file),
                        tr!("{error}.", error = error.to_string()),
                    ),
                };
                (
                    summary,
                    tr!(
                        "{detail} The previous settings stay in effect.",
                        detail = detail
                    ),
                )
            }
        };
        let open_in_editor = tr!("Open in Editor");
        let reply = client::notify(
            &self.connection,
            &Request {
                app_name: "Bande",
                replaces_id: shown.unwrap_or(0),
                app_icon: "preferences-system",
                summary: &summary,
                body: &body,
                actions: &[(EDIT_ACTION, &open_in_editor)],
                urgency: Urgency::Normal,
                sound: None,
                desktop_entry: None,
            },
        );
        match reply {
            Ok(id) => *shown = Some(id),
            Err(error) => tracing::warn!("cannot post the error notification: {error}"),
        }
    }
}

fn located(error: &ConfigError) -> String {
    tr!(
        "Line {line}, column {column}: {message}.",
        line = error.line,
        column = error.column,
        message = error.message.as_str()
    )
}
