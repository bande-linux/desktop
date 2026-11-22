use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedFd, OwnedValue, Value};

use crate::SessionBus;

pub const SYSTEM_POWER_TOO_LOW: u64 = 1 << 0;
const UPDATABLE: u64 = 1 << 1;
const NEEDS_REBOOT: u64 = 1 << 8;
const STATE_SUCCESS: u32 = 2;
const STATE_NEEDS_REBOOT: u32 = 4;
const REMOTE_ENABLED: u64 = 1 << 0;
const REMOTE_DOWNLOAD: u32 = 1;
pub const METADATA: &[u8] = b"metadata";
pub const SIGNATURE: &[u8] = b"signature";
pub const NOTES: &str = "<p>Fixes waking from sleep.</p><ul><li>Quieter fan</li></ul>";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: &'static str,
    pub name: &'static str,
    pub from: &'static str,
    pub to: &'static str,
    pub at_restart: bool,
    pub problems: u64,
}

impl Device {
    pub fn cab(&self) -> Vec<u8> {
        format!("cabinet for {}", self.id).into_bytes()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    UpdateMetadata(String, Vec<u8>, Vec<u8>),
    Install(String, Vec<u8>),
}

#[derive(Default)]
struct World {
    devices: Vec<Device>,
    states: HashMap<String, u32>,
    calls: Vec<Call>,
    port: u16,
}

type Shared = Arc<Mutex<World>>;

fn lock(world: &Shared) -> MutexGuard<'_, World> {
    world.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.fwupd")]
enum FwupdError {
    #[zbus(error)]
    ZBus(zbus::Error),
    NothingToDo(String),
    NotFound(String),
}

type Record = HashMap<String, OwnedValue>;

fn record(entries: Vec<(&str, Value<'_>)>) -> Result<Record, FwupdError> {
    entries
        .into_iter()
        .map(|(key, value)| Ok((key.to_owned(), value.try_into_owned()?)))
        .collect::<Result<_, zbus::zvariant::Error>>()
        .map_err(|error| FwupdError::ZBus(error.into()))
}

fn read(fd: OwnedFd) -> Vec<u8> {
    let mut bytes = Vec::new();
    let read = File::from(std::os::fd::OwnedFd::from(fd)).read_to_end(&mut bytes);
    read.map(|_| bytes).unwrap_or_default()
}

struct Fwupd(Shared);

#[zbus::interface(name = "org.freedesktop.fwupd")]
impl Fwupd {
    fn get_devices(&self) -> Result<Vec<Record>, FwupdError> {
        let world = lock(&self.0);
        world
            .devices
            .iter()
            .map(|device| {
                let flags = UPDATABLE | if device.at_restart { NEEDS_REBOOT } else { 0 };
                let state = world.states.get(device.id).copied().unwrap_or(0);
                let version = match state {
                    STATE_SUCCESS => device.to,
                    _ => device.from,
                };
                record(vec![
                    ("DeviceId", Value::from(device.id)),
                    ("Name", Value::from(device.name)),
                    ("Version", Value::from(version)),
                    ("Flags", Value::from(flags)),
                    ("Problems", Value::from(device.problems)),
                    ("UpdateState", Value::from(state)),
                ])
            })
            .collect()
    }

    fn get_upgrades(&self, device_id: String) -> Result<Vec<Record>, FwupdError> {
        let world = lock(&self.0);
        let device = world
            .devices
            .iter()
            .find(|device| device.id == device_id)
            .ok_or_else(|| FwupdError::NotFound(device_id.clone()))?;
        if world.states.contains_key(device.id) {
            return Err(FwupdError::NothingToDo("no upgrades".to_owned()));
        }
        let location = format!("http://127.0.0.1:{}/{}.cab", world.port, device.id);
        Ok(vec![record(vec![
            ("Version", Value::from(device.to)),
            ("Description", Value::from(NOTES)),
            ("Locations", Value::from(vec![location])),
            ("RemoteId", Value::from("lvfs")),
        ])?])
    }

    fn get_remotes(&self) -> Result<Vec<Record>, FwupdError> {
        let port = lock(&self.0).port;
        Ok(vec![record(vec![
            ("RemoteId", Value::from("lvfs")),
            ("Type", Value::from(REMOTE_DOWNLOAD)),
            ("Flags", Value::from(REMOTE_ENABLED)),
            (
                "Uri",
                Value::from(format!("http://127.0.0.1:{port}/firmware.xml.zst")),
            ),
        ])?])
    }

    fn update_metadata(&self, remote_id: String, data: OwnedFd, signature: OwnedFd) {
        let call = Call::UpdateMetadata(remote_id, read(data), read(signature));
        lock(&self.0).calls.push(call);
    }

    fn install(
        &self,
        id: String,
        handle: OwnedFd,
        _options: HashMap<String, OwnedValue>,
    ) -> Result<(), FwupdError> {
        let mut world = lock(&self.0);
        let device = world
            .devices
            .iter()
            .find(|device| device.id == id)
            .cloned()
            .ok_or_else(|| FwupdError::NotFound(id.clone()))?;
        let state = match device.at_restart {
            true => STATE_NEEDS_REBOOT,
            false => STATE_SUCCESS,
        };
        world.states.insert(id.clone(), state);
        world.calls.push(Call::Install(id, read(handle)));
        Ok(())
    }
}

fn serve(listener: TcpListener, world: Shared) {
    for stream in listener.incoming().flatten() {
        let mut reader = BufReader::new(&stream);
        let mut request = String::new();
        if reader.read_line(&mut request).is_err() {
            continue;
        }
        let mut line = String::new();
        while reader.read_line(&mut line).is_ok_and(|read| read > 2) {
            line.clear();
        }
        let path = request.split(' ').nth(1).unwrap_or_default();
        let body = match path {
            "/firmware.xml.zst" => Some(METADATA.to_vec()),
            "/firmware.xml.zst.jcat" => Some(SIGNATURE.to_vec()),
            cab => lock(&world)
                .devices
                .iter()
                .find(|device| cab == format!("/{}.cab", device.id))
                .map(Device::cab),
        };
        let (status, body) = match body {
            Some(body) => ("200 OK", body),
            None => ("404 Not Found", Vec::new()),
        };
        let mut stream = &stream;
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(&body);
    }
}

pub struct FakeFwupd {
    _daemon: Connection,
    world: Shared,
}

impl FakeFwupd {
    pub fn start(bus: &SessionBus, devices: &[Device]) -> zbus::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let world = Arc::new(Mutex::new(World {
            devices: devices.to_vec(),
            port: listener.local_addr()?.port(),
            ..World::default()
        }));
        let served = Arc::clone(&world);
        std::thread::spawn(move || serve(listener, served));
        let daemon = zbus::blocking::connection::Builder::address(bus.address())?
            .name("org.freedesktop.fwupd")?
            .serve_at("/", Fwupd(Arc::clone(&world)))?
            .build()?;
        Ok(Self {
            _daemon: daemon,
            world,
        })
    }

    pub fn take_calls(&self) -> Vec<Call> {
        std::mem::take(&mut lock(&self.world).calls)
    }
}
