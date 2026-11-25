use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use zbus::zvariant::OwnedValue;

use crate::{
    Dict, Effect, Intent, Kind, Limits, Ref, Referent, Refusal, Returns, Thing, Value, dict_to_json,
};

pub(crate) const FAILURES: usize = 100;

/// Lists the things that match, in order, at most 10,000 and one more when there were more: the
/// rest is cut. The owner's library pages them, makes the set, describes them with [`Describe`]
/// and answers `current`.
pub trait Find<O>: Intent {
    type Thing: Thing;

    fn find(self, owner: &O, limits: &Limits) -> Result<Found<Self::Thing>, Refusal>;
}

/// What a [`Find`] found.
pub struct Found<T> {
    pub references: Vec<Ref<T>>,
    /// Whether text someone else could have chosen was matched.
    pub matched_foreign: bool,
}

impl<T> Found<T> {
    pub fn new(references: Vec<Ref<T>>) -> Self {
        Self {
            references,
            matched_foreign: false,
        }
    }
}

/// Gives the properties of one thing, for `find` and `get`; `NotFound` when it does not exist or
/// lies outside the limits.
pub trait Describe<T: Thing> {
    fn describe(&self, thing: &Ref<T>, wanted: &Wanted, limits: &Limits) -> Result<T, Refusal>;
}

/// The properties a caller gets; others need not be read.
#[derive(Debug, Clone)]
pub struct Wanted {
    pub(crate) names: Vec<String>,
}

impl Wanted {
    pub fn of(properties: &[&str]) -> Self {
        Self {
            names: properties.iter().map(|name| (*name).to_owned()).collect(),
        }
    }

    pub fn has(&self, property: &str) -> bool {
        self.names.iter().any(|name| name == property)
    }
}

/// Prepares and performs an intent that is not `find` or `get` on the owner `O`.
pub trait Perform<O>: Intent {
    /// What Prepare froze for Perform.
    type Prepared: Send + 'static;
    /// `()`, a [`Thing`], a list of them, or [`Computed`].
    type Output: Output;

    /// Checks everything that can be checked and changes nothing.
    fn prepare(self, owner: &O, limits: &Limits) -> Result<Plan<Self::Prepared>, Refusal>;

    /// Does it. Before each change, [`Job::keep`] stores what takes it back; `Conflict` when the
    /// state is no longer what the plan was made for.
    fn perform(
        prepared: Self::Prepared,
        owner: &O,
        job: &Job,
    ) -> Result<Done<Self::Output>, Refusal>;
}

/// A prepared intent: what it will do, to perform once.
pub struct Plan<P> {
    /// The effects these arguments cause.
    pub effects: Vec<Effect>,
    /// One sentence, kept in Activity.
    pub summary: String,
    /// Everything it will do, which is all the approval prompt knows.
    pub rows: Vec<Row>,
    /// The things it acts on.
    pub references: Vec<String>,
    pub prepared: P,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Text,
    RichText,
    Address,
    Path,
    Size,
    Count,
    Thing,
    Loss,
    Copy,
}

impl RowKind {
    fn word(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::RichText => "rich_text",
            Self::Address => "address",
            Self::Path => "path",
            Self::Size => "size",
            Self::Count => "count",
            Self::Thing => "thing",
            Self::Loss => "loss",
            Self::Copy => "copy",
        }
    }
}

/// One line of a plan's description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    pub kind: RowKind,
    pub value: String,
}

impl Row {
    pub fn new(kind: RowKind, label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            kind,
            value: value.into(),
        }
    }

    pub(crate) fn wire(&self) -> Dict {
        [
            ("label", self.label.as_str()),
            ("kind", self.kind.word()),
            ("value", self.value.as_str()),
        ]
        .into_iter()
        .map(|(name, text)| (name.to_owned(), owned_text(text)))
        .collect()
    }
}

pub(crate) fn owned_text(text: &str) -> OwnedValue {
    zbus::zvariant::Str::from(text.to_owned()).into()
}

/// What a performed plan gives.
pub struct Done<T> {
    pub output: T,
    /// Takes it all back; `None` when it cannot be undone.
    pub undo: Option<Undo>,
}

impl<T> Done<T> {
    pub fn new(output: T) -> Self {
        Self { output, undo: None }
    }

    pub fn undone_by(output: T, undo: Undo) -> Self {
        Self {
            output,
            undo: Some(undo),
        }
    }
}

/// The intent that takes a change back, its arguments, and the properties that must still look
/// as the change left them.
#[derive(Debug, Clone, PartialEq)]
pub struct Undo {
    pub(crate) intent: String,
    pub(crate) arguments: serde_json::Value,
    pub(crate) touched: Vec<(String, Vec<String>)>,
}

