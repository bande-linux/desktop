use std::os::fd::BorrowedFd;

use futures_lite::{StreamExt, stream};
use serde_json::Value as Json;
use zbus::blocking::Connection;
use zbus::proxy::CacheProperties;
use zbus::zvariant::{Fd, OwnedFd, OwnedObjectPath};

use crate::job::{CANCELLED, DONE, FAILED};
use crate::{
    Access, Dict, Effect, Limits, Manifests, Opened, PATH, Refusal, Written, dict_to_json,
};

#[zbus::proxy(interface = "org.bande.Intents", default_path = "/org/bande/Intents")]
trait Intents {
    #[allow(clippy::type_complexity)]
    fn find(
        &self,
        intent: &str,
        arguments: &Dict,
        limits: &Dict,
        cursor: &str,
        count: u32,
    ) -> zbus::Result<(Vec<Dict>, String, String, bool, Vec<String>)>;

    fn get(
        &self,
        intent: &str,
        references: &[String],
        properties: &[String],
        arguments: &Dict,
        limits: &Dict,
    ) -> zbus::Result<(Vec<Dict>, Vec<String>)>;

    fn prepare(
        &self,
        intent: &str,
        arguments: &Dict,
        limits: &Dict,
    ) -> zbus::Result<(String, Dict)>;

    fn receipts(&self, limits: &Dict, count: u32) -> zbus::Result<Vec<Dict>>;

    fn prepare_undo(&self, receipt: &str, limits: &Dict) -> zbus::Result<(String, Dict)>;
}

#[zbus::proxy(
    interface = "org.bande.Intents.Files",
    default_path = "/org/bande/Intents"
)]
trait Files {
    #[allow(clippy::type_complexity)]
    fn open(
        &self,
        references: &[String],
        access: &str,
        limits: &Dict,
    ) -> zbus::Result<(Vec<(OwnedFd, String, Dict)>, Vec<String>)>;

    fn register(
        &self,
        folder: Fd<'_>,
        files: &[(&str, &str, &str)],
        limits: &Dict,
    ) -> zbus::Result<Vec<String>>;
}

#[zbus::proxy(interface = "org.bande.Intents.Job")]
trait Job {
    #[zbus(property)]
    fn state(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn unit(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn done(&self) -> zbus::Result<u64>;
    #[zbus(property)]
    fn total(&self) -> zbus::Result<u64>;
    #[zbus(property)]
    fn result(&self) -> zbus::Result<Dict>;
    #[zbus(property)]
    fn error(&self) -> zbus::Result<(String, Dict)>;
    fn cancel(&self) -> zbus::Result<()>;
}

/// Calls the owners of intents on one connection, to which the plans and sets belong.
pub struct Client {
    connection: Connection,
    limits: Dict,
}

/// What `find` gave: one page, and the set of everything that was found.
#[derive(Debug, Default)]
pub struct Page {
    pub things: Vec<Dict>,
    /// Gives the next page; empty after the last.
    pub cursor: String,
    pub set: String,
    /// More was found than a set holds.
    pub cut: bool,
}

/// A plan to perform once, within ten minutes, on the same connection.
#[derive(Debug)]
pub struct Prepared {
    pub plan: String,
    pub effects: Vec<Effect>,
    pub summary: String,
    /// What it will do, for a person.
    pub rows: Vec<Dict>,
    bus: String,
}

/// How a plan ended.
#[derive(Debug)]
pub struct Performed {
    /// Whether it was cancelled before its end.
    pub cancelled: bool,
    pub result: Dict,
}

/// The record of a performed plan that can be undone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub receipt: String,
    pub intent: String,
    pub summary: String,
    pub references: Vec<String>,
    /// Seconds since 1970.
    pub made: i64,
    /// The agent it was done for, or empty.
    pub agent: String,
}

/// The text `name` of `dict`, or nothing.
pub fn text<'a>(dict: &'a Dict, name: &str) -> &'a str {
    match dict.get(name).map(|value| &**value) {
        Some(zbus::zvariant::Value::Str(text)) => text.as_str(),
        _ => "",
    }
}

fn taken<T: TryFrom<zbus::zvariant::OwnedValue> + Default>(dict: &mut Dict, name: &str) -> T {
    dict.remove(name)
        .and_then(|value| value.try_into().ok())
        .unwrap_or_default()
}

fn effects(words: &[String]) -> Vec<Effect> {
    words
        .iter()
        .filter_map(|word| Effect::from_word(word))
        .collect()
}

