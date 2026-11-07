use std::collections::HashMap;
use std::path::Path;
use std::process::Command as Process;
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use bande_packages::{Client, Info, Mode};

use bande_volumes::udisks::{BLOCK, DRIVE, FILESYSTEM, Objects, PARTITION, SWAPSPACE};
use zbus::blocking::Connection;
use zbus::blocking::connection::Builder;
use zbus::blocking::fdo::DBusProxy;
use zbus::message::Header;
use zbus::zvariant::Value;
use zbus::{fdo, interface};

use crate::client::{BUS_NAME, DiskChoice, OBJECT_PATH, Status, setup_error};
use crate::disk::{self, NetworkPlace};
use crate::restic::StopReason;
use crate::store::Destination;
use crate::worker::{Command, Shared, Worker, stop_when_unplugged};

pub const HOUR: u64 = 3600;
pub const DAY: u64 = 24 * HOUR;
const CONFIGURE: &str = "org.bande.backup.configure";
const CONTROL: &str = "org.bande.backup.control";
const ALLOW_INTERACTION: u32 = 1;
const RELEASE_LIMIT: Duration = Duration::from_secs(40);
const MEDIA_ROOTS: [&str; 5] = [
    "/run/media",
    "/media",
    "/mnt",
    "/run/user",
    "/run/bande-backup",
];
const USABLE_FILESYSTEMS: [&str; 7] = ["ext4", "ext3", "btrfs", "xfs", "f2fs", "exfat", "vfat"];

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

fn check_place(url: &str, user: &str, share_password: &str) -> String {
    let Some(place) = disk::parse_network(url) else {
        return setup_error::BAD_ADDRESS.to_owned();
    };
    if let Some(package) = place.missing_package()
        && let Err(error) = install_package(package)
    {
        tracing::warn!("cannot install {package}: {error}");
        return setup_error::FAILED.to_owned();
    }
    if !place.is_reachable() {
        return setup_error::UNREACHABLE.to_owned();
    }
    if matches!(place, NetworkPlace::Rest(_)) {
        return "new".to_owned();
    }
    match disk::peek_share(&place, user, share_password) {
        Ok(true) => "existing".to_owned(),
        Ok(false) => "new".to_owned(),
        Err(error) => {
            tracing::warn!("cannot connect to {url}: {error}");
            setup_error::UNREACHABLE.to_owned()
        }
    }
}

