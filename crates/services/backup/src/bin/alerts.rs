use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use bande_backup::client::{ALERT_FILE, problem};
use bande_l10n::tr;
use bande_notify::Urgency;
use bande_notify::client::{self, Request};
use zbus::blocking::{Connection, MessageIterator};
use zbus::{MatchRule, message};

const OPEN: &str = "open-settings";
const SETTINGS: &str = "org.bande.Settings";
const NOTIFICATIONS: &str = "org.freedesktop.Notifications";

fn texts(kind: &str, disk: &str) -> Option<(String, String)> {
    match kind {
        problem::OVERDUE => Some((
            tr!("No backup for two weeks"),
            tr!(
                "Connect {disk} to back up this computer again.",
                disk = disk
            ),
        )),
        problem::DISK_FULL => Some((
            tr!("The backup disk is too small"),
            tr!(
                "{disk} has no more room, even with the oldest backups removed. Use a larger disk.",
                disk = disk
            ),
        )),
        problem::DAMAGED => Some((
            tr!("Damage found on the backup disk"),
            tr!(
                "Some older versions on {disk} were lost. Backups go on, but the disk may be failing.",
                disk = disk
            ),
        )),
        problem::CANNOT_OPEN => Some((
            tr!("Can't open the backups"),
            tr!(
                "The backups on {disk} are missing or use another password.",
                disk = disk
            ),
        )),
        problem::FAILING => Some((
            tr!("Backups keep failing"),
            tr!(
                "The last few backups to {disk} did not finish.",
                disk = disk
            ),
        )),
        _ => None,
    }
}

fn told_file() -> Option<PathBuf> {
    bande_xdg::state_home().map(|state| state.join("bande/backup-told"))
}

fn signals(session: &Connection) -> zbus::Result<MessageIterator> {
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .sender(NOTIFICATIONS)?
        .interface(NOTIFICATIONS)?
        .build();
    MessageIterator::for_match_rule(rule, session, None)
}

fn wait_for_answer(signals: MessageIterator, id: u32) {
    for signal in signals.flatten() {
        let header = signal.header();
        let body = signal.body();
        match header.member().map(|member| member.as_str()) {
            Some("ActionInvoked") => {
                if let Ok((shown, key)) = body.deserialize::<(u32, String)>()
                    && shown == id
                    && key == OPEN
                {
                    let command = ["bande-settings".to_owned(), "backup".to_owned()];
                    if let Err(error) = bande_xdg::spawn_detached(SETTINGS, &command, None, None) {
                        tracing::warn!("cannot open Settings: {error}");
                    }
                    return;
                }
            }
            Some("NotificationClosed")
                if body
                    .deserialize::<(u32, u32)>()
                    .is_ok_and(|(closed, _)| closed == id) =>
            {
                return;
            }
            _ => {}
        }
    }
}

fn run() -> zbus::Result<()> {
    let Ok(alert) = fs::read_to_string(ALERT_FILE) else {
        return Ok(());
    };
    let mut lines = alert.lines();
    let kind = lines.next().unwrap_or_default();
    let since = lines.next().unwrap_or_default();
    let disk = lines.next().unwrap_or_default().to_owned();
    let Some((title, body)) = texts(kind, &disk) else {
        return Ok(());
    };
    let told = format!("{kind} {since}");
    let told_path = told_file();
    if told_path
        .as_ref()
        .and_then(|path| fs::read_to_string(path).ok())
        .is_some_and(|known| known == told)
    {
        return Ok(());
    }
    let session = Connection::session()?;
    let signals = signals(&session)?;
    let open = tr!("Open Backup Settings");
    let id = client::notify(
        &session,
        &Request {
            app_name: &tr!("Backup"),
            replaces_id: 0,
            app_icon: "drive-harddisk",
            summary: &title,
            body: &body,
            actions: &[(OPEN, &open)],
            urgency: Urgency::Normal,
            sound: None,
            desktop_entry: None,
        },
    )?;
    if let Some(path) = told_path {
        let remembered = path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(&path, told));
        if let Err(error) = remembered {
            tracing::warn!("cannot remember the backup alert: {error}");
        }
    }
    wait_for_answer(signals, id);
    Ok(())
}

fn main() -> ExitCode {
    bande_log::init("bande-backup-alerts");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
