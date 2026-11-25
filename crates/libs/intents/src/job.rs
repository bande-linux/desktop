use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use bande_l10n::tr;
use zbus::blocking::Connection;
use zbus::message::Header;
use zbus::zvariant::Value;

use crate::owner::JOB_LASTS;
use crate::plan::{Job, Unit, locked};
use crate::{Dict, Reason, Refusal};

const INTERFACE: &str = "org.bande.Intents.Job";
const REPORTED_EVERY: Duration = Duration::from_millis(100);

pub(crate) const RUNNING: &str = "running";
pub(crate) const DONE: &str = "done";
pub(crate) const FAILED: &str = "failed";
pub(crate) const CANCELLED: &str = "cancelled";

pub(crate) struct Outcome {
    pub(crate) result: Dict,
    pub(crate) refusal: Option<Refusal>,
}

impl Outcome {
    pub(crate) fn refused(refusal: Refusal) -> Self {
        Self {
            result: Dict::new(),
            refusal: Some(refusal),
        }
    }
}

pub(crate) struct Running {
    connection: Connection,
    sender: String,
    path: String,
    job: OnceLock<Arc<Job>>,
    ended: OnceLock<(Instant, &'static str, Outcome)>,
    exported: AtomicBool,
    reported: Mutex<Option<Instant>>,
}

pub(crate) fn copied(dict: &Dict) -> Result<Dict, Refusal> {
    dict.iter()
        .map(|(name, value)| Ok((name.clone(), value.try_clone()?)))
        .collect::<Result<_, zbus::zvariant::Error>>()
        .map_err(|error| Refusal::failed(error.to_string()))
}

impl Running {
    pub(crate) fn new(connection: Connection, sender: String, path: String) -> Arc<Self> {
        Arc::new(Self {
            connection,
            sender,
            path,
            job: OnceLock::new(),
            ended: OnceLock::new(),
            exported: AtomicBool::new(false),
            reported: Mutex::new(None),
        })
    }

    pub(crate) fn attach(&self, job: Arc<Job>) {
        self.job.get_or_init(|| job);
    }

    pub(crate) fn sender(&self) -> &str {
        &self.sender
    }

    pub(crate) fn path(&self) -> &str {
        &self.path
    }

    pub(crate) fn is_exported(&self) -> bool {
        self.exported.load(Ordering::Acquire)
    }

    pub(crate) fn exported(&self) {
        self.exported.store(true, Ordering::Release);
    }

    pub(crate) fn cancel(&self) {
        if let Some(job) = self.job.get() {
            job.cancel();
        }
    }

    pub(crate) fn has_ended(&self) -> bool {
        self.ended.get().is_some()
    }

    pub(crate) fn is_old(&self) -> bool {
        self.ended
            .get()
            .is_some_and(|(ended, ..)| ended.elapsed() >= JOB_LASTS)
    }

    pub(crate) fn word(&self) -> &'static str {
        self.ended.get().map_or(RUNNING, |(_, word, _)| word)
    }

    pub(crate) fn result(&self) -> Result<Dict, Refusal> {
        match self.ended.get() {
            Some((_, _, outcome)) => copied(&outcome.result),
            None => Ok(Dict::new()),
        }
    }

    fn error(&self) -> (String, Dict) {
        let refusal = self
            .ended
            .get()
            .and_then(|(_, _, outcome)| outcome.refusal.as_ref());
        match refusal {
            Some(refusal) => {
                let (message, mut details) = refusal.body();
                details.insert(
                    "error".to_owned(),
                    zbus::zvariant::Str::from_static(refusal.reason.word()).into(),
                );
                (message, details)
            }
            None => (String::new(), Dict::new()),
        }
    }

    pub(crate) fn answer_at_once(&self) -> Option<Result<(String, Dict), Refusal>> {
        let (_, word, outcome) = self.ended.get()?;
        let partial = |name: &str| {
            outcome
                .result
                .get(name)
                .is_some_and(|value| match &**value {
                    Value::Str(text) => !text.is_empty(),
                    _ => true,
                })
        };
        match &outcome.refusal {
            Some(_) if partial("receipt") || partial("failed") => None,
            Some(refusal) => Some(Err(refusal.clone())),
            None => Some(copied(&outcome.result).map(|result| ((*word).to_owned(), result))),
        }
    }

