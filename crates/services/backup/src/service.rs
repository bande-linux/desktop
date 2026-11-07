use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread;

use bande_volumes::udisks::{BLOCK, DRIVE, FILESYSTEM, Objects};
use zbus::blocking::Connection;
use zbus::blocking::connection::Builder;
use zbus::blocking::fdo::DBusProxy;
use zbus::message::Header;
use zbus::zvariant::Value;
use zbus::{fdo, interface};

use crate::client::{BUS_NAME, DiskChoice, OBJECT_PATH, Status};
use crate::disk;
use crate::restic::StopReason;
use crate::worker::{Command, Shared, Worker, stop_when_unplugged};

pub const HOUR: u64 = 3600;
pub const DAY: u64 = 24 * HOUR;
const CONFIGURE: &str = "org.bande.backup.configure";
const CONTROL: &str = "org.bande.backup.control";
const ALLOW_INTERACTION: u32 = 1;
const USABLE_FILESYSTEMS: [&str; 8] = [
    "ext4", "ext3", "btrfs", "xfs", "f2fs", "exfat", "ntfs", "vfat",
];

#[zbus::proxy(
    interface = "org.freedesktop.PolicyKit1.Authority",
    default_service = "org.freedesktop.PolicyKit1",
    default_path = "/org/freedesktop/PolicyKit1/Authority"
)]
trait Authority {
    fn check_authorization(
        &self,
        subject: &(&str, HashMap<&str, Value<'_>>),
        action_id: &str,
        details: &HashMap<&str, &str>,
        flags: u32,
        cancellation_id: &str,
    ) -> zbus::Result<(bool, bool, HashMap<String, String>)>;
}

async fn authorize(
    connection: &zbus::Connection,
    header: &Header<'_>,
    action: &str,
) -> fdo::Result<()> {
    let sender = header
        .sender()
        .ok_or_else(|| fdo::Error::AccessDenied("no sender".to_owned()))?
        .to_string();
    let authority = AuthorityProxy::new(connection)
        .await
        .map_err(|error| fdo::Error::Failed(error.to_string()))?;
    let subject = (
        "system-bus-name",
        HashMap::from([("name", Value::from(sender.as_str()))]),
    );
    let (authorized, _, _) = authority
        .check_authorization(&subject, action, &HashMap::new(), ALLOW_INTERACTION, "")
        .await
        .map_err(|error| fdo::Error::Failed(error.to_string()))?;
    if authorized {
        Ok(())
    } else {
        Err(fdo::Error::AccessDenied(action.to_owned()))
    }
}

fn sender_of(header: &Header<'_>) -> fdo::Result<String> {
    header
        .sender()
        .map(ToString::to_string)
        .ok_or_else(|| fdo::Error::AccessDenied("no sender".to_owned()))
}

struct Service {
    shared: Arc<Shared>,
    commands: Sender<Command>,
}

impl Service {
    fn send(&self, command: Command) -> fdo::Result<()> {
        self.commands
            .send(command)
            .map_err(|_| fdo::Error::Failed("the backup service is stopping".to_owned()))
    }
}

#[interface(name = "org.bande.Backup")]
impl Service {
    #[zbus(property)]
    fn status(&self) -> Status {
        self.shared.status()
    }

    async fn disks(&self) -> fdo::Result<Vec<DiskChoice>> {
        usable_disks_off_the_bus().await
    }

    async fn has_backups(&self, uuid: String) -> fdo::Result<bool> {
        let choice = disk_with(&uuid).await?;
        blocking::unblock(move || disk::peek(&choice.uuid, &choice.filesystem))
            .await
            .map_err(|error| fdo::Error::Failed(error.to_string()))
    }

    async fn choose(
        &self,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
        uuid: String,
        password: String,
    ) -> fdo::Result<()> {
        authorize(connection, &header, CONFIGURE).await?;
        let choice = disk_with(&uuid).await?;
        self.send(Command::Choose {
            uuid: choice.uuid,
            name: choice.name,
            filesystem: choice.filesystem,
            password,
        })
    }

    async fn stop_using(
        &self,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        authorize(connection, &header, CONFIGURE).await?;
        self.send(Command::StopUsing)
    }

    async fn back_up_now(
        &self,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        authorize(connection, &header, CONTROL).await?;
        self.send(Command::BackUpNow)
    }

