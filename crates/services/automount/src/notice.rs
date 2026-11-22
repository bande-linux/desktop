use std::collections::HashMap;
use std::thread;

use bande_l10n::tr;
use bande_notify::Urgency;
use bande_notify::client::{self, Request, Signal};
use bande_volumes::udisks::{BLOCK, FILESYSTEM, Objects, UDISKS};
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, Value};

const ALREADY_MOUNTED: &str = "org.freedesktop.UDisks2.Error.AlreadyMounted";
const DISMISSED: &str = "org.freedesktop.UDisks2.Error.NotAuthorizedDismissed";
const DISKS: &str = "org.bande.Disks";
const OPEN_DISKS: &str = "open-disks";
const READ_ONLY: &str = "read-only";
const UNLOCK: &str = "unlock";
const ICON: &str = "drive-removable-media";
const FAILED: &str = "org.freedesktop.UDisks2.Error.Failed";
const NOT_SHUT_DOWN: [&str; 4] = ["dirty", "hibernat", "unclean", "unsafe state"];
const REFUSED: [&str; 2] = ["wrong fs type", "bad superblock"];

fn post(
    session: &Connection,
    summary: &str,
    body: &str,
    action: (&'static str, String),
    on_action: impl FnOnce() + Send + 'static,
) {
    let actions = [(action.0, action.1.as_str())];
    let posted = client::notify(
        session,
        &Request {
            app_name: &tr!("Disks"),
            replaces_id: 0,
            app_icon: ICON,
            summary,
            body,
            actions: &actions,
            urgency: Urgency::Normal,
            sound: None,
            desktop_entry: None,
        },
    );
    let id = match posted {
        Ok(id) => id,
        Err(error) => {
            tracing::warn!("cannot post the notification: {error}");
            return;
        }
    };
    let session = session.clone();
    let key = action.0;
    let spawned = thread::Builder::new()
        .name("disk-notice".to_owned())
        .spawn(move || {
            let mut on_action = Some(on_action);
            let followed = client::listen(&session, |signal| match signal {
                Signal::ActionInvoked(shown, chosen) if shown == id && chosen == key => {
                    if let Some(act) = on_action.take() {
                        act();
                    }
                    false
                }
                Signal::Closed(shown) => shown != id,
                _ => true,
            });
            if let Err(error) = followed {
                tracing::warn!("the disk notification cannot be clicked: {error}");
            }
        });
    if let Err(error) = spawned {
        tracing::warn!("the disk notification cannot be clicked: {error}");
    }
}

fn open_disks() {
    if let Err(error) = bande_xdg::spawn_detached(DISKS, &["bande-disks"], None, None) {
        tracing::warn!("cannot open Disks: {error}");
    }
}

fn open_in_disks() -> (&'static str, String) {
    (OPEN_DISKS, tr!("Open Disks"))
}

pub fn not_unlocked(session: &Connection, label: &str) {
    post(
        session,
        &tr!("“{label}” wasn’t unlocked", label = label),
        &tr!("Open Disks to unlock it."),
        open_in_disks(),
        open_disks,
    );
}

pub fn still_locked(session: &Connection, label: &str, unlock: impl FnOnce() + Send + 'static) {
    post(
        session,
        &tr!("“{label}” is locked", label = label),
        &tr!("Unlock it to open the files on it."),
        (UNLOCK, tr!("Unlock…")),
        unlock,
    );
}

fn mount_with(system: &Connection, path: &OwnedObjectPath, options: &str) -> zbus::Result<()> {
    let mut given: HashMap<&str, Value<'_>> = HashMap::new();
    if !options.is_empty() {
        given.insert("options", Value::from(options));
    }
    match system.call_method(Some(UDISKS), path, Some(FILESYSTEM), "Mount", &(given,)) {
        Ok(_) => Ok(()),
        Err(zbus::Error::MethodError(name, _, _))
            if name.as_str() == ALREADY_MOUNTED || name.as_str() == DISMISSED =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn format_of(system: &Connection, path: &OwnedObjectPath) -> String {
    Objects::load(system)
        .map(|objects| objects.text(path, BLOCK, "IdType"))
        .unwrap_or_else(|error| {
            tracing::debug!("cannot read the format of {path}: {error}");
            String::new()
        })
}

pub fn mount(system: &Connection, session: &Connection, path: &OwnedObjectPath, label: &str) {
    let Err(error) = mount_with(system, path, "") else {
        return;
    };
    tracing::warn!("cannot mount {path}: {error}");
    let summary = tr!("“{label}” couldn’t be opened", label = label);
    let (failed, reason) = match &error {
        zbus::Error::MethodError(name, Some(reason), _) => {
            (name.as_str() == FAILED, reason.to_lowercase())
        }
        _ => (false, String::new()),
    };
    let format = format_of(system, path);
    let refused = |signs: &[&str]| {
        format == "ntfs" && failed && signs.iter().any(|sign| reason.contains(sign))
    };
    let advice = if refused(&NOT_SHUT_DOWN) {
        Some(tr!(
            "It may not have been shut down fully in Windows. Open it read-only, or start Windows, shut it down fully and connect the disk again."
        ))
    } else if refused(&REFUSED) {
        Some(tr!(
            "Open it read-only to get to the files on it, or open Disks to check it."
        ))
    } else {
        None
    };
    if let Some(advice) = advice {
        let (retry_system, retry_session, retry_path, retry_label) = (
            system.clone(),
            session.clone(),
            path.clone(),
            label.to_owned(),
        );
        let retry = move || {
            if let Err(error) = mount_with(&retry_system, &retry_path, "ro") {
                tracing::warn!("cannot mount {retry_path} read-only: {error}");
                post(
                    &retry_session,
                    &tr!("“{label}” couldn’t be opened", label = retry_label),
                    &tr!("Open Disks to check it."),
                    open_in_disks(),
                    open_disks,
                );
            }
        };
        post(
            session,
            &summary,
            &advice,
            (READ_ONLY, tr!("Open Read-Only")),
            retry,
        );
        return;
    }
    let body = if format.is_empty() || reason.contains("unknown filesystem type") {
        tr!("This computer can’t read the format of this disk.")
    } else {
        tr!("Open Disks to check it.")
    };
    post(session, &summary, &body, open_in_disks(), open_disks);
}
