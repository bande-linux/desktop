use zbus::blocking::Connection;
use zbus::blocking::fdo::DBusProxy;
use zbus::names::BusName;

use crate::{Error, NearbyPrinter, Result, queue_name};

const MECHANISM: &str = "org.opensuse.CupsPkHelper.Mechanism";
const NOT_PRIVILEGED: &str = "org.opensuse.CupsPkHelper.Mechanism.NotPrivileged";
const DRIVERLESS: &str = "everywhere";

#[zbus::proxy(
    interface = "org.opensuse.CupsPkHelper.Mechanism",
    default_service = "org.opensuse.CupsPkHelper.Mechanism",
    default_path = "/",
    gen_async = false,
    blocking_name = "MechanismProxy"
)]
trait Mechanism {
    #[zbus(allow_interactive_auth)]
    fn printer_add(
        &self,
        name: &str,
        uri: &str,
        ppd: &str,
        info: &str,
        location: &str,
    ) -> zbus::Result<String>;

    #[zbus(allow_interactive_auth)]
    fn printer_delete(&self, name: &str) -> zbus::Result<String>;

    #[zbus(allow_interactive_auth)]
    fn printer_set_default(&self, name: &str) -> zbus::Result<String>;

    #[zbus(allow_interactive_auth)]
    fn printer_set_enabled(&self, name: &str, enabled: bool) -> zbus::Result<String>;

    #[zbus(allow_interactive_auth)]
    fn printer_set_accept_jobs(
        &self,
        name: &str,
        enabled: bool,
        reason: &str,
    ) -> zbus::Result<String>;

    #[zbus(allow_interactive_auth)]
    fn job_cancel_purge(&self, job: i32, purge: bool) -> zbus::Result<String>;
}

pub fn mechanism_available(system: &Connection) -> bool {
    let Ok(bus) = DBusProxy::new(system) else {
        return false;
    };
    bus.list_activatable_names().is_ok_and(|names| {
        names.iter().any(
            |name| matches!(name.inner(), BusName::WellKnown(known) if known.as_str() == MECHANISM),
        )
    })
}

fn outcome(answer: zbus::Result<String>) -> Result<()> {
    match answer {
        Ok(message) if message.is_empty() => Ok(()),
        Ok(message) => Err(Error::Refused(0, message)),
        Err(zbus::Error::MethodError(name, _, _)) if name.as_str() == NOT_PRIVILEGED => {
            Err(Error::NotAllowed)
        }
        Err(error) => Err(Error::Refused(0, error.to_string())),
    }
}

pub struct Mechanism {
    proxy: MechanismProxy<'static>,
}

impl Mechanism {
    pub fn new(system: &Connection) -> Result<Self> {
        MechanismProxy::new(system)
            .map(|proxy| Self { proxy })
            .map_err(|error| Error::Refused(0, error.to_string()))
    }

    pub fn set_default(&self, name: &str) -> Result<()> {
        outcome(self.proxy.printer_set_default(name))
    }

    pub fn remove(&self, name: &str) -> Result<()> {
        outcome(self.proxy.printer_delete(name))
    }

    pub fn pause(&self, name: &str, paused: bool) -> Result<()> {
        outcome(self.proxy.printer_set_enabled(name, !paused))
    }

    pub fn cancel(&self, job: i32) -> Result<()> {
        outcome(self.proxy.job_cancel_purge(job, false))
    }

    pub fn add(&self, printer: &NearbyPrinter) -> Result<String> {
        let name = queue_name(&printer.name);
        outcome(self.proxy.printer_add(
            &name,
            &printer.uri,
            DRIVERLESS,
            &printer.name,
            &printer.location,
        ))?;
        outcome(self.proxy.printer_set_enabled(&name, true))?;
        outcome(self.proxy.printer_set_accept_jobs(&name, true, ""))?;
        Ok(name)
    }
}