fn install_package(package: &str) -> Result<(), String> {
    let system = Connection::system().map_err(|error| error.to_string())?;
    let client = Client::new(&system);
    let ids: Vec<String> = client
        .resolve(&[package.to_owned()])
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|found| found.info == Info::Available)
        .map(|found| found.id)
        .collect();
    if !ids.is_empty() {
        client
            .install(&ids, Mode::Apply, &|_| {})
            .map_err(|error| error.to_string())?;
    }
    if package == "nfs-common" {
        let quiet = Process::new("systemctl")
            .args(["disable", "--now", "rpcbind.socket", "rpcbind.service"])
            .status();
        if let Err(error) = quiet {
            tracing::warn!("cannot stop rpcbind: {error}");
        }
    }
    Ok(())
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

    async fn disks(
        &self,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<Vec<DiskChoice>> {
        authorize(connection, &header, CONTROL).await?;
        usable_disks_off_the_bus().await
    }

    async fn has_backups(
        &self,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
        uuid: String,
    ) -> fdo::Result<bool> {
        authorize(connection, &header, CONTROL).await?;
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
            destination: Destination::Disk {
                uuid: choice.uuid,
                name: choice.name,
                filesystem: choice.filesystem,
            },
            share_password: String::new(),
            password,
        })
    }

    async fn check_network(
        &self,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
        url: String,
        user: String,
        share_password: String,
    ) -> fdo::Result<String> {
        authorize(connection, &header, CONFIGURE).await?;
        Ok(blocking::unblock(move || check_place(&url, &user, &share_password)).await)
    }

    async fn choose_network(
        &self,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
        url: String,
        user: String,
        share_password: String,
        password: String,
    ) -> fdo::Result<()> {
        authorize(connection, &header, CONFIGURE).await?;
        if disk::parse_network(&url).is_none() {
            return Err(fdo::Error::InvalidArgs(url));
        }
        let name = url
            .split("://")
            .nth(1)
            .unwrap_or(url.as_str())
            .trim_end_matches('/')
            .to_owned();
        self.send(Command::Choose {
            destination: Destination::Network { url, user, name },
            share_password,
            password,
        })
    }

    async fn erase_and_choose(
        &self,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
        block: String,
        password: String,
    ) -> fdo::Result<()> {
        authorize(connection, &header, CONFIGURE).await?;
        let known = usable_disks_off_the_bus()
            .await?
            .iter()
            .any(|disk| disk.block == block);
        if !known {
            return Err(fdo::Error::AccessDenied("not an erasable disk".to_owned()));
        }
        self.send(Command::EraseAndChoose { block, password })
    }

    async fn set_excluded(
        &self,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
        folders: Vec<String>,
    ) -> fdo::Result<()> {
        authorize(connection, &header, CONFIGURE).await?;
        if folders
            .iter()
            .any(|folder| !Path::new(folder).is_absolute())
        {
            return Err(fdo::Error::InvalidArgs(
                "folders must be absolute".to_owned(),
            ));
        }
        self.send(Command::SetExcluded(folders))
    }

    async fn release(
        &self,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        authorize(connection, &header, CONTROL).await?;
        let (done, finished) = mpsc::channel();
        self.send(Command::Release(done))?;
        blocking::unblock(move || finished.recv_timeout(RELEASE_LIMIT))
            .await
            .map_err(|_| fdo::Error::Failed("the backup is still stopping".to_owned()))
    }

    async fn stop_using(
        &self,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        authorize(connection, &header, CONFIGURE).await?;
        self.send(Command::StopUsing)
    }

    fn wake(&self) -> fdo::Result<()> {
        self.send(Command::Wake)
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

fn is_media_mount(point: &str) -> bool {
    MEDIA_ROOTS
        .iter()
        .any(|root| Path::new(point).starts_with(root))
}

fn drive_of(objects: &Objects, block: &str) -> Option<String> {
    let backing = objects.path(block, BLOCK, "CryptoBackingDevice");
    let underlying = backing.as_deref().unwrap_or(block);
    objects.path(underlying, BLOCK, "Drive")
}

fn usable_disks() -> zbus::Result<Vec<DiskChoice>> {
    let system = Connection::system()?;
    let objects = Objects::load(&system)?;
    let paths: Vec<String> = objects.paths().map(ToString::to_string).collect();
    let in_use: Vec<String> = paths
        .iter()
        .filter(|path| {
            objects.flag(path, BLOCK, "HintSystem")
                && objects.mount_points(path).iter().any(|point| point == "/")
                || objects.has(path, SWAPSPACE) && objects.flag(path, SWAPSPACE, "Active")
                || objects
                    .mount_points(path)
                    .iter()
                    .any(|point| !is_media_mount(point))
        })
        .filter_map(|path| drive_of(&objects, path))
        .collect();
    let mut disks = Vec::new();
    let mut drives_with_choice = Vec::new();
    for path in &paths {
        if !objects.has(path, FILESYSTEM) || objects.flag(path, BLOCK, "HintIgnore") {
            continue;
        }
        let Some(drive) = drive_of(&objects, path) else {
            continue;
        };
        let filesystem = objects.text(path, BLOCK, "IdType");
        let uuid = objects.text(path, BLOCK, "IdUUID");
        if uuid.is_empty()
            || !USABLE_FILESYSTEMS.contains(&filesystem.as_str())
            || in_use.contains(&drive)
        {
            continue;
        }
        let label = objects.text(path, BLOCK, "IdLabel");
        let model = objects.text(&drive, DRIVE, "Model");
        let name = if label.is_empty() {
            model.clone()
        } else {
            label
        };
        let size = objects.get::<u64>(path, BLOCK, "Size").unwrap_or(0);
        drives_with_choice.push(drive.clone());
        disks.push(with_drive(
            &objects,
            &paths,
            &drive,
            DiskChoice {
                uuid,
                name,
                filesystem,
                size,
                block: String::new(),
                drive_name: model,
                drive_size: 0,
                contents: String::new(),
            },
        ));
    }
    for path in &paths {
        let Some(drive) = objects.path(path, BLOCK, "Drive") else {
            continue;
        };
        let whole = objects.has(path, BLOCK)
            && !objects.has(path, PARTITION)
            && objects.path(path, BLOCK, "CryptoBackingDevice").is_none();
        if !whole
            || in_use.contains(&drive)
            || drives_with_choice.contains(&drive)
            || objects.flag(path, BLOCK, "HintIgnore")
        {
            continue;
        }
        let model = objects.text(&drive, DRIVE, "Model");
        disks.push(with_drive(
            &objects,
            &paths,
            &drive,
            DiskChoice {
                uuid: String::new(),
                name: model.clone(),
                filesystem: objects.text(path, BLOCK, "IdType"),
                size: objects.get::<u64>(path, BLOCK, "Size").unwrap_or(0),
                block: String::new(),
                drive_name: model,
                drive_size: 0,
                contents: String::new(),
            },
        ));
    }
    disks.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(disks)
}

fn with_drive(objects: &Objects, paths: &[String], drive: &str, choice: DiskChoice) -> DiskChoice {
    let mut block = String::new();
    let mut drive_size = 0;
    let mut labels = Vec::new();
    for path in paths {
        if objects.path(path, BLOCK, "Drive").as_deref() != Some(drive) {
            continue;
        }
        if !objects.has(path, PARTITION) {
            block.clone_from(path);
            drive_size = objects.get::<u64>(path, BLOCK, "Size").unwrap_or(0);
        }
        let label = objects.text(path, BLOCK, "IdLabel");
        if !label.is_empty() {
            labels.push(label);
        }
    }
    labels.sort();
    DiskChoice {
        block,
        drive_size,
        contents: labels.join(", "),
        ..choice
    }
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
