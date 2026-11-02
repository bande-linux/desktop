use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use bande_bus::accounts::{account, passwd_name, picture};
use bande_xdg::file_uri;
use zbus::blocking::Connection;
use zbus::object_server::ObjectServer;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use crate::file_chooser::option;
use crate::request::{self, CANCELLED, Cancellation, OTHER, SUCCESS};

type Options = HashMap<String, OwnedValue>;
type Results = HashMap<String, OwnedValue>;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Details {
    pub app_id: String,
    pub reason: Option<String>,
    pub login: String,
    pub name: String,
    pub picture: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shared {
    Yes,
    No,
    Unavailable,
}

pub trait AccountUi: Send + Sync {
    fn share(&self, details: &Details, cancellation: &Cancellation) -> Shared;
}

pub struct Account {
    pub ui: Arc<dyn AccountUi>,
    pub system: Option<Connection>,
}

fn details(system: Option<&Connection>, app_id: String, reason: Option<String>) -> Details {
    let login = std::env::var("USER").unwrap_or_default();
    let real_name = system
        .and_then(|system| account(system, &login).ok())
        .and_then(|user| user.real_name().ok())
        .filter(|name| !name.is_empty());
    Details {
        app_id,
        reason,
        name: real_name
            .or_else(|| passwd_name(&login))
            .unwrap_or_else(|| login.clone()),
        picture: system.and_then(|system| picture(system, &login)),
        login,
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Account")]
impl Account {
    async fn get_user_information(
        &self,
        handle: ObjectPath<'_>,
        app_id: String,
        _window: String,
        options: Options,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, Results) {
        let reason = option::<String>(&options, "reason").filter(|reason| !reason.is_empty());
        let (ui, system) = (Arc::clone(&self.ui), self.system.clone());
        let shared = request::run(server, &handle, move |cancellation| {
            let details = details(system.as_ref(), app_id, reason);
            (ui.share(&details, cancellation), details)
        })
        .await;
        match shared {
            Ok(Some((Shared::Yes, details))) => {
                let entries = [
                    ("id", Some(details.login)),
                    ("name", Some(details.name)),
                    ("image", details.picture.as_deref().map(file_uri)),
                ];
                let results = entries
                    .into_iter()
                    .filter_map(|(key, value)| {
                        let value = OwnedValue::try_from(Value::from(value?)).ok()?;
                        Some((key.to_owned(), value))
                    })
                    .collect();
                (SUCCESS, results)
            }
            Ok(Some((Shared::No, _)) | None) => (CANCELLED, Results::new()),
            Ok(Some((Shared::Unavailable, _))) | Err(_) => (OTHER, Results::new()),
        }
    }
}
