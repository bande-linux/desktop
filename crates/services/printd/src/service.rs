use std::sync::Arc;

use bande_printers::{
    Capabilities, JobInfo, JobOptions, PrinterInfo, ScanCapabilities, ScanOptions, ScannerInfo,
};
use zbus::fdo;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedFd;

use crate::queue::Spool;

pub struct Service {
    pub spool: Arc<Spool>,
}

fn failed(error: impl std::fmt::Display) -> fdo::Error {
    fdo::Error::Failed(error.to_string())
}

#[zbus::interface(name = "org.bande.Print")]
impl Service {
    async fn watch(&self, #[zbus(header)] header: Header<'_>) {
        if let Some(sender) = header.sender() {
            let (spool, sender) = (Arc::clone(&self.spool), sender.to_string());
            blocking::unblock(move || spool.watch(sender)).await;
        }
    }

    fn needs_avahi(&self) -> bool {
        self.spool.needs_avahi()
    }

    fn printers(&self) -> Vec<PrinterInfo> {
        self.spool.printers()
    }

    async fn capabilities(&self, printer: String) -> fdo::Result<Capabilities> {
        let spool = Arc::clone(&self.spool);
        blocking::unblock(move || spool.capabilities(&printer))
            .await
            .map_err(failed)
    }

    async fn submit(
        &self,
        title: String,
        printer: String,
        options: JobOptions,
        document: OwnedFd,
    ) -> fdo::Result<u32> {
        let spool = Arc::clone(&self.spool);
        blocking::unblock(move || spool.submit(&title, &printer, options, document.into()))
            .await
            .map_err(failed)
    }

    fn jobs(&self) -> Vec<JobInfo> {
        self.spool.jobs()
    }

    fn cancel(&self, job: u32) {
        self.spool.cancel(job);
    }

    fn set_default(&self, printer: String) {
        self.spool.set_default(&printer);
    }

    async fn add_address(&self, address: String) -> fdo::Result<String> {
        let spool = Arc::clone(&self.spool);
        blocking::unblock(move || spool.add_address(&address))
            .await
            .map_err(failed)
    }

    fn forget(&self, printer: String) {
        self.spool.forget(&printer);
    }

    fn scanners(&self) -> Vec<ScannerInfo> {
        self.spool.scanners()
    }

    async fn scan_capabilities(&self, scanner: String) -> fdo::Result<ScanCapabilities> {
        let spool = Arc::clone(&self.spool);
        blocking::unblock(move || spool.scan_capabilities(&scanner))
            .await
            .map_err(fdo::Error::Failed)
    }

    async fn scan(
        &self,
        scanner: String,
        token: u64,
        options: ScanOptions,
    ) -> fdo::Result<Vec<(OwnedFd, u32)>> {
        let spool = Arc::clone(&self.spool);
        let running = spool
            .begin_scan(&scanner, token)
            .map_err(fdo::Error::Failed)?;
        blocking::unblock(move || spool.scan(&running, &options))
            .await
            .map(|pages| {
                pages
                    .into_iter()
                    .map(|(page, dpi)| (OwnedFd::from(page), dpi))
                    .collect()
            })
            .map_err(fdo::Error::Failed)
    }

    fn cancel_scan(&self, scanner: String, token: u64) {
        self.spool.cancel_scan(&scanner, token);
    }

    #[zbus(signal)]
    pub async fn changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}