impl Undo {
    /// `intent` is public or registered with [`crate::Domain::private`].
    pub fn by<I: Intent>(intent: I) -> Result<Self, Refusal> {
        Ok(Self {
            intent: I::ID.to_owned(),
            arguments: dict_to_json(&intent.encode()?),
            touched: Vec::new(),
        })
    }

    /// The change touched these properties of `thing`; undo is refused when [`Store::look`] then
    /// tells them differently.
    pub fn touching<T: Referent>(mut self, thing: &Ref<T>, properties: &[&str]) -> Self {
        self.touched.push((
            thing.as_str().to_owned(),
            properties.iter().map(|name| (*name).to_owned()).collect(),
        ));
        self
    }

    /// The arguments of this undo, when `I` is its intent.
    pub fn arguments<I: Intent>(&self) -> Option<I> {
        if self.intent != I::ID {
            return None;
        }
        let arguments = I::definition()
            .arguments_from_json(I::ID, self.arguments.as_object()?)
            .ok()?;
        I::decode(arguments).ok()
    }
}

/// What an owner keeps the things of its domains in.
pub trait Store: Send + Sync + 'static {
    /// The named properties of a thing as text that is the same as long as they mean the same.
    fn look(&self, reference: &str, properties: &[String]) -> Result<String, Refusal>;

    /// What is left to undo of a change that a crash or a failure cut short, after the owner has
    /// finished it or taken back what was half done: all of it, unless the store knows better.
    fn recover(&self, undo: Undo) -> Result<Option<Undo>, Refusal> {
        Ok(Some(undo))
    }
}

/// What performing gives: things, nothing or computed values.
pub trait Output: Send + 'static {
    fn returns() -> Option<Returns>;

    fn into_result(self) -> Result<Dict, Refusal>;
}

impl Output for () {
    fn returns() -> Option<Returns> {
        None
    }

    fn into_result(self) -> Result<Dict, Refusal> {
        Ok(Dict::new())
    }
}

impl<T: Thing> Output for Vec<T> {
    fn returns() -> Option<Returns> {
        Some(Returns::List(T::TYPE.to_owned()))
    }

    fn into_result(self) -> Result<Dict, Refusal> {
        let things = self
            .into_iter()
            .map(|thing| thing_dict(thing, |_| true))
            .collect::<Result<Vec<_>, _>>()?;
        entry("things", things)
    }
}

impl<T: Thing> Output for T {
    fn returns() -> Option<Returns> {
        Vec::<T>::returns()
    }

    fn into_result(self) -> Result<Dict, Refusal> {
        vec![self].into_result()
    }
}

/// The result of a `compute`: one value that is no stored thing.
pub struct Computed<T>(pub T);

impl<T: Value + Send + 'static> Output for Computed<T> {
    fn returns() -> Option<Returns> {
        Some(Returns::Value(T::kind()))
    }

    fn into_result(self) -> Result<Dict, Refusal> {
        let mut result = Dict::new();
        crate::give(&mut result, "value", self.0)?;
        Ok(result)
    }
}

/// The result of a `compute` that gives several records.
pub struct ComputedList<T>(pub Vec<T>);

/// Named values that are no thing; derived with `#[derive(Record)]`.
pub trait Record: Send + 'static {
    fn kinds() -> BTreeMap<String, Kind>;

    fn into_values(self) -> Result<Dict, Refusal>;
}

impl<T: Record> Output for ComputedList<T> {
    fn returns() -> Option<Returns> {
        Some(Returns::Values(T::kinds()))
    }

    fn into_result(self) -> Result<Dict, Refusal> {
        let values = self
            .0
            .into_iter()
            .map(Record::into_values)
            .collect::<Result<Vec<_>, _>>()?;
        entry("value", values)
    }
}

fn entry(name: &str, dicts: Vec<Dict>) -> Result<Dict, Refusal> {
    let mut result = Dict::new();
    put(&mut result, name, dicts)?;
    Ok(result)
}

pub(crate) fn put<'a>(
    dict: &mut Dict,
    name: &str,
    value: impl Into<zbus::zvariant::Value<'a>>,
) -> Result<(), Refusal> {
    let value = value
        .into()
        .try_to_owned()
        .map_err(|error| Refusal::failed(error.to_string()))?;
    dict.insert(name.to_owned(), value);
    Ok(())
}

