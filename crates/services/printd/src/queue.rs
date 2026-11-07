use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use bande_notify::Urgency;
use bande_notify::client::{self, Request};
use bande_printers::{Cups, Error, JobInfo, JobOptions, Kind, PrinterInfo, Result};
use zbus::blocking::Connection;

use crate::L10N;
use crate::link::{Address, is_identity_change, send_document};
use crate::mdns::Found;
use crate::printer::{
    Payload, RemoteState, abilities, accepted_job, cancel_job, job_state, payload_for,
    print_request, printer_state_reasons,
};
use crate::raster::write_document;
use crate::store::{Known, Pins, Printers, StoredJob, queue_folder};

const CUPS_PREFIX: &str = "cups:";
const PDF_MAGIC: &[u8] = b"%PDF-";
const LARGEST_DOCUMENT: u64 = 4 << 30;
const FOLLOW_EVERY: Duration = Duration::from_secs(5);
const FIRST_RETRY: Duration = Duration::from_secs(15);
const LONGEST_RETRY: Duration = Duration::from_secs(300);
const FOLLOW_FAILURES: u32 = 6;
const STAY_AFTER_IDLE: Duration = Duration::from_secs(30);
const IPP_USB: &str = "/usr/sbin/ipp-usb";
const IPP_USB_PORTS: std::ops::Range<u16> = 60000..60004;
const BUSY: u16 = 0x0507;
const FORMAT_REFUSED: u16 = 0x040A;
const TROUBLES: [&str; 8] = [
    "media-empty",
    "media-jam",
    "media-needed",
    "toner-empty",
    "marker-supply-empty",
    "door-open",
    "cover-open",
    "paused",
];

struct Job {
    stored: StoredJob,
    state: &'static str,
    message: String,
    cancelled: Arc<AtomicBool>,
}

#[derive(Default)]
struct Inner {
    printers: Printers,
    found: HashMap<String, Found>,
    usb: HashSet<String>,
    watchers: HashSet<String>,
    jobs: Vec<Job>,
    sending_to: HashSet<String>,
    next_job: u32,
    stop_browsing: Option<std::os::unix::net::UnixStream>,
    last_busy: Option<Instant>,
}

pub struct Spool {
    inner: Mutex<Inner>,
    changed: Condvar,
    pins: Pins,
    session: Connection,
    announce: Box<dyn Fn() + Send + Sync>,
}

fn document_path(id: u32) -> Option<PathBuf> {
    queue_folder().map(|folder| folder.join(format!("{id}.pdf")))
}

fn record_path(id: u32) -> Option<PathBuf> {
    queue_folder().map(|folder| folder.join(format!("{id}.json")))
}

