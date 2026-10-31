use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};

use unbound_settings_panes::{PaneId, Permission};
use unbound_toolkit::ui::Toggle;
use unbound_xdg::{DesktopEntry, Locale, app_name, applications, current_desktops, data_dirs};
use zbus::blocking::Connection;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedValue;

use crate::L10N;
use crate::failure::unexpected;
use crate::follow::{Command, Update, Updates, spawn};
use crate::forget::Forget;
use crate::pages::{Widgets, boxed, muted_row};
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

type AppPermissions = HashMap<String, Vec<String>>;

const NOT_FOUND: &str = "org.freedesktop.portal.Error.NotFound";
const SCREENCAST: &str = "screencast";
const ALLOWED: &str = "yes";
const DENIED: &str = "no";
const NO_LOCATION: &str = "NONE";
const EXACT_LOCATION: &str = "EXACT";

#[zbus::proxy(
    interface = "org.freedesktop.impl.portal.PermissionStore",
    default_service = "org.freedesktop.impl.portal.PermissionStore",
    default_path = "/org/freedesktop/impl/portal/PermissionStore"
)]
trait PermissionStore {
    fn lookup(&self, table: &str, id: &str) -> zbus::Result<(AppPermissions, OwnedValue)>;
    fn list(&self, table: &str) -> zbus::Result<Vec<String>>;
    fn delete(&self, table: &str, id: &str) -> zbus::Result<()>;
    fn set_permission(
        &self,
        table: &str,
        create: bool,
        id: &str,
        app: &str,
        permissions: &[&str],
    ) -> zbus::Result<()>;
    fn delete_permission(&self, table: &str, id: &str, app: &str) -> zbus::Result<()>;
    #[zbus(signal)]
    fn changed(
        &self,
        table: &str,
        id: &str,
        deleted: bool,
        data: zbus::zvariant::Value<'_>,
        permissions: AppPermissions,
    ) -> zbus::Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub app: String,
    pub name: String,
    pub allowed: bool,
    pub tokens: Vec<String>,
}

pub type Grants = Vec<(Permission, Vec<Grant>)>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionCommand {
    Refresh,
    Allow(Permission, String, bool),
    Forget(Permission, String),
}

fn entry(permission: Permission) -> (&'static str, &'static str) {
    match permission {
        Permission::Camera => ("devices", "camera"),
        Permission::Microphone => ("devices", "microphone"),
        Permission::Screenshots => ("screenshot", "screenshot"),
        Permission::ScreenSharing => (SCREENCAST, SCREENCAST),
        Permission::Location => ("location", "location"),
    }
}

fn absent(error: &zbus::Error) -> bool {
    match error {
        zbus::Error::MethodError(name, _, _) => {
            name.as_str() == NOT_FOUND
                || name.as_str() == "org.freedesktop.DBus.Error.ServiceUnknown"
        }
        zbus::Error::FDO(error) => matches!(**error, zbus::fdo::Error::ServiceUnknown(_)),
        _ => false,
    }
}

fn or_empty<T: Default>(result: zbus::Result<T>) -> zbus::Result<T> {
    match result {
        Err(error) if absent(&error) => Ok(T::default()),
        result => result,
    }
}

fn allowed(permission: Permission, values: &[String]) -> bool {
    let first = values.first().map(String::as_str);
    match permission {
        Permission::Location => first.is_some_and(|level| level != NO_LOCATION),
        Permission::ScreenSharing => true,
        _ => first == Some(ALLOWED),
    }
}

fn app_label(entries: &[DesktopEntry], app: &str) -> String {
    if app.is_empty() {
        L10N.tr("other-apps")
    } else {
        app_name(entries, app)
    }
}

fn read_grants(
    store: &PermissionStoreProxyBlocking<'_>,
    permission: Permission,
    entries: &[DesktopEntry],
) -> zbus::Result<Vec<Grant>> {
    let (table, id) = entry(permission);
    let mut grants: Vec<Grant> = Vec::new();
    let ids = match permission {
        Permission::ScreenSharing => or_empty(store.list(table))?,
        _ => vec![id.to_owned()],
    };
    for id in ids {
        let (apps, _) = or_empty(store.lookup(table, &id).map(|(apps, _)| (apps, ())))?;
        for (app, values) in apps {
            match grants.iter_mut().find(|grant| grant.app == app) {
                Some(grant) => grant.tokens.push(id.clone()),
                None => grants.push(Grant {
                    name: app_label(entries, &app),
                    allowed: allowed(permission, &values),
                    tokens: vec![id.clone()],
                    app,
                }),
            }
        }
    }
    grants.sort_by_key(|grant| (grant.app.is_empty(), grant.name.to_lowercase()));
    Ok(grants)
}

fn read_all(store: &PermissionStoreProxyBlocking<'_>) -> Grants {
    let entries = applications(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    );
    Permission::ALL
        .into_iter()
        .map(|permission| {
            let grants = read_grants(store, permission, &entries).unwrap_or_else(|error| {
                tracing::warn!("cannot read the {permission:?} permissions: {error}");
                Vec::new()
            });
            (permission, grants)
        })
        .collect()
}