    pub(crate) fn end(&self, outcome: Outcome, stopped: bool) {
        let word = match (&outcome.refusal, stopped) {
            (_, true) => CANCELLED,
            (Some(_), _) => FAILED,
            (None, _) => DONE,
        };
        self.ended.get_or_init(|| (Instant::now(), word, outcome));
        self.report(
            HashMap::from([("State", Value::from(word))]),
            &["Result", "Error", "Done", "Total"],
        );
    }

    pub(crate) fn progressed(&self) {
        let mut reported = locked(&self.reported);
        if reported.is_some_and(|at| at.elapsed() < REPORTED_EVERY) {
            return;
        }
        *reported = Some(Instant::now());
        drop(reported);
        let (done, total) = self.progress();
        self.report(
            HashMap::from([("Done", Value::from(done)), ("Total", Value::from(total))]),
            &[],
        );
    }

    fn progress(&self) -> (u64, u64) {
        self.job.get().map_or((0, 0), |job| {
            (
                job.progress.done.load(Ordering::Relaxed),
                job.progress.total.load(Ordering::Relaxed),
            )
        })
    }

    fn report(&self, changed: HashMap<&str, Value<'_>>, invalidated: &[&str]) {
        if !self.is_exported() {
            return;
        }
        let sent = self.connection.emit_signal(
            Some(self.sender.as_str()),
            self.path.as_str(),
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            &(INTERFACE, changed, invalidated),
        );
        bande_bus::warn_unsent("a job’s progress", sent);
    }
}

pub(crate) struct JobObject(pub(crate) Arc<Running>);

impl JobObject {
    fn allow(&self, header: Option<&Header<'_>>) -> zbus::fdo::Result<()> {
        let sender = header.and_then(Header::sender);
        match sender.is_none_or(|sender| sender.as_str() == self.0.sender) {
            true => Ok(()),
            false => Err(zbus::fdo::Error::AccessDenied(tr!(
                "This was started by another program"
            ))),
        }
    }
}

#[zbus::interface(name = "org.bande.Intents.Job")]
impl JobObject {
    #[zbus(property)]
    fn state(&self, #[zbus(header)] header: Option<Header<'_>>) -> zbus::fdo::Result<String> {
        self.allow(header.as_ref())?;
        Ok(self.0.word().to_owned())
    }

    #[zbus(property)]
    fn unit(&self, #[zbus(header)] header: Option<Header<'_>>) -> zbus::fdo::Result<String> {
        self.allow(header.as_ref())?;
        let unit = self.0.job.get().and_then(|job| *locked(&job.progress.unit));
        Ok(unit.unwrap_or(Unit::Things).word().to_owned())
    }

    #[zbus(property)]
    fn done(&self, #[zbus(header)] header: Option<Header<'_>>) -> zbus::fdo::Result<u64> {
        self.allow(header.as_ref())?;
        Ok(self.0.progress().0)
    }

    #[zbus(property)]
    fn total(&self, #[zbus(header)] header: Option<Header<'_>>) -> zbus::fdo::Result<u64> {
        self.allow(header.as_ref())?;
        Ok(self.0.progress().1)
    }

    #[zbus(property)]
    fn result(&self, #[zbus(header)] header: Option<Header<'_>>) -> zbus::fdo::Result<Dict> {
        self.allow(header.as_ref())?;
        self.0
            .result()
            .map_err(|refusal| zbus::fdo::Error::Failed(refusal.message))
    }

    #[zbus(property)]
    fn error(
        &self,
        #[zbus(header)] header: Option<Header<'_>>,
    ) -> zbus::fdo::Result<(String, Dict)> {
        self.allow(header.as_ref())?;
        Ok(self.0.error())
    }

    fn cancel(&self, #[zbus(header)] header: Header<'_>) -> Result<(), Refusal> {
        self.allow(Some(&header))
            .map_err(|error| Refusal::new(Reason::AccessDenied, error.to_string()))?;
        self.0.cancel();
        Ok(())
    }
}
