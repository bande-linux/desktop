use std::path::PathBuf;

use blocking::unblock;
use zbus::object_server::SignalEmitter;

use crate::discovery::Discovery;
use crate::mounts::{ConnectRequest, Mounts};
use crate::server::Server;

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
        let mounted = self.0.mounted_urls();
        let mut list: Vec<(String, String)> = self
            .1
            .list()
            .into_iter()
            .map(|entry| (entry.name, entry.url))
            .collect();
        for entry in self.0.saved() {
            if entry.reconnect
                && !mounted.contains(&entry.url)
                && !list.iter().any(|(_, url)| *url == entry.url)
            {
                let name = Server::parse(&entry.url)
                    .map_or_else(|| entry.url.clone(), |server| server.volume_name());
                list.push((name, entry.url));
            }
        }
        list
    }

    fn unresponsive(&self) -> Vec<String> {
        self.0
            .unresponsive()
            .into_iter()
            .map(|point| point.display().to_string())
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
        unblock(move || {
            mounts.forget(&url);
            mounts.announce();
        })
        .await;
    }

    async fn install(&self, packages: Vec<String>) -> zbus::fdo::Result<()> {
        let mounts = self.0.clone();
        unblock(move || mounts.install(&packages))
            .await
            .map_err(zbus::fdo::Error::Failed)
    }
}
