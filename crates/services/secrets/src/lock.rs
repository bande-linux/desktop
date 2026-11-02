use std::collections::HashMap;
use std::sync::Arc;

use futures_lite::StreamExt;
use rustix::process::getuid;
use zbus::message::Type;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::{Connection, MatchRule, MessageStream};

use crate::service::Daemon;

const LOGIN: &str = "org.freedesktop.login1";
const SESSION: &str = "org.freedesktop.login1.Session";

async fn ours(system: &Connection, session: &str) -> zbus::Result<bool> {
    let reply = system
        .call_method(
            Some(LOGIN),
            session,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(SESSION, "User"),
        )
        .await?;
    let user: OwnedValue = reply.body().deserialize()?;
    let (uid, _): (u32, OwnedObjectPath) = user.try_into()?;
    Ok(uid == getuid().as_raw())
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
