use std::collections::HashMap;
use std::os::fd::AsFd;
use std::sync::Mutex;

use bande_l10n::tr;
use zbus::fdo::DBusProxy;
use zbus::names::BusName;

use crate::manifest::Manifests;
use crate::plan::locked;
use crate::{Reason, Refusal};

const REMEMBERED: usize = 64;
const PASSING_ON: [&str; 2] = ["org.bande.Agents", "org.bande.Routines"];

/// Which connections may call, and the program each is.
#[derive(Default)]
pub(crate) struct Callers(Mutex<HashMap<String, String>>);

fn refused() -> Refusal {
    Refusal::new(
        Reason::AccessDenied,
        tr!("A program in a sandbox cannot do this"),
    )
}

fn program_of(process: u32) -> Option<String> {
    let root = format!("/proc/{process}");
    let same = std::fs::read_link(format!("{root}/ns/mnt")).ok()?
        == std::fs::read_link("/proc/self/ns/mnt").ok()?;
    let sandboxed = std::path::Path::new(&format!("{root}/root/.flatpak-info")).exists();
    let executable = std::fs::read_link(format!("{root}/exe")).ok()?;
    let name = executable.file_name()?.to_string_lossy().into_owned();
    (same && !sandboxed).then_some(name)
}

impl Callers {
    /// The program behind `sender`, refused when it runs in a sandbox.
    pub(crate) async fn admit(
        &self,
        bus: &zbus::Connection,
        sender: &str,
    ) -> Result<String, Refusal> {
        if let Some(program) = locked(&self.0).get(sender) {
            return Ok(program.clone());
        }
        let name = BusName::try_from(sender.to_owned()).map_err(zbus::Error::from)?;
        let credentials = DBusProxy::new(bus)
            .await?
            .get_connection_credentials(name)
            .await
            .map_err(zbus::Error::from)?;
        let process = match credentials.process_fd() {
            Some(handle) => bande_bus::pid_of_handle(handle.as_fd()),
            None => credentials.process_id(),
        };
        let program = process.and_then(program_of).ok_or_else(refused)?;
        let mut known = locked(&self.0);
        if known.len() >= REMEMBERED {
            known.clear();
        }
        known.insert(sender.to_owned(), program.clone());
        Ok(program)
    }

    /// Whether `sender` may pass on the name of an agent.
    pub(crate) async fn passes_on(
        &self,
        bus: &zbus::Connection,
        sender: &str,
        manifests: &Manifests,
    ) -> Result<(), Refusal> {
        let proxy = DBusProxy::new(bus).await?;
        for name in PASSING_ON.into_iter().chain(manifests.bus_names()) {
            let Ok(name) = BusName::try_from(name.to_owned()) else {
                continue;
            };
            if proxy
                .get_name_owner(name)
                .await
                .is_ok_and(|owner| owner.as_str() == sender)
            {
                return Ok(());
            }
        }
        Err(Refusal::new(
            Reason::AccessDenied,
            tr!("This program cannot act for an agent"),
        ))
    }
}
