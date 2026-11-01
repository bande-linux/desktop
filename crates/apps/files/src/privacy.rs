use std::path::{Path, PathBuf};
use std::thread;

use bande_config::Search;
use bande_inbox::Outbox;
use zbus::blocking::Connection;
use zbus::blocking::fdo::PropertiesProxy;

const CONFIG_BUS_NAME: &str = "org.bande.Config";
const CONFIG_PATH: &str = "/org/bande/Config";

#[zbus::proxy(
    interface = "org.bande.Config",
    default_service = "org.bande.Config",
    default_path = "/org/bande/Config"
)]
trait Config {
    #[zbus(property)]
    fn search_excluded(&self) -> zbus::Result<Vec<String>>;
}

fn excluded(config: &ConfigProxyBlocking<'_>, home: &Path) -> Vec<PathBuf> {
    let search = Search {
        exclude: config
            .search_excluded()
            .unwrap_or_default()
            .into_iter()
            .map(PathBuf::from)
            .collect(),
        ..Search::default()
    };
    search.excluded(Some(home))
}

pub fn follow_search_excluded(home: PathBuf, folders: Outbox<Vec<PathBuf>>) {
    let work = move || {
        let follow = || -> zbus::Result<()> {
            let session = Connection::session()?;
            let config = ConfigProxyBlocking::builder(&session)
                .cache_properties(zbus::proxy::CacheProperties::No)
                .build()?;
            let changes = PropertiesProxy::builder(&session)
                .destination(CONFIG_BUS_NAME)?
                .path(CONFIG_PATH)?
                .build()?
                .receive_properties_changed()?;
            let mut last = excluded(&config, &home);
            if !folders.send(last.clone()) {
                return Ok(());
            }
            for _ in changes {
                let now = excluded(&config, &home);
                if now != last {
                    last = now.clone();
                    if !folders.send(now) {
                        break;
                    }
                }
            }
            Ok(())
        };
        if let Err(error) = follow() {
            tracing::warn!("cannot follow the Search privacy list in org.bande.Config: {error}");
        }
    };
    if let Err(error) = thread::Builder::new()
        .name("search privacy".to_owned())
        .spawn(work)
    {
        tracing::warn!("cannot start following the Search privacy list: {error}");
    }
}
