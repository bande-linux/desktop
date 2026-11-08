use std::collections::HashMap;
use std::io;
use std::process::Command;

use bande_volumes::eject_block_unmount_only;
use bande_volumes::udisks::{BLOCK, Objects, UDISKS};
use zbus::blocking::Connection;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};

use crate::store::Destination;

const BLOCK_INTERFACE: &str = "org.freedesktop.UDisks2.Block";
const TABLE_INTERFACE: &str = "org.freedesktop.UDisks2.PartitionTable";
const NEW_LABEL: &str = "Bande Backup";
const NEW_FILESYSTEM: &str = "ext4";

fn failed(error: zbus::Error) -> io::Error {
    io::Error::other(error.to_string())
}

pub fn erase(block: &str) -> io::Result<Destination> {
    let system = Connection::system().map_err(failed)?;
    let objects = Objects::load(&system).map_err(failed)?;
    eject_block_unmount_only(&system, &objects, block).map_err(failed)?;
    let path = ObjectPath::try_from(block).map_err(|error| io::Error::other(error.to_string()))?;
    let no_options: HashMap<&str, Value<'_>> = HashMap::new();
    system
        .call_method(
            Some(UDISKS),
            &path,
            Some(BLOCK_INTERFACE),
            "Format",
            &("gpt", HashMap::from([("tear-down", Value::from(true))])),
        )
        .map_err(failed)?;
    let format_options: HashMap<&str, Value<'_>> =
        HashMap::from([("label", Value::from(NEW_LABEL))]);
    let reply = system
        .call_method(
            Some(UDISKS),
            &path,
            Some(TABLE_INTERFACE),
            "CreatePartitionAndFormat",
            &(
                0_u64,
                0_u64,
                "",
                "BandeBackup",
                &no_options,
                NEW_FILESYSTEM,
                &format_options,
            ),
        )
        .map_err(failed)?;
    let created: OwnedObjectPath = reply.body().deserialize().map_err(failed)?;
    let objects = Objects::load(&system).map_err(failed)?;
    let device = objects.device(created.as_str());
    match Command::new("tune2fs")
        .args(["-m", "0"])
        .arg(&device)
        .output()
    {
        Ok(output) if output.status.success() => {}
        _ => tracing::warn!("cannot free the reserved blocks on {device}"),
    }
    let uuid = objects.text(created.as_str(), BLOCK, "IdUUID");
    if uuid.is_empty() {
        return Err(io::Error::other("the new filesystem has no identifier"));
    }
    Ok(Destination::Disk {
        uuid,
        name: NEW_LABEL.to_owned(),
        filesystem: NEW_FILESYSTEM.to_owned(),
    })
}