impl Spool {
    pub fn new(session: Connection, announce: Box<dyn Fn() + Send + Sync>) -> Arc<Self> {
        let mut inner = Inner {
            printers: Printers::load(),
            next_job: 1,
            last_busy: Some(Instant::now()),
            ..Inner::default()
        };
        if let Some(folder) = queue_folder()
            && let Ok(entries) = std::fs::read_dir(folder)
        {
            for entry in entries.flatten() {
                let stored = std::fs::read(entry.path())
                    .ok()
                    .filter(|_| entry.path().extension().is_some_and(|kind| kind == "json"))
                    .and_then(|bytes| serde_json::from_slice::<StoredJob>(&bytes).ok());
                if let Some(stored) = stored {
                    inner.next_job = inner.next_job.max(stored.id + 1);
                    inner.jobs.push(Job {
                        stored,
                        state: "waiting",
                        message: String::new(),
                        cancelled: Arc::default(),
                    });
                }
            }
        }
        inner.jobs.sort_by_key(|job| job.stored.id);
        let spool = Arc::new(Self {
            inner: Mutex::new(inner),
            changed: Condvar::new(),
            pins: Pins::load(),
            session,
            announce,
        });
        spool.start_workers();
        spool
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn touched(&self) {
        self.changed.notify_all();
        (self.announce)();
    }

    pub fn wait_until_idle(&self) {
        let mut inner = self.lock();
        loop {
            let busy = !inner.watchers.is_empty() || !inner.jobs.is_empty();
            if busy {
                inner.last_busy = None;
                inner = self
                    .changed
                    .wait(inner)
                    .unwrap_or_else(PoisonError::into_inner);
                continue;
            }
            let since = *inner.last_busy.get_or_insert_with(Instant::now);
            let left = STAY_AFTER_IDLE.saturating_sub(since.elapsed());
            if left.is_zero() {
                return;
            }
            inner = self
                .changed
                .wait_timeout(inner, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    pub fn watch(self: &Arc<Self>, watcher: String) {
        self.lock().watchers.insert(watcher);
        self.update_browsing();
        self.touched();
    }

    pub fn unwatch(self: &Arc<Self>, watcher: &str) {
        let removed = self.lock().watchers.remove(watcher);
        if removed {
            self.update_browsing();
            self.touched();
        }
    }

    fn update_browsing(self: &Arc<Self>) {
        let mut inner = self.lock();
        let wanted = !inner.watchers.is_empty()
            || inner
                .jobs
                .iter()
                .any(|job| !job.stored.printer.starts_with(CUPS_PREFIX));
        match (wanted, inner.stop_browsing.is_some()) {
            (true, false) => {
                let Ok((keep, wake)) = std::os::unix::net::UnixStream::pair() else {
                    return;
                };
                inner.stop_browsing = Some(keep);
                drop(inner);
                let usb = Arc::clone(self);
                std::thread::spawn(move || usb.look_for_usb_printers());
                let spool = Arc::clone(self);
                std::thread::spawn(move || {
                    let browsing = crate::mdns::browse(&wake, |found| {
                        spool.found(found);
                        true
                    });
                    if let Err(error) = browsing {
                        tracing::warn!("cannot look for printers: {error}");
                    }
                });
            }
            (false, true) => {
                inner.stop_browsing = None;
                inner.found.clear();
            }
            _ => {}
        }
    }

    fn look_for_usb_printers(&self) {
        if !Path::new(IPP_USB).exists() {
            return;
        }
        for port in IPP_USB_PORTS {
            let uri = format!("ipp://localhost:{port}/ipp/print");
            let Some(address) = Address::parse(&uri) else {
                continue;
            };
            let Ok((_, model)) = abilities(&address, &uri, &self.pins) else {
                break;
            };
            let id = format!("usb:{port}");
            let mut inner = self.lock();
            inner.printers.known.insert(
                id.clone(),
                Known {
                    name: model.clone(),
                    model,
                    plain: Some(uri),
                    ..Known::default()
                },
            );
            inner.usb.insert(id);
        }
        self.touched();
    }

    fn found(&self, found: Vec<Found>) {
        let mut inner = self.lock();
        let previous = std::mem::take(&mut inner.found);
        for printer in found {
            let id = printer.id();
            let known = inner.printers.known.entry(id.clone()).or_default();
            known.name.clone_from(&printer.instance);
            known.model.clone_from(&printer.model);
            known.location.clone_from(&printer.location);
            known.plain.clone_from(&printer.plain);
            known.secure.clone_from(&printer.secure);
            inner.found.insert(id, printer);
        }
        let changed =
            previous.keys().collect::<HashSet<_>>() != inner.found.keys().collect::<HashSet<_>>();
        if changed {
            inner.printers.save();
        }
        drop(inner);
        self.touched();
    }

    pub fn printers(&self) -> Vec<PrinterInfo> {
        let inner = self.lock();
        let default = inner.printers.default.clone();
        let cups = Cups::new();
        let queues = match cups.installed() {
            true => cups.printers().unwrap_or_default(),
            false => Vec::new(),
        };
        let mut list: Vec<PrinterInfo> = inner
            .printers
            .known
            .iter()
            .filter(|(id, known)| {
                (inner.found.contains_key(*id)
                    || inner.usb.contains(*id)
                    || known.by_address
                    || known.used)
                    && !queues.iter().any(|queue| queue.title() == known.name)
            })
            .map(|(id, known)| PrinterInfo {
                id: id.clone(),
                name: known.name.clone(),
                model: known.model.clone(),
                location: known.location.clone(),
                kind: if known.by_address {
                    Kind::Address
                } else {
                    Kind::Direct
                },
                default: default.as_ref() == Some(id),
                reachable: inner.found.contains_key(id)
                    || inner.usb.contains(id)
                    || known.by_address,
            })
            .collect();
        list.extend(queues.iter().map(|queue| {
            let id = format!("{CUPS_PREFIX}{}", queue.name);
            PrinterInfo {
                default: default.as_ref() == Some(&id) || (default.is_none() && queue.default),
                id,
                name: queue.title().to_owned(),
                model: queue.model.clone(),
                location: queue.location.clone(),
                kind: Kind::Cups,
                reachable: true,
            }
        }));
        list.sort_by_key(|printer| printer.name.to_lowercase());
        list
    }

    pub fn capabilities(&self, printer: &str) -> Result<bande_printers::Capabilities> {
        if printer.starts_with(CUPS_PREFIX) {
            return Ok(bande_printers::Capabilities {
                media: crate::CUPS_MEDIA.iter().map(ToString::to_string).collect(),
                default_media: String::new(),
                two_sided: true,
                color: true,
                maximum_copies: 999,
            });
        }
        let known = self.known(printer).ok_or(Error::Malformed)?;
        let (found, _) = self.reach(&known)?;
        Ok(found.public())
    }

    fn known(&self, printer: &str) -> Option<Known> {
        self.lock().printers.known.get(printer).cloned()
    }

    fn reach(&self, known: &Known) -> Result<(crate::printer::Abilities, String)> {
        let mut last = Error::Malformed;
        for uri in [&known.secure, &known.plain].into_iter().flatten() {
            let Some(address) = Address::parse(uri) else {
                continue;
            };
            match abilities(&address, uri, &self.pins) {
                Ok((found, _)) => return Ok((found, uri.clone())),
                Err(error) if is_identity_change(&error) => return Err(error),
                Err(error) => last = error,
            }
        }
        Err(last)
    }

    pub fn set_default(&self, printer: &str) {
        let mut inner = self.lock();
        inner.printers.default = Some(printer.to_owned());
        inner.printers.save();
        drop(inner);
        self.touched();
    }

    pub fn forget(&self, printer: &str) {
        let mut inner = self.lock();
        let forgotten = inner.printers.known.remove(printer);
        if inner.printers.default.as_deref() == Some(printer) {
            inner.printers.default = None;
        }
        inner.printers.save();
        drop(inner);
        for uri in forgotten
            .iter()
            .flat_map(|known| [&known.secure, &known.plain])
            .flatten()
        {
            if let Some(address) = Address::parse(uri) {
                self.pins.forget(&address.host);
            }
        }
        self.touched();
    }

    pub fn add_address(&self, address: &str) -> Result<String> {
        let address = address.trim();
        let uri = match address.contains("://") {
            true => address.to_owned(),
            false => format!("ipp://{address}/ipp/print"),
        };
        let mut known = Known {
            name: address.to_owned(),
            by_address: true,
            ..Known::default()
        };
        match uri.starts_with("ipps://") {
            true => known.secure = Some(uri.clone()),
            false => known.plain = Some(uri.clone()),
        }
        let (_, model) = Address::parse(&uri)
            .ok_or(Error::Malformed)
            .and_then(|parsed| abilities(&parsed, &uri, &self.pins))?;
        if !model.is_empty() {
            known.name.clone_from(&model);
            known.model = model;
        }
        let id = uri.clone();
        let mut inner = self.lock();
        inner.printers.known.insert(id.clone(), known);
        inner.printers.save();
        drop(inner);
        self.touched();
        Ok(id)
    }

    pub fn jobs(&self) -> Vec<JobInfo> {
        self.lock()
            .jobs
            .iter()
            .map(|job| JobInfo {
                id: job.stored.id,
                title: job.stored.title.clone(),
                printer: job.stored.printer.clone(),
                state: job.state.to_owned(),
                message: job.message.clone(),
            })
            .collect()
    }

    pub fn cancel(&self, id: u32) {
        let inner = self.lock();
        if let Some(job) = inner.jobs.iter().find(|job| job.stored.id == id) {
            job.cancelled.store(true, Ordering::Relaxed);
        }
        drop(inner);
        self.touched();
    }

    pub fn submit(
        self: &Arc<Self>,
        title: &str,
        printer: &str,
        options: JobOptions,
        document: OwnedFd,
    ) -> io::Result<u32> {
        let id = {
            let mut inner = self.lock();
            let id = inner.next_job;
            inner.next_job += 1;
            id
        };
        let (Some(path), Some(record)) = (document_path(id), record_path(id)) else {
            return Err(io::Error::other("no place to keep the document"));
        };
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let mut source = io::Read::take(File::from(document), LARGEST_DOCUMENT);
        let mut kept = File::create(&path)?;
        io::copy(&mut source, &mut kept)?;
        kept.flush()?;
        let mut magic = [0; 5];
        let starts = std::fs::File::open(&path)
            .and_then(|mut file| io::Read::read_exact(&mut file, &mut magic));
        if starts.is_err() || magic != *PDF_MAGIC {
            let _ = std::fs::remove_file(&path);
            return Err(io::Error::other("the document is not a PDF"));
        }
        let stored = StoredJob {
            id,
            title: title.to_owned(),
            printer: printer.to_owned(),
            options,
        };
        std::fs::write(&record, serde_json::to_vec(&stored)?)?;
        let mut inner = self.lock();
        if let Some(known) = inner.printers.known.get_mut(printer) {
            known.used = true;
        }
        inner
            .printers
            .default
            .get_or_insert_with(|| printer.to_owned());
        inner.printers.save();
        inner.jobs.push(Job {
            stored,
            state: "waiting",
            message: String::new(),
            cancelled: Arc::default(),
        });
        drop(inner);
        self.update_browsing();
        self.start_workers();
        self.touched();
        Ok(id)
    }

    fn start_workers(self: &Arc<Self>) {
        let mut inner = self.lock();
        let printers: HashSet<String> = inner
            .jobs
            .iter()
            .map(|job| job.stored.printer.clone())
            .collect();
        for printer in printers {
            if inner.sending_to.insert(printer.clone()) {
                let spool = Arc::clone(self);
                std::thread::spawn(move || spool.send_all(&printer));
            }
        }
    }

    fn set_state(&self, id: u32, state: &'static str, message: String) {
        let mut inner = self.lock();
        let Some(job) = inner.jobs.iter_mut().find(|job| job.stored.id == id) else {
            return;
        };
        if job.state == state && job.message == message {
            return;
        }
        job.state = state;
        job.message = message;
        drop(inner);
        (self.announce)();
    }

    fn send_all(self: Arc<Self>, printer: &str) {
        loop {
            let mut inner = self.lock();
            let next = inner
                .jobs
                .iter()
                .find(|job| job.stored.printer == printer)
                .map(|job| (job.stored.clone(), Arc::clone(&job.cancelled)));
            let Some((job, cancelled)) = next else {
                inner.sending_to.remove(printer);
                drop(inner);
                self.update_browsing();
                self.touched();
                return;
            };
            drop(inner);
            match self.send_one(&job, &cancelled) {
                Ok(()) => {}
                Err(failure) => self.tell_failure(&job, &failure),
            }
            for path in [document_path(job.id), record_path(job.id)]
                .into_iter()
                .flatten()
            {
                let _ = std::fs::remove_file(path);
            }
            self.lock().jobs.retain(|other| other.stored.id != job.id);
            self.touched();
        }
    }

    fn tell_failure(&self, job: &StoredJob, failure: &str) {
        let name = L10N.tr("app-name");
        let summary = L10N.tr_args("job-failed", [("title", job.title.clone().into())]);
        let sent = client::notify(
            &self.session,
            &Request {
                app_name: &name,
                replaces_id: 0,
                app_icon: "printer-error",
                summary: &summary,
                body: failure,
                actions: &[],
                urgency: Urgency::Normal,
                sound: None,
            },
        );
        if let Err(error) = sent {
            tracing::warn!("cannot tell about a failed print: {error}");
        }
    }

    fn send_one(&self, job: &StoredJob, cancelled: &AtomicBool) -> std::result::Result<(), String> {
        let path = document_path(job.id).ok_or_else(|| L10N.tr("job-lost"))?;
        if let Some(queue) = job.printer.strip_prefix(CUPS_PREFIX) {
            let document = std::fs::read(&path).map_err(|_| L10N.tr("job-lost"))?;
            return Cups::new()
                .print(queue, &job.title, &job.options, &document)
                .map_err(|error| error.to_string());
        }
        let mut wait = FIRST_RETRY;
        let mut raster_only = false;
        loop {
            if cancelled.load(Ordering::Relaxed) {
                return Ok(());
            }
            let known = self
                .known(&job.printer)
                .ok_or_else(|| L10N.tr("printer-gone"))?;
            let reached = self.reach(&known);
            let (found, uri) = match reached {
                Ok(reached) => reached,
                Err(error) if is_identity_change(&error) => {
                    return Err(L10N.tr("printer-identity-changed"));
                }
                Err(error) => {
                    tracing::info!("waiting for {}: {error}", known.name);
                    self.set_state(job.id, "waiting", L10N.tr("printer-unreachable"));
                    self.wait_a_while(wait, &job.printer, cancelled);
                    wait = (wait * 2).min(LONGEST_RETRY);
                    continue;
                }
            };
            let Some(address) = Address::parse(&uri) else {
                return Err(L10N.tr("printer-gone"));
            };
            let payload = match raster_only {
                true => found.raster_plan(&job.options).map(Payload::Raster),
                false => payload_for(&found, &job.options),
            };
            let Some(payload) = payload else {
                return Err(L10N.tr("printer-cannot-take-document"));
            };
            self.set_state(job.id, "sending", String::new());
            let request = print_request(&uri, &job.title, &job.options, &payload);
            let is_cancelled = || cancelled.load(Ordering::Relaxed);
            let sent = send_document(&address, &self.pins, request, &mut |out| match &payload {
                Payload::Pdf => io::copy(&mut File::open(&path)?, out).map(drop),
                Payload::Raster(plan) => write_document(&path, plan, &is_cancelled, out),
            });
            match sent {
                Ok(groups) => {
                    if let Some(remote) = accepted_job(&groups) {
                        return self.follow(job, &address, &uri, remote, cancelled);
                    }
                    return Ok(());
                }
                Err(Error::Refused(FORMAT_REFUSED, _)) if !raster_only => raster_only = true,
                Err(Error::Refused(BUSY, _) | Error::Io(_)) => {
                    if is_cancelled() {
                        return Ok(());
                    }
                    self.set_state(job.id, "waiting", L10N.tr("printer-busy"));
                    self.wait_a_while(wait, &job.printer, cancelled);
                    wait = (wait * 2).min(LONGEST_RETRY);
                }
                Err(error) => return Err(error.to_string()),
            }
        }
    }

    fn wait_a_while(&self, wait: Duration, printer: &str, cancelled: &AtomicBool) {
        let until = Instant::now() + wait;
        let mut inner = self.lock();
        let was_found = inner.found.contains_key(printer);
        while !cancelled.load(Ordering::Relaxed) {
            if !was_found && inner.found.contains_key(printer) {
                return;
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            let (next, _) = self
                .changed
                .wait_timeout(inner, left)
                .unwrap_or_else(PoisonError::into_inner);
            inner = next;
        }
    }

    fn follow(
        &self,
        job: &StoredJob,
        address: &Address,
        uri: &str,
        remote: i32,
        cancelled: &AtomicBool,
    ) -> std::result::Result<(), String> {
        let mut failures = 0;
        let mut warned = false;
        self.set_state(job.id, "printing", String::new());
        loop {
            if cancelled.load(Ordering::Relaxed) {
                if let Err(error) = cancel_job(address, uri, &self.pins, remote) {
                    tracing::warn!("cannot cancel on the printer: {error}");
                }
                return Ok(());
            }
            match job_state(address, uri, &self.pins, remote) {
                Ok(RemoteState::Done) => return Ok(()),
                Ok(RemoteState::Stopped(reason)) => return Err(reason),
                Ok(RemoteState::Going) => {
                    failures = 0;
                    let trouble = printer_state_reasons(address, uri, &self.pins)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|reason| {
                            reason
                                .trim_end_matches("-error")
                                .trim_end_matches("-warning")
                                .trim_end_matches("-report")
                                .to_owned()
                        })
                        .find(|reason| TROUBLES.contains(&reason.as_str()));
                    match trouble {
                        Some(trouble) => {
                            let message = L10N.tr(&format!("trouble-{trouble}"));
                            self.set_state(job.id, "stopped", message.clone());
                            if !warned {
                                warned = true;
                                self.tell_failure(job, &message);
                            }
                        }
                        None => {
                            warned = false;
                            self.set_state(job.id, "printing", String::new());
                        }
                    }
                }
                Err(_) => {
                    failures += 1;
                    if failures >= FOLLOW_FAILURES {
                        return Ok(());
                    }
                }
            }
            self.wait_a_while(FOLLOW_EVERY, &job.printer, cancelled);
        }
    }
}
