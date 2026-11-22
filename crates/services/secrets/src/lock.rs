use std::collections::HashMap;
use std::sync::Arc;

use futures_lite::StreamExt;
use rustix::process::getuid;
use zbus::message::Type;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::{Connection, MatchRule, MessageStream};

use crate::service::Daemon;

const LOGIN: &str = "org.freedesktop.login1";
const LOGIN_PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const SESSION: &str = "org.freedesktop.login1.Session";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const BANDE_DESKTOP: &str = "bande";

async fn ours(system: &Connection, session: &str) -> zbus::Result<bool> {
    let reply = system
        .call_method(
            Some(LOGIN),
            session,
            Some(PROPERTIES),
            "Get",
            &(SESSION, "User"),
        )
        .await?;
    let user: OwnedValue = reply.body().deserialize()?;
    let (uid, _): (u32, OwnedObjectPath) = user.try_into()?;
    Ok(uid == getuid().as_raw())
}

async fn desktop(system: &Connection, session: &str) -> zbus::Result<String> {
    let reply = system
        .call_method(
            Some(LOGIN),
            session,
            Some(PROPERTIES),
            "Get",
            &(SESSION, "Desktop"),
        )
        .await?;
    let desktop: OwnedValue = reply.body().deserialize()?;
    Ok(desktop.try_into()?)
}

/// Whether logind has a Bande session of this user, whose desktop the login window names `bande`.
pub async fn has_bande_session(system: &Connection) -> zbus::Result<bool> {
    let reply = system
        .call_method(Some(LOGIN), LOGIN_PATH, Some(MANAGER), "ListSessions", &())
        .await?;
    let sessions: Vec<(String, u32, String, String, OwnedObjectPath)> =
        reply.body().deserialize()?;
    let uid = getuid().as_raw();
    for (_, owner, _, _, path) in sessions {
        if owner != uid {
            continue;
        }
        match desktop(system, path.as_str()).await {
            Ok(desktop) if desktop == BANDE_DESKTOP => return Ok(true),
            Ok(_) => {}
            Err(error) => tracing::debug!("cannot read the desktop of session {path}: {error}"),
        }
    }
    Ok(false)
}

pub async fn revoke_grants_on_lock(daemon: Arc<Daemon>, system: Connection) -> zbus::Result<()> {
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(LOGIN)?
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .add_arg(SESSION)?
        .build();
    let mut changes = MessageStream::for_match_rule(rule, &system, None).await?;
    while let Some(message) = changes.next().await {
        let Ok(message) = message else {
            continue;
        };
        let Ok((_, changed, _)) = message
            .body()
            .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
        else {
            continue;
        };
        let locked = changed
            .get("LockedHint")
            .and_then(|value| bool::try_from(value).ok())
            .unwrap_or(false);
        let Some(session) = message.header().path().map(|path| path.to_string()) else {
            continue;
        };
        if locked && ours(&system, &session).await.unwrap_or(false) {
            daemon.revoke_grants();
        }
    }
    Ok(())
}
