use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bande_l10n::tr;
use zbus::zvariant::OwnedValue;

use crate::client::text;
use crate::{
    Dict, Effect, Intent, Kind, Limits, Ref, Referent, Refusal, Returns, Thing, Value,
    dict_to_json, domain,
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
#[derive(Debug, Default)]
pub struct Wanted {
    pub(crate) names: Vec<String>,
    pub(crate) asked: Option<(String, Dict)>,
}

impl Wanted {
    pub fn of(properties: &[&str]) -> Self {
        Self {
            names: properties.iter().map(|name| (*name).to_owned()).collect(),
            asked: None,
        }
    }

    /// The `get` that asks, when it is `I` and was given arguments besides the things.
    pub fn arguments<I: Intent>(&self) -> Result<Option<I>, Refusal> {
        match &self.asked {
            Some((id, arguments)) if id == I::ID => {
                I::decode(crate::job::copied(arguments)?).map(Some)
            }
            _ => Ok(None),
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
    /// `()`, a [`Thing`], a list of them, references to another program's things, [`Computed`] or
    /// [`ComputedList`].
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
    const ALL: [(Self, &'static str); 9] = [
        (Self::Text, "text"),
        (Self::RichText, "rich_text"),
        (Self::Address, "address"),
        (Self::Path, "path"),
        (Self::Size, "size"),
        (Self::Count, "count"),
        (Self::Thing, "thing"),
        (Self::Loss, "loss"),
        (Self::Copy, "copy"),
    ];

    fn word(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(kind, _)| *kind == self)
            .map_or("", |(_, word)| word)
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

    pub(crate) fn from_wire(row: &Dict) -> Self {
        let word = text(row, "kind");
        let kind = RowKind::ALL
            .iter()
            .find(|(_, known)| *known == word)
            .map_or(RowKind::Text, |(kind, _)| *kind);
        Self::new(kind, text(row, "label"), text(row, "value"))
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

    /// An undo by an intent another program serves, such as `files.trash` of what was made in a
    /// folder. That program prepares it with the limits of whoever undoes, and what it does has a
    /// receipt of its own there.
    pub fn elsewhere(intent: &str, arguments: &Dict) -> Self {
        Self {
            intent: intent.to_owned(),
            arguments: dict_to_json(arguments),
            touched: Vec::new(),
        }
    }

    pub(crate) fn then(&mut self, step: &Self) -> Result<(), Refusal> {
        let same = || Refusal::failed("the steps of a job are undone by one intent");
        if self.intent != step.intent {
            return Err(same());
        }
        let all = self.arguments.as_object_mut().ok_or_else(same)?;
        for (name, value) in step.arguments.as_object().ok_or_else(same)? {
            match (all.get_mut(name), value) {
                (Some(serde_json::Value::Array(all)), serde_json::Value::Array(more)) => {
                    all.extend(more.iter().cloned());
                }
                (Some(known), value) if known == value => {}
                _ => return Err(same()),
            }
        }
        self.touched.extend(step.touched.iter().cloned());
        Ok(())
    }

    pub(crate) fn of_steps(steps: &[Self]) -> Result<Option<Self>, Refusal> {
        let Some((first, rest)) = steps.split_first() else {
            return Ok(None);
        };
        let mut all = first.clone();
        rest.iter().try_for_each(|step| all.then(step))?;
        Ok(Some(all))
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
    /// The named properties of a thing as text that is the same as long as they mean the same; also
    /// of another program's thing, when an undo touches one.
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

    #[doc(hidden)]
    fn into_result(self, limits: &Limits) -> Result<Dict, Refusal>;
}

impl Output for () {
    fn returns() -> Option<Returns> {
        None
    }

    fn into_result(self, _limits: &Limits) -> Result<Dict, Refusal> {
        Ok(Dict::new())
    }
}

impl<T: Thing> Output for Vec<T> {
    fn returns() -> Option<Returns> {
        Some(Returns::List(T::TYPE.to_owned()))
    }

    fn into_result(self, limits: &Limits) -> Result<Dict, Refusal> {
        let domain = domain(T::TYPE);
        let shown = limits.allows(domain, Effect::Read);
        let foreign = limits.allows(domain, Effect::ReadForeign);
        let things = self
            .into_iter()
            .map(|thing| Ok(thing_dict(thing, |_| shown, foreign)?.0))
            .collect::<Result<Vec<_>, Refusal>>()?;
        entry("things", things)
    }
}

impl<T: Referent> Output for Vec<Ref<T>> {
    fn returns() -> Option<Returns> {
        T::TYPES
            .first()
            .map(|kind| Returns::List((*kind).to_owned()))
    }

    fn into_result(self, _limits: &Limits) -> Result<Dict, Refusal> {
        let things = self
            .into_iter()
            .map(|reference| {
                let mut thing = Dict::new();
                put(&mut thing, "type", reference.thing_type())?;
                put(&mut thing, "reference", reference.as_str())?;
                Ok(thing)
            })
            .collect::<Result<Vec<_>, Refusal>>()?;
        entry("things", things)
    }
}

impl<T: Thing> Output for T {
    fn returns() -> Option<Returns> {
        Vec::<T>::returns()
    }

    fn into_result(self, limits: &Limits) -> Result<Dict, Refusal> {
        vec![self].into_result(limits)
    }
}

/// The result of a `compute`: one value that is no stored thing.
pub struct Computed<T>(pub T);

impl<T: Value + Send + 'static> Output for Computed<T> {
    fn returns() -> Option<Returns> {
        Some(Returns::Value(T::kind()))
    }

    fn into_result(self, _limits: &Limits) -> Result<Dict, Refusal> {
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

    fn into_result(self, _limits: &Limits) -> Result<Dict, Refusal> {
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

pub(crate) fn thing_dict<T: Thing>(
    thing: T,
    kept: impl Fn(&str) -> bool,
    foreign_allowed: bool,
) -> Result<(Dict, bool), Refusal> {
    let reference = thing.reference().to_owned();
    let also_foreign = thing.also_foreign();
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
                || also_foreign.contains(&name.as_str())
        })
        .cloned()
        .collect();
    let withheld = !foreign_allowed && !foreign.is_empty();
    if withheld {
        properties.retain(|name, _| !foreign.contains(name));
        foreign.clear();
    }
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
    Ok((properties, withheld))
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

type Keep = dyn Fn(&[Undo], Step) -> Result<(), Refusal> + Send + Sync;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    Kept,
    Left,
}

/// A running plan: what it has done, what failed, and whether it was cancelled.
pub struct Job {
    agent: Option<String>,
    effects: Vec<Effect>,
    keep: Box<Keep>,
    steps: Mutex<Vec<Undo>>,
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
        effects: Vec<Effect>,
        keep: impl Fn(&[Undo], Step) -> Result<(), Refusal> + Send + Sync + 'static,
        changed: impl Fn() + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            agent,
            effects,
            keep: Box::new(keep),
            steps: Mutex::default(),
            changed: Box::new(changed),
            progress: Progress::default(),
        })
    }

    /// The agent it is done for.
    pub fn agent(&self) -> Option<&str> {
        self.agent.as_deref()
    }

    /// Stores what takes back the step about to start, so that a crash from here on leaves something
    /// to undo. An error is returned from Perform at once, since nothing may be changed that could
    /// not be taken back. With many things there are many steps, of one or more things each: they
    /// are undone by one intent, whose lists are those of the steps after one another and whose other
    /// arguments are the same in each. A step is added to what is on disk, which costs one write.
    pub fn keep(&self, step: Undo) -> Result<(), Refusal> {
        let mut steps = locked(&self.steps);
        steps.push(step);
        (self.keep)(&steps, Step::Kept).inspect_err(|_| {
            steps.pop();
        })
    }

    /// The step kept last was not made after all, so it is left out of the undo.
    pub fn not_done(&self) -> Result<(), Refusal> {
        let mut steps = locked(&self.steps);
        match steps.pop() {
            Some(_) => (self.keep)(&steps, Step::Left),
            None => Ok(()),
        }
    }

    /// What takes back every step that was kept, for [`Done`].
    pub fn undo(&self) -> Result<Option<Undo>, Refusal> {
        Undo::of_steps(&locked(&self.steps))
    }

    /// Looks at what a step turned out to cause, before it starts: `Conflict` when the plan did not
    /// have one of `effects`.
    pub fn still(&self, effects: &[Effect]) -> Result<(), Refusal> {
        match effects.iter().all(|effect| self.effects.contains(effect)) {
            true => Ok(()),
            false => Err(Refusal::conflict(tr!(
                "This would now do more than what was prepared"
            ))),
        }
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

    pub(crate) fn was_partial(&self) -> bool {
        self.was_stopped() || !locked(&self.progress.failed).is_empty()
    }

    pub(crate) fn result(
        &self,
        output: Dict,
        within: impl Fn(Refusal) -> Refusal,
    ) -> Result<Dict, Refusal> {
        let mut result = output;
        let failed = locked(&self.progress.failed)
            .iter()
            .map(|(reference, refusal)| {
                let refusal = within(refusal.clone());
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
