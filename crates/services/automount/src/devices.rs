use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use bande_volumes::user_mounts_root;
use rustix::event::{PollFd, PollFlags, poll};
use udev::{Device, Enumerator, EventType, MonitorBuilder};

use crate::mounts::{Mounts, Started};

const GO_MTPFS: &str = "/usr/bin/go-mtpfs";
const GPHOTOFS: &str = "/usr/bin/gphotofs";
const STILL_IMAGE_INTERFACE: &str = ":060101:";
const RETRY_DELAYS: [u64; 4] = [3, 6, 12, 24];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Phone,
    Camera,
}

#[derive(Debug, Clone)]
struct Attached {
    syspath: PathBuf,
    kind: Kind,
    name: String,
    serial: String,
    bus: String,
    address: String,
}

fn property(device: &Device, name: &str) -> String {
    device
        .property_value(name)
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn attribute(device: &Device, name: &str) -> String {
    device
        .attribute_value(name)
        .map(|value| value.to_string_lossy().trim().to_owned())
        .unwrap_or_default()
}

fn announces_mtp(syspath: &Path) -> bool {
    let Ok(entries) = fs::read_dir(syspath) else {
        return false;
    };
    entries.flatten().any(|entry| {
        fs::read_to_string(entry.path().join("interface"))
            .is_ok_and(|interface| interface.trim().eq_ignore_ascii_case("MTP"))
    })
}

fn classify(device: &Device) -> Option<Kind> {
    if !property(device, "ID_MTP_DEVICE").is_empty() || announces_mtp(device.syspath()) {
        return Some(Kind::Phone);
    }
    let interfaces = property(device, "ID_USB_INTERFACES");
    (interfaces.contains(STILL_IMAGE_INTERFACE) || !property(device, "ID_GPHOTO2").is_empty())
        .then_some(Kind::Camera)
}

fn attached(device: &Device) -> Option<Attached> {
    let kind = classify(device)?;
    let name = [attribute(device, "product"), property(device, "ID_MODEL")]
        .into_iter()
        .find(|name| !name.is_empty())
        .unwrap_or_else(|| "Device".to_owned());
    Some(Attached {
        syspath: device.syspath().to_owned(),
        kind,
        name: name.replace(['/', ','], " ").replace('_', " "),
        serial: attribute(device, "serial"),
        bus: attribute(device, "busnum"),
        address: attribute(device, "devnum"),
    })
}

fn escape_regex(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        if !character.is_alphanumeric() {
            out.push('\\');
        }
        out.push(character);
    }
    out
}

fn command_for(device: &Attached, point: &Path) -> Option<Command> {
    match device.kind {
        Kind::Phone => {
            if !Path::new(GO_MTPFS).exists() {
                return None;
            }
            let mut command = Command::new(GO_MTPFS);
            if !device.serial.is_empty() {
                command
                    .arg("-dev")
                    .arg(format!("{}$", escape_regex(&device.serial)));
            }
            command.arg(point);
            Some(command)
        }
        Kind::Camera => {
            if !Path::new(GPHOTOFS).exists() {
                return None;
            }
            let (Ok(bus), Ok(address)) = (device.bus.parse::<u32>(), device.address.parse::<u32>())
            else {
                return None;
            };
            let mut command = Command::new(GPHOTOFS);
            command
                .arg("-f")
                .arg(format!("--port=usb:{bus:03},{address:03}"))
                .arg(point);
            Some(command)
        }
    }
}

#[derive(Clone)]
pub struct Devices {
    mounts: Mounts,
    mounted: Arc<Mutex<HashMap<PathBuf, PathBuf>>>,
    waiting: Arc<Mutex<Vec<Attached>>>,
    unlocked: Arc<Mutex<bool>>,
}

impl Devices {
    pub fn start(mounts: Mounts) -> Self {
        let devices = Self {
            mounts,
            mounted: Arc::default(),
            waiting: Arc::default(),
            unlocked: Arc::default(),
        };
        let watcher = devices.clone();
        let spawned = thread::Builder::new()
            .name("usb devices".to_owned())
            .spawn(move || {
                if let Err(error) = watcher.follow() {
                    tracing::warn!("cannot follow phones and cameras: {error}");
                }
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot follow phones and cameras: {error}");
        }
        devices
    }

    pub fn session_changed(&self, unlocked: bool) {
        *self.unlocked.lock().unwrap_or_else(PoisonError::into_inner) = unlocked;
        if !unlocked {
            return;
        }
        let waiting =
            std::mem::take(&mut *self.waiting.lock().unwrap_or_else(PoisonError::into_inner));
        for device in waiting {
            self.mount_with_retries(device);
        }
    }

    fn follow(&self) -> std::io::Result<()> {
        let monitor = MonitorBuilder::new()?
            .match_subsystem_devtype("usb", "usb_device")?
            .listen()?;
        let mut enumerator = Enumerator::new()?;
        enumerator.match_subsystem("usb")?;
        enumerator.match_property("DEVTYPE", "usb_device")?;
        for device in enumerator.scan_devices()? {
            self.appeared(&device);
        }
        loop {
            let mut watched = [PollFd::new(&monitor, PollFlags::IN)];
            poll(&mut watched, None)?;
            for event in monitor.iter() {
                match event.event_type() {
                    EventType::Add | EventType::Bind => self.appeared(&event.device()),
                    EventType::Remove => self.removed(event.device().syspath()),
                    _ => {}
                }
            }
        }
    }

    fn appeared(&self, device: &Device) {
        let Some(device) = attached(device) else {
            return;
        };
        if self
            .mounted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(&device.syspath)
        {
            return;
        }
        if *self.unlocked.lock().unwrap_or_else(PoisonError::into_inner) {
            self.mount_with_retries(device);
        } else {
            self.waiting
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(device);
        }
    }

    fn removed(&self, syspath: &Path) {
        self.waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|device| device.syspath != syspath);
        let point = self
            .mounted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(syspath);
        if let Some(point) = point
            && let Err(error) = self.mounts.disconnect(&point)
        {
            tracing::warn!("cannot unmount {}: {error}", point.display());
        }
    }

    fn mount_with_retries(&self, device: Attached) {
        let devices = self.clone();
        let spawned = thread::Builder::new()
            .name("mount device".to_owned())
            .spawn(move || {
                for delay in RETRY_DELAYS {
                    if !device.syspath.exists() {
                        return;
                    }
                    match devices.try_mount(&device) {
                        Ok(()) => return,
                        Err(text) => tracing::debug!("{} did not mount: {text}", device.name),
                    }
                    thread::sleep(Duration::from_secs(delay));
                }
                if device.syspath.exists() {
                    devices.mounts.notify_device_locked(&device.name);
                }
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot mount a device: {error}");
        }
    }

    fn try_mount(&self, device: &Attached) -> Result<(), String> {
        let root = user_mounts_root().ok_or_else(|| "no runtime folder".to_owned())?;
        let point = root.join(crate::mounts::free_name(&device.name));
        let command = command_for(device, &point).ok_or_else(|| "no tool".to_owned())?;
        match self.mounts.start_daemon(command, &point, None) {
            Ok(()) => {
                self.mounted
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(device.syspath.clone(), point);
                Ok(())
            }
            Err(Started::Refused(_)) => Err("refused".to_owned()),
            Err(Started::Failed(text)) => Err(text),
        }
    }
}
