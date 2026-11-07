use std::os::fd::OwnedFd;

use serde::{Deserialize, Serialize};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{Fd, Type};

use crate::ipp::Request;

pub const BUS_NAME: &str = "org.bande.Print";
pub const OBJECT_PATH: &str = "/org/bande/Print";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum Kind {
    Direct,
    Address,
    Cups,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PrinterInfo {
    pub id: String,
    pub name: String,
    pub model: String,
    pub location: String,
    pub kind: Kind,
    pub default: bool,
    pub reachable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
pub struct Capabilities {
    pub media: Vec<String>,
    pub default_media: String,
    pub two_sided: bool,
    pub color: bool,
    pub maximum_copies: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct JobOptions {
    pub media: String,
    pub landscape: bool,
    pub sides: String,
    pub color: bool,
    pub copies: i32,
    pub pages: Vec<(i32, i32)>,
}

impl JobOptions {
    pub fn write_to(&self, request: &mut Request) {
        if !self.media.is_empty() {
            request.string(crate::ipp::KEYWORD, "media", &self.media);
        }
        if self.landscape {
            request.integer(crate::ipp::ENUM, "orientation-requested", LANDSCAPE);
        }
        if !self.sides.is_empty() {
            request.string(crate::ipp::KEYWORD, "sides", &self.sides);
        }
        request.string(
            crate::ipp::KEYWORD,
            "print-color-mode",
            if self.color { "color" } else { "monochrome" },
        );
        if self.copies > 1 {
            request.integer(crate::ipp::INTEGER, "copies", self.copies);
        }
        if !self.pages.is_empty() {
            request.ranges("page-ranges", &self.pages);
        }
    }
}

const LANDSCAPE: i32 = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct JobInfo {
    pub id: u32,
    pub title: String,
    pub printer: String,
    pub state: String,
    pub message: String,
}

pub struct PrintClient<'a> {
    proxy: Proxy<'a>,
}

impl<'a> PrintClient<'a> {
    pub fn new(session: &Connection) -> zbus::Result<Self> {
        Ok(Self {
            proxy: Proxy::new(session, BUS_NAME, OBJECT_PATH, BUS_NAME)?,
        })
    }

    pub fn watch(&self) -> zbus::Result<()> {
        self.proxy.call("Watch", &())
    }

    pub fn printers(&self) -> zbus::Result<Vec<PrinterInfo>> {
        self.proxy.call("Printers", &())
    }

    pub fn capabilities(&self, printer: &str) -> zbus::Result<Capabilities> {
        self.proxy.call("Capabilities", &(printer,))
    }

    pub fn submit(
        &self,
        title: &str,
        printer: &str,
        options: &JobOptions,
        document: OwnedFd,
    ) -> zbus::Result<u32> {
        self.proxy
            .call("Submit", &(title, printer, options, Fd::from(document)))
    }

    pub fn jobs(&self) -> zbus::Result<Vec<JobInfo>> {
        self.proxy.call("Jobs", &())
    }

    pub fn cancel(&self, job: u32) -> zbus::Result<()> {
        self.proxy.call("Cancel", &(job,))
    }

    pub fn set_default(&self, printer: &str) -> zbus::Result<()> {
        self.proxy.call("SetDefault", &(printer,))
    }

    pub fn add_address(&self, address: &str) -> zbus::Result<String> {
        self.proxy.call("AddAddress", &(address,))
    }

    pub fn forget(&self, printer: &str) -> zbus::Result<()> {
        self.proxy.call("Forget", &(printer,))
    }

    pub fn changes(&self) -> zbus::Result<zbus::blocking::proxy::SignalIterator<'a>> {
        self.proxy.receive_signal("Changed")
    }
}