fn allow(
    store: &PermissionStoreProxyBlocking<'_>,
    permission: Permission,
    app: &str,
    on: bool,
) -> zbus::Result<()> {
    let (table, id) = entry(permission);
    let values: Vec<String> = match permission {
        Permission::Location => {
            let (apps, _) = store.lookup(table, id)?;
            let mut values = apps.get(app).cloned().unwrap_or_default();
            let level = if on { EXACT_LOCATION } else { NO_LOCATION };
            match values.first_mut() {
                Some(first) => level.clone_into(first),
                None => values.push(level.to_owned()),
            }
            values
        }
        _ => vec![if on { ALLOWED } else { DENIED }.to_owned()],
    };
    let values: Vec<&str> = values.iter().map(String::as_str).collect();
    store.set_permission(table, false, id, app, &values)
}

fn forget(store: &PermissionStoreProxyBlocking<'_>, app: &str) -> zbus::Result<()> {
    for token in store.list(SCREENCAST)? {
        let (apps, _) = store.lookup(SCREENCAST, &token)?;
        if !apps.contains_key(app) {
            continue;
        }
        if apps.len() == 1 {
            store.delete(SCREENCAST, &token)?;
        } else {
            store.delete_permission(SCREENCAST, &token, app)?;
        }
    }
    Ok(())
}

fn follow_changes(session: &Connection, refresh: Sender<PermissionCommand>) {
    let session = session.clone();
    spawn("permission-changes", move || {
        let follow = || -> zbus::Result<()> {
            let store = PermissionStoreProxyBlocking::builder(&session)
                .cache_properties(CacheProperties::No)
                .build()?;
            for _ in store.receive_changed()? {
                if refresh.send(PermissionCommand::Refresh).is_err() {
                    break;
                }
            }
            Ok(())
        };
        if let Err(error) = follow() {
            tracing::warn!("cannot follow the permission store: {error}");
        }
    });
}

pub fn follow_permissions(session: &Connection, updates: Updates) -> Sender<PermissionCommand> {
    let (sender, receiver) = mpsc::channel();
    follow_changes(session, sender.clone());
    let session = session.clone();
    spawn("permissions", move || {
        let store = match PermissionStoreProxyBlocking::builder(&session)
            .cache_properties(CacheProperties::No)
            .build()
        {
            Ok(store) => store,
            Err(error) => {
                tracing::warn!("cannot reach the permission store: {error}");
                return;
            }
        };
        if !updates.send(Update::Permissions(read_all(&store))) {
            return;
        }
        while let Ok(first) = receiver.recv() {
            for command in std::iter::once(first).chain(receiver.try_iter()) {
                let result = match command {
                    PermissionCommand::Refresh => Ok(()),
                    PermissionCommand::Allow(permission, app, on) => {
                        allow(&store, permission, &app, on)
                    }
                    PermissionCommand::Forget(_, app) => forget(&store, &app),
                };
                if let Err(error) = result {
                    updates.send(unexpected(error));
                }
            }
            if !updates.send(Update::Permissions(read_all(&store))) {
                break;
            }
        }
    });
    sender
}

impl SettingsView {
    pub(crate) fn grants(&self, permission: Permission) -> &[Grant] {
        self.permissions
            .iter()
            .find(|(candidate, _)| *candidate == permission)
            .map_or(&[], |(_, grants)| grants.as_slice())
    }

    pub(crate) fn has_grants(&self, pane: PaneId) -> bool {
        Permission::of(pane).is_some_and(|permission| !self.grants(permission).is_empty())
    }

    pub(crate) fn grant_summary(&self, pane: PaneId) -> Option<String> {
        let count = self.grants(Permission::of(pane)?).len();
        Some(L10N.tr_args("app-count", [("count", count.into())]))
    }

    pub(crate) fn permission_rows(&mut self, permission: Permission) -> Widgets {
        let grants = self.grants(permission).to_vec();
        if grants.is_empty() {
            return vec![muted_row(
                &L10N.tr(&format!("{}-none", permission.pane().keyword())),
            )];
        }
        let mut rows = Vec::new();
        for grant in grants {
            let anchor = format!("grant:{}", grant.app);
            let control = match permission {
                Permission::ScreenSharing => {
                    let target = Forget::Share(grant.app.clone());
                    if self.forgetting.as_ref() == Some(&target) {
                        let row = self.forget_row(&grant.name, &L10N.tr("forget-share-warning"));
                        rows.push(row);
                        continue;
                    }
                    self.forget_button(target)
                }
                _ => {
                    let app = grant.app.clone();
                    let set = intent(&self.intents, move |on| {
                        Intent::Grant(permission, app.clone(), on)
                    });
                    boxed(Toggle::new(&grant.name, grant.allowed, set))
                }
            };
            let row = Row::new(Text::new(&grant.name, Tone::Primary), Some(control));
            rows.push(self.item_row(anchor, row));
        }
        rows
    }

    pub(crate) fn grant(&mut self, permission: Permission, app: String, on: bool) {
        if let Some((_, grants)) = self
            .permissions
            .iter_mut()
            .find(|(candidate, _)| *candidate == permission)
            && let Some(grant) = grants.iter_mut().find(|grant| grant.app == app)
        {
            grant.allowed = on;
        }
        self.focus_next = Some(Anchor::Item(format!("grant:{app}")));
        self.commands
            .push(Command::Permission(PermissionCommand::Allow(
                permission, app, on,
            )));
    }
}
