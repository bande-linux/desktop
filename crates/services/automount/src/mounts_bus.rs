use std::path::PathBuf;

use blocking::unblock;
use zbus::object_server::SignalEmitter;

use crate::discovery::Discovery;
use crate::mounts::{ConnectRequest, Mounts};

pub struct MountsInterface(pub Mounts, pub Discovery);

#[zbus::interface(name = "org.bande.Mounts")]
impl MountsInterface {
    #[allow(clippy::too_many_arguments)]
    async fn connect(
        &self,
        url: String,
        user: String,
        password: String,
        remember: bool,
        reconnect: bool,
        trusted_key: String,
    ) -> (String, Vec<String>) {
        let mounts = self.0.clone();
        let request = ConnectRequest {
            url,
            user,
            password,
            remember,
            reconnect,
            trusted_key,
        };
        unblock(move || mounts.connect(&request).wire()).await
    }

    async fn disconnect(&self, mount_point: String) -> zbus::fdo::Result<()> {
        let mounts = self.0.clone();
        unblock(move || mounts.disconnect(&PathBuf::from(mount_point)))
            .await
            .map_err(zbus::fdo::Error::Failed)
    }

    fn nearby(&self) -> Vec<(String, String)> {
        self.1
            .list()
            .into_iter()
            .map(|entry| (entry.name, entry.url))
            .collect()
    }

    #[zbus(signal)]
    pub(crate) async fn nearby_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    fn servers(&self) -> Vec<(String, String, bool)> {
        self.0
            .saved()
            .into_iter()
            .map(|entry| (entry.url, entry.user, entry.reconnect))
            .collect()
    }

    async fn forget(&self, url: String) {
        let mounts = self.0.clone();
        unblock(move || mounts.forget(&url)).await;
    }

    async fn install(&self, packages: Vec<String>) -> zbus::fdo::Result<()> {
        let mounts = self.0.clone();
        unblock(move || mounts.install(&packages))
            .await
            .map_err(zbus::fdo::Error::Failed)
    }
}
