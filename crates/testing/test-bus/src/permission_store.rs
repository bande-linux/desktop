use std::collections::HashMap;

use zbus::blocking::Connection;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

use crate::SessionBus;

pub const BUS_NAME: &str = "org.freedesktop.impl.portal.PermissionStore";
pub const OBJECT_PATH: &str = "/org/freedesktop/impl/portal/PermissionStore";

type AppPermissions = HashMap<String, Vec<String>>;
type Table = HashMap<String, AppPermissions>;

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.portal.Error")]
enum PortalError {
    #[zbus(error)]
    ZBus(zbus::Error),
    NotFound(String),
}

struct Store {
    tables: HashMap<String, Table>,
}

impl Store {
    fn entry(&self, table: &str, id: &str) -> Result<&AppPermissions, PortalError> {
        self.tables
            .get(table)
            .ok_or_else(|| PortalError::NotFound(format!("No such table {table}")))?
            .get(id)
            .ok_or_else(|| PortalError::NotFound(format!("No entry for {id}")))
    }
}

fn empty_data() -> OwnedValue {
    OwnedValue::from(0u8)
}

#[zbus::interface(name = "org.freedesktop.impl.portal.PermissionStore")]
impl Store {
    fn lookup(&self, table: &str, id: &str) -> Result<(AppPermissions, OwnedValue), PortalError> {
        Ok((self.entry(table, id)?.clone(), empty_data()))
    }

    fn list(&self, table: &str) -> Result<Vec<String>, PortalError> {
        let mut ids: Vec<String> = self
            .tables
            .get(table)
            .ok_or_else(|| PortalError::NotFound(format!("No such table {table}")))?
            .keys()
            .cloned()
            .collect();
        ids.sort();
        Ok(ids)
    }

    async fn delete(
        &mut self,
        table: &str,
        id: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), PortalError> {
        self.entry(table, id)?;
        let removed = self
            .tables
            .get_mut(table)
            .and_then(|entries| entries.remove(id))
            .unwrap_or_default();
        Self::changed(&emitter, table, id, true, Value::from(0u8), removed).await?;
        Ok(())
    }

    async fn set_permission(
        &mut self,
        table: &str,
        create: bool,
        id: &str,
        app: &str,
        permissions: Vec<String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), PortalError> {
        if !create {
            self.entry(table, id)?;
        }
        let entry = self
            .tables
            .entry(table.to_owned())
            .or_default()
            .entry(id.to_owned())
            .or_default();
        entry.insert(app.to_owned(), permissions);
        let current = entry.clone();
        Self::changed(&emitter, table, id, false, Value::from(0u8), current).await?;
        Ok(())
    }

    async fn delete_permission(
        &mut self,
        table: &str,
        id: &str,
        app: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), PortalError> {
        self.entry(table, id)?;
        let current = self
            .tables
            .get_mut(table)
            .and_then(|entries| entries.get_mut(id))
            .map(|entry| {
                entry.remove(app);
                entry.clone()
            })
            .unwrap_or_default();
        Self::changed(&emitter, table, id, false, Value::from(0u8), current).await?;
        Ok(())
    }

    #[zbus(signal)]
    async fn changed(
        emitter: &SignalEmitter<'_>,
        table: &str,
        id: &str,
        deleted: bool,
        data: Value<'_>,
        permissions: AppPermissions,
    ) -> zbus::Result<()>;
}

pub struct FakePermissionStore {
    connection: Connection,
}

impl FakePermissionStore {
    pub fn start(bus: &SessionBus, entries: &[(&str, &str, &str, &[&str])]) -> zbus::Result<Self> {
        let mut tables: HashMap<String, Table> = HashMap::new();
        for (table, id, app, permissions) in entries {
            tables
                .entry((*table).to_owned())
                .or_default()
                .entry((*id).to_owned())
                .or_default()
                .insert(
                    (*app).to_owned(),
                    permissions
                        .iter()
                        .map(|value| (*value).to_owned())
                        .collect(),
                );
        }
        let connection = bus.connect()?;
        connection
            .object_server()
            .at(OBJECT_PATH, Store { tables })?;
        connection.request_name(BUS_NAME)?;
        Ok(Self { connection })
    }

    pub fn permissions(&self, table: &str, id: &str) -> Option<AppPermissions> {
        let store = self
            .connection
            .object_server()
            .interface::<_, Store>(OBJECT_PATH)
            .ok()?;
        store.get().entry(table, id).ok().cloned()
    }
}