    async fn pause(
        &self,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        authorize(connection, &header, CONTROL).await?;
        self.send(Command::Pause)
    }

    async fn resume(
        &self,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        authorize(connection, &header, CONTROL).await?;
        self.send(Command::Resume)
    }

    async fn set_on_battery(
        &self,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
        allowed: bool,
    ) -> fdo::Result<()> {
        authorize(connection, &header, CONTROL).await?;
        self.shared.set_on_battery(allowed);
        Ok(())
    }

    fn open_browser(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        self.send(Command::OpenBrowser(sender_of(&header)?))
    }

    fn close_browser(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        self.send(Command::CloseBrowser(sender_of(&header)?))
    }
}

async fn usable_disks_off_the_bus() -> fdo::Result<Vec<DiskChoice>> {
    blocking::unblock(usable_disks)
        .await
        .map_err(|error| fdo::Error::Failed(error.to_string()))
}

async fn disk_with(uuid: &str) -> fdo::Result<DiskChoice> {
    usable_disks_off_the_bus()
        .await?
        .into_iter()
        .find(|disk| disk.uuid == uuid)
        .ok_or_else(|| fdo::Error::Failed("disk-gone".to_owned()))
}

fn usable_disks() -> zbus::Result<Vec<DiskChoice>> {
    let system = Connection::system()?;
    let objects = Objects::load(&system)?;
    let startup_drive = objects
        .paths()
        .map(ToString::to_string)
        .find(|path| {
            objects.has(path, FILESYSTEM)
                && objects.mount_points(path).iter().any(|point| point == "/")
        })
        .and_then(|path| objects.path(&path, BLOCK, "Drive"));
    let mut disks = Vec::new();
    for path in objects.paths().map(ToString::to_string) {
        if !objects.has(&path, FILESYSTEM) || objects.flag(&path, BLOCK, "HintIgnore") {
            continue;
        }
        let filesystem = objects.text(&path, BLOCK, "IdType");
        let uuid = objects.text(&path, BLOCK, "IdUUID");
        let drive = objects.path(&path, BLOCK, "Drive");
        if uuid.is_empty()
            || !USABLE_FILESYSTEMS.contains(&filesystem.as_str())
            || (drive.is_some() && drive == startup_drive)
        {
            continue;
        }
        let label = objects.text(&path, BLOCK, "IdLabel");
        let name = if label.is_empty() {
            drive
                .as_deref()
                .map_or_else(String::new, |drive| objects.text(drive, DRIVE, "Model"))
        } else {
            label
        };
        let size = objects.get::<u64>(&path, BLOCK, "Size").unwrap_or(0);
        disks.push(DiskChoice {
            uuid,
            name,
            filesystem,
            size,
        });
    }
    disks.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(disks)
}

pub fn emit_status(bus: &Connection) {
    let interface = bus.object_server().interface::<_, Service>(OBJECT_PATH);
    let Ok(interface) = interface else {
        return;
    };
    let emitter = interface.signal_emitter();
    let guard = interface.get();
    zbus::block_on(async {
        if let Err(error) = guard.status_changed(emitter).await {
            tracing::debug!("cannot announce the backup status: {error}");
        }
    });
}

fn follow_clients(system: &Connection, commands: Sender<Command>) -> zbus::Result<()> {
    let proxy = DBusProxy::new(system)?;
    let changes = proxy.receive_name_owner_changed()?;
    thread::spawn(move || {
        for change in changes {
            let Ok(args) = change.args() else {
                continue;
            };
            if args.new_owner().is_none()
                && commands
                    .send(Command::ClientGone(args.name().to_string()))
                    .is_err()
            {
                return;
            }
        }
    });
    Ok(())
}

pub fn serve() -> zbus::Result<()> {
    let shared = Shared::new();
    let (sender, receiver) = mpsc::channel();
    let service = Service {
        shared: shared.clone(),
        commands: sender.clone(),
    };
    let bus = Builder::system()?
        .serve_at(OBJECT_PATH, service)?
        .name(BUS_NAME)?
        .build()?;
    shared.attach_bus(bus.clone());
    follow_clients(&bus, sender.clone())?;
    stop_when_unplugged(shared.clone());
    Worker::new(shared.clone(), sender).run(&receiver);
    shared.stopper.stop(StopReason::Reconfigured);
    Ok(())
}