/// `thing` as it travels: its reference, type, the properties `kept` keeps, which of them are
/// foreign, and which were cut.
pub(crate) fn thing_dict<T: Thing>(thing: T, kept: impl Fn(&str) -> bool) -> Result<Dict, Refusal> {
    let reference = thing.reference().to_owned();
    let mut properties = thing.properties()?;
    properties.retain(|name, _| kept(name));
    let definition = T::definition();
    let mut foreign: Vec<String> = properties
        .keys()
        .filter(|name| {
            definition
                .properties
                .get(*name)
                .is_some_and(|property| property.foreign)
        })
        .cloned()
        .collect();
    foreign.sort();
    let mut cut = Vec::new();
    for (name, value) in &mut properties {
        if let zbus::zvariant::Value::Str(text) = &**value
            && text.len() > crate::SHOWN_TEXT
        {
            let end = text.floor_char_boundary(crate::SHOWN_TEXT);
            let shortened = owned_text(&text[..end]);
            *value = shortened;
            cut.push(name.clone());
        }
    }
    put(&mut properties, "reference", reference)?;
    put(&mut properties, "type", T::TYPE)?;
    put(&mut properties, "foreign", foreign)?;
    if !cut.is_empty() {
        put(&mut properties, "cut", cut)?;
    }
    Ok(properties)
}

/// What a job counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Bytes,
    Things,
}

impl Unit {
    pub(crate) fn word(self) -> &'static str {
        match self {
            Self::Bytes => "bytes",
            Self::Things => "things",
        }
    }
}

type Keep = dyn Fn(Undo) -> Result<(), Refusal> + Send + Sync;

/// A running plan: what it has done, what failed, and whether it was cancelled.
pub struct Job {
    agent: Option<String>,
    keep: Box<Keep>,
    changed: Box<dyn Fn() + Send + Sync>,
    pub(crate) progress: Progress,
}

#[derive(Default)]
pub(crate) struct Progress {
    pub(crate) unit: Mutex<Option<Unit>>,
    pub(crate) done: AtomicU64,
    pub(crate) total: AtomicU64,
    pub(crate) failed: Mutex<Vec<(String, Refusal)>>,
    cancel: AtomicBool,
    stopped: AtomicBool,
}

pub(crate) fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Job {
    pub(crate) fn new(
        agent: Option<String>,
        keep: impl Fn(Undo) -> Result<(), Refusal> + Send + Sync + 'static,
        changed: impl Fn() + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            agent,
            keep: Box::new(keep),
            changed: Box::new(changed),
            progress: Progress::default(),
        })
    }

    /// The agent it is done for.
    pub fn agent(&self) -> Option<&str> {
        self.agent.as_deref()
    }

    /// Stores what takes back everything up to and including the step about to start, so that a
    /// crash from here on leaves something to undo. An error is returned from Perform at once,
    /// since nothing may be changed that could not be taken back. Each call writes the whole undo to
    /// disk: with many things, an owner whose [`Store::recover`] can tell which were changed keeps
    /// the undo of all of them once.
    pub fn keep(&self, undo: Undo) -> Result<(), Refusal> {
        (self.keep)(undo)
    }

    /// How much there is to do.
    pub fn expect(&self, unit: Unit, total: u64) {
        *locked(&self.progress.unit) = Some(unit);
        self.progress.total.store(total, Ordering::Relaxed);
        (self.changed)();
    }

    /// Adds to what is done.
    pub fn advance(&self, by: u64) {
        self.progress.done.fetch_add(by, Ordering::Relaxed);
        (self.changed)();
    }

    /// Leaves out one of many things; the job goes on.
    pub fn failed(&self, reference: &str, refusal: Refusal) {
        let mut failed = locked(&self.progress.failed);
        if failed.len() < FAILURES {
            failed.push((reference.to_owned(), refusal));
        }
    }

    /// Whether to stop before the next thing. The plan then ends as cancelled, with what was done.
    pub fn cancelled(&self) -> bool {
        let cancelled = self.progress.cancel.load(Ordering::Relaxed);
        if cancelled {
            self.progress.stopped.store(true, Ordering::Relaxed);
        }
        cancelled
    }

    pub(crate) fn cancel(&self) {
        self.progress.cancel.store(true, Ordering::Relaxed);
    }

    pub(crate) fn was_stopped(&self) -> bool {
        self.progress.stopped.load(Ordering::Relaxed)
    }

    pub(crate) fn result(&self, output: Dict) -> Result<Dict, Refusal> {
        let mut result = output;
        let failed = locked(&self.progress.failed)
            .iter()
            .map(|(reference, refusal)| {
                let mut entry = refusal.details();
                put(&mut entry, "reference", reference.as_str())?;
                put(&mut entry, "error", refusal.reason.word())?;
                put(&mut entry, "message", refusal.message.as_str())?;
                Ok(entry)
            })
            .collect::<Result<Vec<_>, Refusal>>()?;
        if !failed.is_empty() {
            put(&mut result, "failed", failed)?;
        }
        put(
            &mut result,
            "done",
            self.progress.done.load(Ordering::Relaxed),
        )?;
        Ok(result)
    }
}