impl Performed {
    /// The receipt that undoes it, if there is one.
    pub fn receipt(&self) -> Option<&str> {
        Some(text(&self.result, "receipt")).filter(|receipt| !receipt.is_empty())
    }

    /// The things it made, changed or found.
    pub fn things(&self) -> Vec<Dict> {
        self.result
            .get("things")
            .and_then(|things| things.try_clone().ok()?.try_into().ok())
            .unwrap_or_default()
    }

    /// The result as JSON.
    pub fn json(&self) -> Json {
        dict_to_json(&self.result)
    }
}

impl Client {
    pub fn new(connection: &Connection) -> Self {
        Self {
            connection: connection.clone(),
            limits: Dict::new(),
        }
    }

    /// Passes these limits with every call.
    pub fn limited(mut self, limits: Dict) -> Self {
        self.limits = limits;
        self
    }

    /// Calls other owners for a caller with these limits.
    pub fn passing(connection: &Connection, limits: &Limits) -> Result<Self, Refusal> {
        Ok(Self::new(connection).limited(limits.encode()?))
    }

    fn owner(&self, bus: &str) -> Result<IntentsProxyBlocking<'static>, Refusal> {
        Ok(IntentsProxyBlocking::builder(&self.connection)
            .destination(bus.to_owned())
            .and_then(|builder| builder.path(PATH))?
            .cache_properties(CacheProperties::No)
            .build()?)
    }

    /// One page of what the `find` intent of the owner on `bus` finds; an empty cursor starts.
    pub fn find(
        &self,
        bus: &str,
        intent: &str,
        arguments: &Dict,
        cursor: &str,
        count: u32,
    ) -> Result<Page, Refusal> {
        let (things, cursor, set, cut, _) =
            self.owner(bus)?
                .find(intent, arguments, &self.limits, cursor, count)?;
        Ok(Page {
            things,
            cursor,
            set,
            cut,
        })
    }

    pub fn get(
        &self,
        bus: &str,
        intent: &str,
        references: &[String],
        properties: &[String],
        arguments: &Dict,
    ) -> Result<Vec<Dict>, Refusal> {
        let (things, _) =
            self.owner(bus)?
                .get(intent, references, properties, arguments, &self.limits)?;
        Ok(things)
    }

    fn files(
        &self,
        manifests: &Manifests,
        kind: &str,
    ) -> Result<FilesProxyBlocking<'static>, Refusal> {
        let manifest = manifests
            .thing(kind)
            .ok_or_else(|| Refusal::unavailable(bande_l10n::tr!("Files cannot be opened now")))?;
        Ok(FilesProxyBlocking::builder(&self.connection)
            .destination(manifest.bus.clone())?
            .cache_properties(CacheProperties::No)
            .build()?)
    }

    /// Opens things of one type, such as files or attachments, through their owner, and gives the
    /// effects that has.
    pub fn open_files(
        &self,
        manifests: &Manifests,
        references: &[String],
        access: Access,
    ) -> Result<(Vec<Opened>, Vec<Effect>), Refusal> {
        let Some(kind) = references.first().and_then(|it| it.split(':').next()) else {
            return Ok((Vec::new(), Vec::new()));
        };
        let (files, words) =
            self.files(manifests, kind)?
                .open(references, access.word(), &self.limits)?;
        let opened = files
            .into_iter()
            .map(|(file, reference, mut about)| Opened {
                file: file.into(),
                reference,
                name: taken(&mut about, "name"),
                media_type: taken(&mut about, "media_type"),
                size: taken(&mut about, "size"),
            })
            .collect();
        Ok((opened, effects(&words)))
    }

    /// Has the files owner check and name what was written in `folder`, and gives the references.
    pub fn register_files(
        &self,
        manifests: &Manifests,
        folder: BorrowedFd<'_>,
        written: &[Written],
    ) -> Result<Vec<String>, Refusal> {
        let files: Vec<(&str, &str, &str)> = written
            .iter()
            .map(|file| {
                (
                    file.temporary.as_str(),
                    file.name.as_str(),
                    file.replaces.as_deref().unwrap_or_default(),
                )
            })
            .collect();
        Ok(self
            .files(manifests, "files.file")?
            .register(folder.into(), &files, &self.limits)?)
    }

    pub fn prepare(&self, bus: &str, intent: &str, arguments: &Dict) -> Result<Prepared, Refusal> {
        let (plan, description) = self.owner(bus)?.prepare(intent, arguments, &self.limits)?;
        Ok(Self::prepared(bus, plan, description))
    }

    /// Prepares what takes back the change of `receipt`.
    pub fn prepare_undo(&self, bus: &str, receipt: &str) -> Result<Prepared, Refusal> {
        let (plan, description) = self.owner(bus)?.prepare_undo(receipt, &self.limits)?;
        Ok(Self::prepared(bus, plan, description))
    }

    fn prepared(bus: &str, plan: String, mut description: Dict) -> Prepared {
        let words: Vec<String> = taken(&mut description, "effects");
        Prepared {
            plan,
            effects: effects(&words),
            summary: taken(&mut description, "summary"),
            rows: taken(&mut description, "rows"),
            bus: bus.to_owned(),
        }
    }

    /// Performs a plan and waits for its end.
    pub fn perform(&self, prepared: Prepared) -> Result<Performed, Refusal> {
        let reply = self
            .owner(&prepared.bus)?
            .inner()
            .call_method("Perform", &(&prepared.plan,))?;
        let (job, state, result): (OwnedObjectPath, String, Dict) = reply.body().deserialize()?;
        let header = reply.header();
        let performer = header
            .sender()
            .map_or(prepared.bus.as_str(), |sender| sender.as_str());
        match job.as_str() {
            "/" => Ok(Performed {
                cancelled: state == CANCELLED,
                result,
            }),
            _ => zbus::block_on(self.wait(performer, &job)),
        }
    }

    async fn wait(&self, performer: &str, path: &OwnedObjectPath) -> Result<Performed, Refusal> {
        let job = JobProxy::builder(self.connection.inner())
            .destination(performer.to_owned())?
            .path(path.clone())?
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        let changes = zbus::fdo::PropertiesProxy::builder(self.connection.inner())
            .destination(performer.to_owned())?
            .path(path.clone())?
            .build()
            .await?
            .receive_properties_changed()
            .await?
            .map(|_| true);
        let owners = job
            .inner()
            .receive_owner_changed()
            .await?
            .map(|owner| owner.is_some());
        let mut events = stream::or(changes, owners);
        loop {
            let state = job.state().await.map_err(|_| Refusal::interrupted())?;
            match state.as_str() {
                DONE | CANCELLED => {
                    return Ok(Performed {
                        cancelled: state == CANCELLED,
                        result: job.result().await?,
                    });
                }
                FAILED => {
                    let (message, mut details) = job.error().await?;
                    let word: String = taken(&mut details, "error");
                    return Err(Refusal::from_parts(&word, message, details));
                }
                _ => {}
            }
            if events.next().await != Some(true) {
                return Err(Refusal::interrupted());
            }
        }
    }

    /// What can be undone in the domains of the owner on `bus`, the newest first.
    pub fn receipts(&self, bus: &str, count: u32) -> Result<Vec<Receipt>, Refusal> {
        Ok(self
            .owner(bus)?
            .receipts(&self.limits, count)?
            .into_iter()
            .map(|mut receipt| Receipt {
                receipt: taken(&mut receipt, "receipt"),
                intent: taken(&mut receipt, "intent"),
                summary: taken(&mut receipt, "summary"),
                references: taken(&mut receipt, "references"),
                made: taken(&mut receipt, "made"),
                agent: taken(&mut receipt, "agent"),
            })
            .collect())
    }

    /// Undoes `receipt`, with its owner found in `manifests`.
    pub fn undo(&self, manifests: &Manifests, receipt: &str) -> Result<String, Refusal> {
        let owner = manifests
            .serving(receipt)
            .ok_or_else(|| Refusal::not_found(bande_l10n::tr!("There is nothing to undo")))?;
        let prepared = self.prepare_undo(&owner.bus, receipt)?;
        let summary = prepared.summary.clone();
        self.perform(prepared)?;
        Ok(summary)
    }

    /// What can be undone in every domain, the newest first; owners that do not answer are left
    /// out and logged.
    pub fn all_receipts(&self, manifests: &Manifests) -> Vec<Receipt> {
        let mut all: Vec<Receipt> = manifests
            .iter()
            .filter(|manifest| manifest.intents.values().any(|intent| intent.undo))
            .filter_map(|manifest| {
                self.receipts(&manifest.bus, u32::MAX)
                    .inspect_err(|refusal| {
                        tracing::warn!("{} does not tell its receipts: {refusal}", manifest.bus);
                    })
                    .ok()
            })
            .flatten()
            .collect();
        all.sort_by_key(|receipt| std::cmp::Reverse(receipt.made));
        all
    }
}
