#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, NodeId, Role, TreeId};
use unbound_settings::{
    Command, Package, PaneId, SettingsView, SoftwareState, check, feed, follow_software,
};
use unbound_test_bus::SessionBus;
use unbound_toolkit::{Palette, Scheme, TextSystem};
use zbus::object_server::{ObjectServer, SignalEmitter};
use zbus::zvariant::{OwnedObjectPath, Value};

const SIZE: (u32, u32) = (900, 600);
const WAIT: Duration = Duration::from_secs(5);
const UPDATES: [(&str, &str, u64); 2] = [
    (
        "firefox-esr;128.4.0;arm64;debian-security",
        "Web browser",
        70_000_000,
    ),
    (
        "linux-image-asahi;6.12.9;arm64;asahi",
        "Linux kernel",
        120_000_000,
    ),
];

#[derive(Default)]
struct World {
    transactions: u32,
    installed: Vec<String>,
}

struct PackageKit(Arc<Mutex<World>>);

#[zbus::interface(name = "org.freedesktop.PackageKit")]
impl PackageKit {
    async fn create_transaction(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let number = {
            let mut world = self.0.lock().expect("world");
            world.transactions += 1;
            world.transactions
        };
        let path = OwnedObjectPath::try_from(format!("/org/freedesktop/PackageKit/{number}"))
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        server
            .at(&path, Transaction(Arc::clone(&self.0)))
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        Ok(path)
    }
}

struct Transaction(Arc<Mutex<World>>);

#[zbus::interface(name = "org.freedesktop.PackageKit.Transaction")]
impl Transaction {
    fn set_hints(&self, _hints: Vec<String>) {}

    async fn refresh_cache(
        &self,
        _force: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let _ = Self::finished(&emitter, 1, 10).await;
    }

    async fn get_updates(&self, _filter: u64, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        if self.0.lock().expect("world").installed.is_empty() {
            for (id, summary, _) in UPDATES {
                let _ = Self::package(&emitter, 2, id, summary).await;
            }
        }
        let _ = Self::finished(&emitter, 1, 10).await;
    }

    async fn get_details(
        &self,
        ids: Vec<String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        for (id, _, size) in UPDATES
            .iter()
            .filter(|(id, ..)| ids.iter().any(|wanted| wanted == id))
        {
            let details = HashMap::from([
                ("package-id", Value::from(*id)),
                ("download-size", Value::from(*size)),
            ]);
            let _ = Self::details(&emitter, details).await;
        }
        let _ = Self::finished(&emitter, 1, 10).await;
    }

    async fn update_packages(
        &self,
        _flags: u64,
        ids: Vec<String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        if ids.iter().any(|id| id.starts_with("linux-image")) {
            let _ = Self::require_restart(&emitter, 4, &ids[0]).await;
        }
        self.0.lock().expect("world").installed = ids;
        let _ = Self::finished(&emitter, 1, 10).await;
    }

    #[zbus(signal)]
    async fn package(
        emitter: &SignalEmitter<'_>,
        info: u32,
        id: &str,
        summary: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn details(
        emitter: &SignalEmitter<'_>,
        data: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn require_restart(emitter: &SignalEmitter<'_>, kind: u32, id: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn finished(emitter: &SignalEmitter<'_>, exit: u32, runtime: u32) -> zbus::Result<()>;
}

fn find(view: &mut SettingsView, role: Role, label: &str) -> Option<NodeId> {
    view.scene(SIZE.0, SIZE.1);
    view.accessibility()
        .nodes
        .into_iter()
        .find(|(_, node)| {
            node.role() == role && node.label().is_some_and(|text| text.starts_with(label))
        })
        .map(|(id, _)| id)
}

#[test]
fn updates_are_listed_with_sizes_installed_and_a_kernel_asks_for_a_restart() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let world = Arc::new(Mutex::new(World::default()));
    let _daemon = zbus::blocking::connection::Builder::address(bus.address())
        .expect("address")
        .name("org.freedesktop.PackageKit")
        .expect("name")
        .serve_at(
            "/org/freedesktop/PackageKit",
            PackageKit(Arc::clone(&world)),
        )
        .expect("serve")
        .build()
        .expect("fake PackageKit");
    let system = bus.connect().expect("connection");
    assert_eq!(
        check(&system),
        SoftwareState::Available(vec![
            Package {
                id: UPDATES[0].0.to_owned(),
                name: "firefox-esr".to_owned(),
                version: "128.4.0".to_owned(),
                summary: "Web browser".to_owned(),
                size: 70_000_000,
            },
            Package {
                id: UPDATES[1].0.to_owned(),
                name: "linux-image-asahi".to_owned(),
                version: "6.12.9".to_owned(),
                summary: "Linux kernel".to_owned(),
                size: 120_000_000,
            },
        ])
    );

    let (feed, updates) = feed().expect("feed");
    let sender = follow_software(&system, updates);
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    view.select_pane(PaneId::SoftwareUpdate);
    let pump = |view: &mut SettingsView, done: &dyn Fn(&mut SettingsView) -> bool| {
        let deadline = Instant::now() + WAIT;
        loop {
            for update in feed.drain() {
                view.apply(update);
            }
            for command in view.take_commands() {
                if let Command::Software(command) = command {
                    sender.send(command).expect("software worker");
                }
            }
            if done(view) {
                return;
            }
            assert!(Instant::now() < deadline, "the view never got there");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    pump(&mut view, &|view| {
        find(
            view,
            Role::Row,
            "2 updates are available, 190 MB to download",
        )
        .is_some()
    });
    assert!(find(&mut view, Role::Row, "linux-image-asahi, 6.12.9").is_some());
    let install = find(&mut view, Role::Button, "Install Now").expect("Install Now");
    view.perform(&ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: install,
        data: None,
    });
    pump(&mut view, &|view| {
        find(view, Role::Button, "Restart Now").is_some()
    });
    assert_eq!(world.lock().expect("world").installed.len(), 2);
    assert_eq!(
        view.take_commands(),
        Vec::<Command>::new(),
        "nothing restarts without asking"
    );
}
