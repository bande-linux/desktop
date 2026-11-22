use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use bande_l10n::tr;
use bande_notify::Urgency;
use bande_notify::client::{self, Request};
use bande_volumes::udisks::{ATA, Change, DRIVE, Health, NVME, Objects};
use zbus::blocking::Connection;

const FILE: &str = "bande/failing-disks";
const DISKS: &str = "org.bande.Disks";

pub struct FailingDisks {
    session: Connection,
    file: Option<PathBuf>,
    reported: Mutex<BTreeSet<String>>,
}

impl FailingDisks {
    pub fn new(session: &Connection) -> Self {
        let file = bande_xdg::state_home().map(|state| state.join(FILE));
        let reported = match file.as_deref().map(fs::read_to_string) {
            Some(Ok(text)) => text.lines().map(str::to_owned).collect(),
            Some(Err(error)) if error.kind() != io::ErrorKind::NotFound => {
                tracing::warn!("cannot read which failing disks were reported: {error}");
                BTreeSet::new()
            }
            _ => BTreeSet::new(),
        };
        Self {
            session: session.clone(),
            file,
            reported: Mutex::new(reported),
        }
    }

    pub fn changed(&self, objects: &Objects, change: &Change) {
        let drive = match change {
            Change::Added(path) => path,
            Change::Changed(path, interface, _) if interface == ATA || interface == NVME => path,
            Change::Changed(..) | Change::Removed(..) => return,
        };
        let id = objects.text(drive, DRIVE, "Id");
        let Some(health) = objects.health(drive).filter(|_| !id.is_empty()) else {
            return;
        };
        let failing = health == Health::Failing;
        let mut reported = self.reported.lock().unwrap_or_else(PoisonError::into_inner);
        let changed = match failing {
            true => reported.insert(id),
            false => reported.remove(&id),
        };
        if !changed {
            return;
        }
        if let Some(file) = &self.file {
            let lines: String = reported.iter().map(|id| format!("{id}\n")).collect();
            if let Err(error) = bande_xdg::replace_file(file, lines) {
                tracing::warn!("cannot remember which failing disks were reported: {error}");
            }
        }
        drop(reported);
        if failing {
            self.notify(objects, drive);
        }
    }

    fn notify(&self, objects: &Objects, drive: &str) {
        let name = [
            objects.text(drive, DRIVE, "Vendor"),
            objects.text(drive, DRIVE, "Model"),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
        let name = match name.is_empty() {
            true => objects.text(drive, DRIVE, "Id"),
            false => name,
        };
        let posted = client::notify(
            &self.session,
            &Request {
                app_name: &tr!("Disks"),
                replaces_id: 0,
                app_icon: "drive-harddisk",
                summary: &tr!("A Disk Is Failing"),
                body: &tr!(
                    "“{name}” reports that it is about to fail. Back up its files now and replace the disk.",
                    name = name
                ),
                actions: &[],
                urgency: Urgency::Critical,
                sound: None,
                desktop_entry: Some(DISKS),
            },
        );
        if let Err(error) = posted {
            tracing::warn!("cannot report a failing disk: {error}");
        }
    }
}
