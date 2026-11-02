use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use zbus::blocking::{Connection, Proxy, proxy};
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;

use crate::SessionBus;

pub const ROOT_PATH: &str = "/org/a11y/atspi/accessible/root";
pub const ROLE_BUTTON: u32 = 43;

pub type ObjectRef = (String, OwnedObjectPath);

struct Status;

#[zbus::interface(name = "org.a11y.Status")]
impl Status {
    #[zbus(property)]
    fn is_enabled(&self) -> bool {
        true
    }
}

struct Socket {
    embedded: Sender<String>,
    unique_name: String,
}

#[zbus::interface(name = "org.a11y.atspi.Socket")]
impl Socket {
    fn embed(&self, plug: ObjectRef) -> zbus::fdo::Result<ObjectRef> {
        let _ = self.embedded.send(plug.0);
        let root = OwnedObjectPath::try_from(ROOT_PATH)
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        Ok((self.unique_name.clone(), root))
    }
}

pub struct FakeRegistry {
    _status: Connection,
    _registry: Connection,
    plugs: Receiver<String>,
}

impl FakeRegistry {
    pub fn start(bus: &SessionBus) -> zbus::Result<Self> {
        let status = zbus::blocking::connection::Builder::address(bus.address())?
            .name("org.a11y.Bus")?
            .serve_at("/org/a11y/bus", Status)?
            .build()?;
        let (embedded, plugs) = mpsc::channel();
        let registry = bus.connect()?;
        let unique_name = registry
            .unique_name()
            .map(ToString::to_string)
            .unwrap_or_default();
        registry.object_server().at(
            ROOT_PATH,
            Socket {
                embedded,
                unique_name,
            },
        )?;
        registry.request_name("org.a11y.atspi.Registry")?;
        Ok(Self {
            _status: status,
            _registry: registry,
            plugs,
        })
    }

    pub fn embedded_root(&self, timeout: Duration) -> Option<ObjectRef> {
        let app = self.plugs.recv_timeout(timeout).ok()?;
        Some((app, OwnedObjectPath::try_from(ROOT_PATH).ok()?))
    }
}

pub fn accessible<'a>(
    atspi: &'a Connection,
    object: &'a ObjectRef,
    interface: &'a str,
) -> zbus::Result<Proxy<'a>> {
    proxy::Builder::new(atspi)
        .destination(object.0.as_str())?
        .path(object.1.as_str())?
        .interface(interface)?
        .cache_properties(CacheProperties::No)
        .build()
}

pub fn find_named(atspi: &Connection, object: &ObjectRef, name: &str) -> Option<ObjectRef> {
    let node = accessible(atspi, object, "org.a11y.atspi.Accessible").ok()?;
    if node.get_property::<String>("Name").ok()? == name {
        return Some(object.clone());
    }
    let children: Vec<ObjectRef> = node.call("GetChildren", &()).ok()?;
    children
        .iter()
        .find_map(|child| find_named(atspi, child, name))
}
