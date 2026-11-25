use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::{Duration, Instant};

use bande_l10n::tr;
use futures_lite::StreamExt;
use zbus::blocking::Connection;
use zbus::message::Header;
use zbus::zvariant::{self, OwnedObjectPath};

use crate::callers::Callers;
use crate::domain::{Connect, Entry, FilesOpener, Finds, Gets, Matches, Ready, ThingEntry};
use crate::files::MOST_OPENED;
use crate::guard::{self, Question};
use crate::job::{JobObject, Outcome, Running};
use crate::manifest::{Manifest, Manifests};
use crate::plan::{Step, locked, put};
use crate::receipts::{self, KeptUndo, Receipt, Receipts, State, digest, now, unstored};
use crate::value::SET_PREFIX;
use crate::{
    Access, Client, Definition, Dict, Domain, Effect, IN_SET, Job, Kind, LISTED, Limits, Parameter,
    Prepared, Reason, Refusal, Row, RowKind, Store, Undo, Wanted, Written, domain,
};

/// The object every owner serves its intents at.
pub const PATH: &str = "/org/bande/Intents";
const PLAN_LASTS: Duration = Duration::from_secs(10 * 60);
const SET_LASTS: Duration = Duration::from_secs(10 * 60);
pub(crate) const JOB_LASTS: Duration = Duration::from_secs(10 * 60);
const AT_ONCE: Duration = Duration::from_millis(100);
const END_IS_READ: Duration = Duration::from_millis(200);
const MOST_PLANS: usize = 16;
const MOST_JOBS: usize = 16;
const MOST_SETS: usize = 16;
const MOST_GOT: usize = 16 * 1024 * 1024;
const ONLY: &str = "only";
const MOST_FOUND: u32 = 100;
const PROPERTIES: &str = "properties";
const CURRENT: &str = "current";
const GONE: &str = "gone";

/// The references of one thing type that an app's last focused window shows.
pub type Current = Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>;

/// A program that serves intents.
pub struct Owner {
    bus: &'static str,
    program: PathBuf,
    entries: BTreeMap<String, Entry>,
    things: BTreeMap<String, ThingEntry>,
    files: Option<Arc<FilesOpener>>,
    connect: Vec<Box<Connect>>,
    current: Current,
}

impl Owner {
    /// The installed `program` that serves on the bus name `bus`.
    pub fn new(bus: &'static str, program: impl Into<PathBuf>) -> Self {
        Self {
            bus,
            program: program.into(),
            entries: BTreeMap::new(),
            things: BTreeMap::new(),
            files: None,
            connect: Vec::new(),
            current: Arc::new(|_| Vec::new()),
        }
    }

    pub fn with<O: Store>(mut self, domain: Domain<O>) -> Self {
        self.entries.extend(domain.entries);
        self.things.extend(domain.things);
        self.files = domain.files.or(self.files);
        self.connect.push(domain.connect);
        self
    }

    /// Answers `current` in every `find`.
    pub fn current(mut self, current: Current) -> Self {
        self.current = current;
        self
    }

    /// The bus name it serves on.
    pub fn bus(&self) -> &'static str {
        self.bus
    }

    /// Writes the manifest into `folder`.
    pub fn write_manifest(&self, folder: &Path) -> io::Result<()> {
        Manifest {
            bus: self.bus.to_owned(),
            program: self.program.clone(),
            things: self
                .things
                .iter()
                .map(|(kind, thing)| (kind.clone(), thing.definition.clone()))
                .collect(),
            intents: self
                .entries
                .iter()
                .filter(|(_, entry)| entry.public)
                .map(|(id, entry)| (id.clone(), entry.definition.clone()))
                .collect(),
        }
        .write(folder)
    }

    /// Serves the intents on `connection`, which then takes the bus name or is the app that has
    /// it. Receipts are in `$XDG_STATE_HOME/bande/intents`.
    pub fn serve(self, connection: &Connection) -> zbus::Result<Serving> {
        let folder = bande_xdg::state_home()
            .map(|state| state.join("bande/intents"))
            .ok_or_else(|| zbus::Error::Failure("neither XDG_STATE_HOME nor HOME is set".into()))?;
        for connect in &self.connect {
            connect(connection);
        }
        let serves_files = self.files.is_some();
        let shared = Arc::new(Shared {
            entries: self.entries,
            things: self.things,
            files: self.files,
            current: self.current,
            folder,
            connection: connection.clone(),
            callers: Callers::default(),
            receipts: Mutex::default(),
            state: Mutex::new(Waiting {
                last_call: Some(Instant::now()),
                ..Waiting::default()
            }),
            changed: Condvar::new(),
        });
        connection
            .object_server()
            .at(PATH, Intents(Arc::clone(&shared)))?;
        if serves_files {
            connection
                .object_server()
                .at(PATH, FilesObject(Intents(Arc::clone(&shared))))?;
        }
        Ok(Serving(shared))
    }
}

/// Intents being served.
pub struct Serving(Arc<Shared>);

impl Serving {
    /// Whether a job runs or a call is being answered.
    pub fn is_working(&self) -> bool {
        self.0.state().is_working()
    }

    /// Whether a plan, set or job waits, or a call is being answered.
    pub fn is_busy(&self) -> bool {
        let mut state = self.0.state();
        state.forget_old();
        state.is_busy()
    }

    /// Blocks until nothing has been asked for `idle`, or the bus is gone, and nothing waits;
    /// false when the bus is gone.
    pub fn wait_until_idle(&self, idle: Duration) -> bool {
        let mut state = self.0.state();
        loop {
            state.forget_old();
            let wait = match state.is_busy() && !state.disconnected {
                true => state.next_end(),
                false if state.is_working() => None,
                false => {
                    let rested = state.last_call.map_or(idle, |last| last.elapsed());
                    if rested >= idle || state.disconnected {
                        return !state.disconnected;
                    }
                    Some(idle - rested)
                }
            };
            state = match wait {
                Some(wait) => {
                    self.0
                        .changed
                        .wait_timeout(state, wait)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
                None => self
                    .0
                    .changed
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner),
            };
        }
    }

    /// Cancels the jobs and blocks until each thing in hand is whole.
    pub fn stop(&self) {
        for job in self.0.state().jobs.values() {
            job.cancel();
        }
        self.finish();
    }

    /// Blocks until the jobs and calls have ended, and their callers could read how.
    pub fn finish(&self) {
        let mut state = self.0.state();
        while state.is_working() {
            state = self
                .0
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        let unread = state.jobs.values().any(|job| job.is_exported());
        drop(state);
        if unread {
            std::thread::sleep(END_IS_READ);
        }
    }

    pub(crate) fn lost_the_bus(&self) {
        self.0.state().disconnected = true;
        self.0.changed.notify_all();
    }

    pub(crate) fn watch(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

type FilesAnswered = (Vec<(zvariant::OwnedFd, String, Dict)>, Vec<String>);
type Answered = (Vec<Dict>, String, String, bool, Vec<String>);

struct Held {
    sender: String,
    limits: Limits,
    made: Instant,
    domain: String,
    intent: String,
    ready: Ready,
    program: String,
    unkept: bool,
    undoes: Option<String>,
}

struct Set {
    sender: String,
    intent: String,
    made: Instant,
    references: Vec<String>,
    cut: bool,
    foreign: bool,
}

#[derive(Default)]
struct Waiting {
    plans: HashMap<String, Held>,
    sets: HashMap<String, Set>,
    jobs: HashMap<String, Arc<Running>>,
    watched: HashSet<String>,
    calls: usize,
    last_call: Option<Instant>,
    disconnected: bool,
}

impl Waiting {
    fn forget_old(&mut self) {
        self.plans
            .retain(|_, plan| plan.made.elapsed() < PLAN_LASTS);
        self.sets.retain(|_, set| set.made.elapsed() < SET_LASTS);
    }

    fn is_working(&self) -> bool {
        self.calls > 0 || self.jobs.values().any(|job| !job.has_ended())
    }

    fn is_busy(&self) -> bool {
        self.is_working()
            || !self.plans.is_empty()
            || !self.sets.is_empty()
            || self.jobs.values().any(|job| !job.is_old())
    }

    fn next_end(&self) -> Option<Duration> {
        let plans = self
            .plans
            .values()
            .map(|plan| PLAN_LASTS.saturating_sub(plan.made.elapsed()));
        let sets = self
            .sets
            .values()
            .map(|set| SET_LASTS.saturating_sub(set.made.elapsed()));
        let jobs = self.jobs.values().filter_map(|job| job.readable_for());
        plans.chain(sets).chain(jobs).min()
    }

    fn forget_caller(&mut self, sender: &str) {
        self.plans.retain(|_, plan| plan.sender != sender);
        self.sets.retain(|_, set| set.sender != sender);
        self.watched.remove(sender);
    }
}

struct Shared {
    entries: BTreeMap<String, Entry>,
    things: BTreeMap<String, ThingEntry>,
    files: Option<Arc<FilesOpener>>,
    current: Current,
    folder: PathBuf,
    connection: Connection,
    callers: Callers,
    receipts: Mutex<receipts::Open>,
    state: Mutex<Waiting>,
    changed: Condvar,
}

fn token() -> Result<String, Refusal> {
    let mut bytes = [0u8; 16];
    let mut filled = 0;
    while filled < bytes.len() {
        filled +=
            rustix::rand::getrandom(&mut bytes[filled..], rustix::rand::GetRandomFlags::empty())
                .map_err(|error| Refusal::failed(error.to_string()))?;
    }
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn unknown_intent(id: &str) -> Refusal {
    Refusal::new(
        Reason::UnknownIntent,
        tr!("Nothing can do “{intent}”", intent = id),
    )
}

fn expired() -> Refusal {
    Refusal::new(Reason::Expired, tr!("This is too old or already done"))
}

struct Call(Arc<Shared>);

impl Call {
    fn begin(shared: &Arc<Shared>) -> Self {
        let mut state = shared.state();
        state.calls += 1;
        state.last_call = Some(Instant::now());
        Self(Arc::clone(shared))
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        let mut state = self.0.state();
        state.calls -= 1;
        state.last_call = Some(Instant::now());
        self.0.changed.notify_all();
    }
}

struct Caller {
    sender: String,
    program: String,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, Waiting> {
        locked(&self.state)
    }

    fn forget_job(&self, path: &str) {
        let Some(job) = self.state().jobs.remove(path) else {
            return;
        };
        if !job.is_exported() {
            return;
        }
        let removed = self.connection.object_server().remove::<JobObject, _>(path);
        if let Err(error) = removed {
            tracing::warn!("cannot remove a finished job: {error}");
        }
        self.changed.notify_all();
    }

    fn entry(&self, id: &str, private: bool) -> Result<&Entry, Refusal> {
        self.entries
            .get(id)
            .filter(|entry| entry.public || private)
            .ok_or_else(|| unknown_intent(id))
    }

    fn with_receipts<T>(
        &self,
        of: &str,
        use_them: impl FnOnce(&mut Receipts) -> Result<T, Refusal>,
    ) -> Result<T, Refusal> {
        let mut open = locked(&self.receipts);
        if !open.contains_key(of) {
            let mut receipts = Receipts::open(&self.folder, of)?;
            self.recover(of, &mut receipts)?;
            open.insert(of.to_owned(), receipts);
        }
        match open.get_mut(of) {
            Some(receipts) => use_them(receipts),
            None => Err(expired()),
        }
    }

    fn store(&self, of: &str) -> Result<Arc<dyn Store>, Refusal> {
        let (_, entry) = self
            .entries
            .iter()
            .find(|(id, _)| domain(id) == of)
            .ok_or_else(|| unknown_intent(of))?;
        (entry.store)()
    }

    fn recover(&self, of: &str, receipts: &mut Receipts) -> Result<(), Refusal> {
        let begun: Vec<Receipt> = receipts
            .all()
            .iter()
            .filter(|receipt| receipt.state == State::Begun)
            .cloned()
            .collect();
        for receipt in begun {
            let interrupted = (!receipt.asked.is_empty()).then(|| receipt.clone());
            receipts.interrupted.push(receipt.plan.clone());
            let left = match receipts.undo(&receipt) {
                Some(undo) => match self.left(of, undo) {
                    Err(refusal) if refusal.reason == Reason::NotFound || receipt.is_old() => {
                        tracing::warn!("{} cannot be undone: {refusal}", receipt.intent);
                        None
                    }
                    left => left?,
                },
                None => None,
            };
            let kept = match left {
                Some((undo, looks)) => {
                    receipts.kept(&receipt.token, undo, looks).and_then(|undo| {
                        receipts.keep(Receipt {
                            state: State::Whole,
                            undo: Some(undo),
                            ..receipt
                        })
                    })
                }
                None => receipts.remove(&receipt.token),
            };
            kept.map_err(|error| unstored(&error))?;
            if let Some(receipt) = interrupted {
                self.tell_interrupted(&receipt);
            }
        }
        Ok(())
    }

    fn left(&self, of: &str, undo: Undo) -> Result<Option<(Undo, Vec<String>)>, Refusal> {
        let Some(undo) = self.store(of)?.recover(undo)? else {
            return Ok(None);
        };
        let looks = self.looks(of, &undo)?;
        Ok(Some((undo, looks)))
    }

    fn tell_interrupted(&self, receipt: &Receipt) {
        let told = bande_notify::client::notify(
            &self.connection,
            &bande_notify::client::Request {
                app_name: "",
                replaces_id: 0,
                app_icon: "",
                summary: &receipt.summary,
                body: &tr!("This was interrupted. It may have been done."),
                actions: &[],
                urgency: bande_notify::Urgency::Normal,
                sound: None,
                desktop_entry: None,
            },
        );
        if let Err(error) = told {
            tracing::warn!(
                "cannot tell that {} was interrupted: {error}",
                receipt.intent
            );
        }
    }

    fn looks(&self, of: &str, undo: &Undo) -> Result<Vec<String>, Refusal> {
        let store = self.store(of)?;
        undo.touched
            .iter()
            .map(|(reference, properties)| look(&*store, reference, properties))
            .collect()
    }

    fn expand(
        &self,
        sender: &str,
        definition: &Definition,
        arguments: &mut Dict,
        most: usize,
    ) -> Result<(), Refusal> {
        for parameter in &definition.parameters {
            let Kind::List { items, .. } = &parameter.kind else {
                continue;
            };
            let (Kind::Ref { of }, Some(value)) =
                (&**items, arguments.get(parameter.name.as_str()))
            else {
                continue;
            };
            let Some(given) = value
                .try_clone()
                .ok()
                .and_then(|value| Vec::<String>::try_from(value).ok())
            else {
                continue;
            };
            let references = self.references(sender, parameter, given, most)?;
            let fits = references.iter().all(|reference| {
                of.iter()
                    .any(|kind| crate::reference_token(reference, kind).is_some())
            });
            if !fits {
                return Err(Refusal::invalid(&parameter.name, parameter.kind.accepts()));
            }
            put(arguments, &parameter.name, references)?;
        }
        Ok(())
    }

    fn references(
        &self,
        sender: &str,
        parameter: &Parameter,
        given: Vec<String>,
        most: usize,
    ) -> Result<Vec<String>, Refusal> {
        match given.as_slice() {
            [set] if set.starts_with(SET_PREFIX) => self.in_set(sender, set),
            _ if given.len() > most => {
                Err(Refusal::invalid(&parameter.name, parameter.kind.accepts()))
            }
            _ => Ok(given),
        }
    }

    fn in_set(&self, sender: &str, set: &str) -> Result<Vec<String>, Refusal> {
        let mut state = self.state();
        state.forget_old();
        state
            .sets
            .get(set)
            .filter(|held| held.sender == sender)
            .map(|held| held.references.clone())
            .ok_or_else(expired)
    }

    fn within(&self, refusal: Refusal, limits: &Limits) -> Refusal {
        let mut refusal = refusal;
        refusal.candidates.retain(|candidate| {
            let kind = candidate.split(':').next().unwrap_or_default();
            limits.grants(domain(kind))
                && match self.things.get(kind) {
                    Some(thing) => (thing.describe)(candidate, &Wanted::default(), limits).is_ok(),
                    None => limits.within(domain(kind)).is_none(),
                }
        });
        refusal
    }

    fn find(
        &self,
        caller: &Caller,
        id: &str,
        mut arguments: Dict,
        limits: &Limits,
        cursor: &str,
        count: u32,
    ) -> Result<Answered, Refusal> {
        let entry = self.entry(id, false)?;
        let Finds { kind, finder } = entry.finds.as_ref().ok_or_else(|| unknown_intent(id))?;
        limits.check(domain(id), &[Effect::Read])?;
        let thing = self.things.get(kind).ok_or_else(|| unknown_intent(id))?;
        let (set, start) = match cursor.rsplit_once('/') {
            Some((set, start)) => (
                set.to_owned(),
                start.parse::<usize>().map_err(|_| expired())?,
            ),
            None if cursor.is_empty() => {
                self.expand(&caller.sender, &entry.definition, &mut arguments, LISTED)?;
                let shown = entry.definition.parameter(CURRENT).is_some()
                    && arguments
                        .get(CURRENT)
                        .is_some_and(|value| matches!(**value, zvariant::Value::Bool(true)));
                let mut found = finder(arguments, limits)?;
                if shown {
                    let current = (self.current)(kind);
                    found
                        .references
                        .retain(|reference| current.contains(reference));
                }
                (self.hold_set(caller, id, found)?, 0)
            }
            None => return Err(expired()),
        };
        let count = count.clamp(1, MOST_FOUND) as usize;
        let (references, more, cut, matched_foreign) = {
            let mut state = self.state();
            state.forget_old();
            let held = state
                .sets
                .get(&set)
                .filter(|held| held.sender == caller.sender && held.intent == id)
                .ok_or_else(expired)?;
            let end = start.saturating_add(count);
            let page: Vec<String> = held
                .references
                .iter()
                .skip(start)
                .take(count)
                .cloned()
                .collect();
            (page, end < held.references.len(), held.cut, held.foreign)
        };
        let foreign_allowed = limits.allows(domain(id), Effect::ReadForeign);
        if matched_foreign {
            limits.check(domain(id), &[Effect::ReadForeign])?;
        }
        let wanted = Wanted {
            names: thing
                .definition
                .properties
                .iter()
                .filter(|(_, property)| property.find && (foreign_allowed || !property.foreign))
                .map(|(name, _)| name.clone())
                .collect(),
            asked: None,
        };
        let mut things = Vec::new();
        for reference in &references {
            match (thing.describe)(reference, &wanted, limits) {
                Ok((described, _)) => things.push(described),
                Err(refusal) if refusal.reason == Reason::NotFound => {}
                Err(refusal) => return Err(refusal),
            }
        }
        let cursor = match more {
            true => format!("{set}/{}", start + count),
            false => String::new(),
        };
        let effects = effects_of(&things, matched_foreign);
        Ok((things, cursor, set, cut, effects))
    }

    fn hold_set(&self, caller: &Caller, id: &str, found: Matches) -> Result<String, Refusal> {
        let name = format!("{SET_PREFIX}{}", token()?);
        let mut state = self.state();
        state.forget_old();
        let mut own: Vec<(Instant, String)> = state
            .sets
            .iter()
            .filter(|(_, set)| set.sender == caller.sender)
            .map(|(name, set)| (set.made, name.clone()))
            .collect();
        own.sort();
        let excess = (own.len() + 1).saturating_sub(MOST_SETS);
        for (_, oldest) in own.into_iter().take(excess) {
            state.sets.remove(&oldest);
        }
        state.sets.insert(
            name.clone(),
            Set {
                sender: caller.sender.clone(),
                intent: id.to_owned(),
                made: Instant::now(),
                references: found.references,
                cut: found.cut,
                foreign: found.foreign,
            },
        );
        Ok(name)
    }

    fn get(
        &self,
        caller: &Caller,
        id: &str,
        references: Vec<String>,
        properties: Vec<String>,
        arguments: Dict,
        limits: &Limits,
    ) -> Result<(Vec<Dict>, Vec<String>), Refusal> {
        let entry = self.entry(id, false)?;
        let Gets { types, check } = entry.gets.as_ref().ok_or_else(|| unknown_intent(id))?;
        let parameter = entry
            .definition
            .parameters
            .first()
            .filter(|parameter| parameter.kind.names_things());
        if let Some(name) = arguments
            .keys()
            .find(|name| *name == PROPERTIES || parameter.is_some_and(|it| it.name == **name))
        {
            return Err(Refusal::invalid(name, Vec::new()));
        }
        limits.check(domain(id), &[Effect::Read])?;
        let refused =
            |parameter: &Parameter| Refusal::invalid(&parameter.name, parameter.kind.accepts());
        let references = match (parameter, references.len()) {
            (None, 0) => types.iter().map(|kind| format!("{kind}:{ONLY}")).collect(),
            (Some(parameter), 1..) => {
                self.references(&caller.sender, parameter, references, LISTED)?
            }
            (None, _) => return Err(Refusal::invalid("references", Vec::new())),
            (Some(parameter), 0) => return Err(refused(parameter)),
        };
        let is_list = parameter.is_some_and(|it| matches!(it.kind, Kind::List { .. }));
        if let Some(parameter) = parameter
            && !is_list
            && references.len() > 1
        {
            return Err(refused(parameter));
        }
        let named = !arguments.is_empty();
        let asking = |reference: &str| -> Result<Dict, Refusal> {
            let mut asked = crate::job::copied(&arguments)?;
            match parameter {
                Some(parameter) if is_list => {
                    put(&mut asked, &parameter.name, vec![reference.to_owned()])?;
                }
                Some(parameter) => put(&mut asked, &parameter.name, reference.to_owned())?,
                None => {}
            }
            Ok(asked)
        };
        if let Some(reference) = references.first() {
            check(asking(reference)?)?;
        }
        let foreign_allowed = limits.allows(domain(id), Effect::ReadForeign);
        let mut things = Vec::new();
        let mut weight = 0;
        for reference in &references {
            let kind = reference.split(':').next().unwrap_or_default();
            let thing = types
                .iter()
                .filter_map(|kind| self.things.get(*kind))
                .find(|thing| thing.references.contains(&kind))
                .ok_or_else(|| match parameter {
                    Some(parameter) => refused(parameter),
                    None => unknown_intent(id),
                })?;
            let known = &thing.definition.properties;
            let names = match properties.is_empty() {
                true => known
                    .iter()
                    .filter(|(_, property)| {
                        !property.content && (foreign_allowed || !property.foreign)
                    })
                    .map(|(name, _)| name.clone())
                    .collect(),
                false => {
                    for name in &properties {
                        let property = known.get(name).ok_or_else(|| {
                            Refusal::invalid(PROPERTIES, known.keys().cloned().collect())
                        })?;
                        if property.foreign {
                            limits.check(domain(id), &[Effect::ReadForeign])?;
                        }
                    }
                    properties.clone()
                }
            };
            let wanted = Wanted {
                names,
                asked: match named {
                    true => Some((id.to_owned(), asking(reference)?)),
                    false => None,
                },
            };
            let (described, withheld) = (thing.describe)(reference, &wanted, limits)?;
            if withheld && !properties.is_empty() {
                limits.check(domain(id), &[Effect::ReadForeign])?;
            }
            weight += described
                .values()
                .map(|value| text_in(value))
                .sum::<usize>();
            if weight > MOST_GOT {
                return Err(Refusal::new(
                    Reason::Limited,
                    tr!("This is too much at once. Get fewer things or properties."),
                ));
            }
            things.push(described);
        }
        let effects = effects_of(&things, false);
        Ok((things, effects))
    }

    fn prepare_named(
        &self,
        caller: &Caller,
        id: &str,
        mut arguments: Dict,
        limits: &Limits,
        private: bool,
    ) -> Result<(Ready, bool), Refusal> {
        let entry = self.entry(id, private)?;
        let prepare = entry.preparer.as_ref().ok_or_else(|| unknown_intent(id))?;
        if !limits.grants(domain(id)) {
            limits.check(domain(id), &[])?;
        }
        let most = match private {
            true => IN_SET,
            false => LISTED,
        };
        self.expand(&caller.sender, &entry.definition, &mut arguments, most)?;
        let mut ready = prepare(arguments, limits)?;
        ready.effects.sort();
        ready.effects.dedup();
        if let Some(effect) = ready
            .effects
            .iter()
            .find(|effect| !entry.definition.effects.contains(effect))
        {
            tracing::error!("{id} does not declare that it can {effect}");
            return Err(Refusal::failed(tr!(
                "“{intent}” would do more than it says",
                intent = id
            )));
        }
        limits.check(domain(id), &ready.effects)?;
        Ok((ready, entry.definition.undo))
    }

    fn prepare(
        &self,
        caller: &Caller,
        id: &str,
        arguments: Dict,
        limits: &Limits,
    ) -> Result<(String, Dict), Refusal> {
        let (mut ready, undo) = self.prepare_named(caller, id, arguments, limits, false)?;
        let of = domain(id).to_owned();
        let kept = self.with_receipts(&of, |receipts| Ok(receipts.can_keep()))?;
        let unkept = undo && !kept;
        if unkept {
            if limits.agent().is_some() {
                return Err(Refusal::unavailable(tr!(
                    "Undo cannot be kept now, so this is not done"
                )));
            }
            ready.rows.push(Row::new(
                RowKind::Loss,
                tr!("Undo"),
                tr!("This cannot be undone: there is no room to keep it"),
            ));
        }
        self.hold(Held {
            sender: caller.sender.clone(),
            limits: limits.clone(),
            made: Instant::now(),
            domain: of,
            intent: id.to_owned(),
            ready,
            program: caller.program.clone(),
            unkept,
            undoes: None,
        })
    }

    fn hold(&self, held: Held) -> Result<(String, Dict), Refusal> {
        let name = token()?;
        let mut description = Dict::new();
        let words: Vec<&str> = held.ready.effects.iter().map(|it| it.word()).collect();
        put(&mut description, "effects", words)?;
        put(&mut description, "summary", held.ready.summary.as_str())?;
        let rows: Vec<Dict> = held.ready.rows.iter().map(Row::wire).collect();
        put(&mut description, "rows", rows)?;
        let mut state = self.state();
        state.forget_old();
        let waiting = state
            .plans
            .values()
            .filter(|plan| plan.sender == held.sender && plan.limits.agent() == held.limits.agent())
            .count();
        if waiting >= MOST_PLANS {
            return Err(Refusal::new(
                Reason::Limited,
                tr!("Too much is waiting. Try again in 10 minutes."),
            ));
        }
        state.plans.insert(name.clone(), held);
        Ok((name, description))
    }

    fn prepare_undo(
        &self,
        caller: &Caller,
        receipt: &str,
        limits: &Limits,
    ) -> Result<(String, Dict), Refusal> {
        let (of, name) = receipt.split_once(':').ok_or_else(expired)?;
        limits.check(of, &[])?;
        self.store(of).map_err(|refusal| match refusal.reason {
            Reason::UnknownIntent => expired(),
            _ => refusal,
        })?;
        let (undo, left, summary) = self.with_receipts(of, |receipts| {
            let found = receipts
                .find(name)
                .filter(|receipt| {
                    limits.agent().is_none() || limits.agent() == receipt.agent.as_deref()
                })
                .ok_or_else(expired)?;
            let undo = receipts.undo(found).ok_or_else(expired)?;
            let left = found.undo.as_ref().map(|undo| undo.left.clone());
            Ok((undo, left.unwrap_or_default(), found.summary.clone()))
        })?;
        if self.looks(of, &undo)? != left {
            return Err(Refusal::conflict(tr!(
                "This was changed again since: {change}",
                change = summary
            )));
        }
        let ready = match self.entries.contains_key(&undo.intent) {
            true => {
                let arguments = self
                    .entry(&undo.intent, true)
                    .and_then(|entry| undo_arguments(&entry.definition, &undo))?;
                let (ready, _) =
                    self.prepare_named(caller, &undo.intent, arguments, limits, true)?;
                ready
            }
            false => self.prepare_elsewhere(&undo, limits)?,
        };
        self.hold(Held {
            sender: caller.sender.clone(),
            limits: limits.clone(),
            made: Instant::now(),
            domain: of.to_owned(),
            intent: undo.intent.clone(),
            ready,
            program: caller.program.clone(),
            unkept: true,
            undoes: Some(name.to_owned()),
        })
    }

    fn prepare_elsewhere(&self, undo: &Undo, limits: &Limits) -> Result<Ready, Refusal> {
        let manifests = Manifests::installed();
        let (manifest, definition) = manifests.intent(&undo.intent).ok_or_else(|| {
            Refusal::unavailable(tr!("The program that can undo this is not installed"))
        })?;
        let client = Client::passing(&self.connection, limits)?;
        let prepared = undo
            .in_calls(definition)
            .into_iter()
            .map(|(undo, references)| {
                let arguments = undo_arguments(definition, &undo)?;
                let prepared = client.prepare(&manifest.bus, &undo.intent, &arguments)?;
                Ok((prepared, references))
            })
            .collect::<Result<Vec<(Prepared, Vec<String>)>, Refusal>>()?;
        let plans = || prepared.iter().map(|(prepared, _)| prepared);
        let summaries: Vec<&str> = plans().map(|plan| plan.summary.as_str()).collect();
        Ok(Ready {
            effects: plans()
                .flat_map(|plan| plan.effects.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            summary: summaries.join("; "),
            rows: plans()
                .flat_map(|plan| &plan.rows)
                .map(Row::from_wire)
                .collect(),
            references: Vec::new(),
            run: Box::new(move |job, _| {
                for (call, (prepared, references)) in prepared.into_iter().enumerate() {
                    if job.cancelled() {
                        break;
                    }
                    let performed = match client.perform(prepared) {
                        Ok(performed) => performed,
                        Err(refusal) if call == 0 => return Err(refusal),
                        Err(refusal) => {
                            for reference in references {
                                job.failed(&reference, refusal.clone());
                            }
                            continue;
                        }
                    };
                    for (reference, refusal) in performed.failed() {
                        job.failed(&reference, refusal);
                    }
                    if performed.cancelled {
                        job.stop();
                        break;
                    }
                }
                Ok((Dict::new(), None))
            }),
        })
    }

    fn take_plan(&self, caller: &Caller, plan: &str) -> Result<Held, Refusal> {
        let mut state = self.state();
        state.forget_old();
        match state.plans.get(plan) {
            Some(held) if held.sender == caller.sender => {
                state.plans.remove(plan).ok_or_else(expired)
            }
            Some(_) => Err(Refusal::new(
                Reason::AccessDenied,
                tr!("This was prepared by another program"),
            )),
            None => {
                drop(state);
                Err(match self.was_interrupted(plan) {
                    true => Refusal::interrupted(),
                    false => expired(),
                })
            }
        }
    }

    fn was_interrupted(&self, plan: &str) -> bool {
        let mut domains: Vec<&str> = self.entries.keys().map(|id| domain(id)).collect();
        domains.dedup();
        domains.into_iter().any(|of| {
            self.with_receipts(of, |receipts| {
                Ok(receipts.interrupted.iter().any(|it| it == plan))
            })
            .unwrap_or(false)
        })
    }

    fn run(&self, plan: &str, held: Held, job: &Arc<Job>, record: &Record) -> Outcome {
        let question = Question {
            agent: held.limits.agent(),
            effects: &held.ready.effects,
        };
        if let Err(refusal) = guard::allow(&question) {
            return Outcome::refused(refusal);
        }
        match held.limits.agent() {
            Some(_) => tracing::info!("performs {} for an agent", held.intent),
            None => tracing::info!("performs {} for {}", held.intent, held.program),
        }
        reached("performing");
        if !record.asked.is_empty()
            && let Err(refusal) = self.store_receipt(record, State::Begun, None)
        {
            return Outcome::refused(refusal);
        }
        let outcome = (held.ready.run)(job, &held.limits);
        reached("performed");
        let (output, left, refusal) = match outcome {
            Ok((output, undo)) => (output, Ok(undo), None),
            Err(refusal) => (Dict::new(), self.recovered(record), Some(refusal)),
        };
        let mut receipt = String::new();
        let settled = left.and_then(|undo| match undo.filter(|_| !held.unkept) {
            Some(undo) => {
                let looks = self.looks(&held.domain, &undo)?;
                self.store_receipt(record, State::Whole, Some((undo, looks)))?;
                receipt = format!("{}:{}", held.domain, record.token);
                Ok(())
            }
            None => self.with_receipts(&held.domain, |receipts| {
                if refusal.is_none()
                    && let Some(undone) = &held.undoes
                    && !job.was_partial()
                {
                    receipts.remove(undone).map_err(|error| unstored(&error))?;
                }
                receipts
                    .remove(&record.token)
                    .map_err(|error| unstored(&error))
            }),
        });
        if let Err(refusal) = settled {
            tracing::warn!("{} {plan} has no undo: {refusal}", held.intent);
        }
        if refusal.is_none()
            && let Some(undone) = held.undoes.as_deref().filter(|_| job.was_partial())
            && let Err(refusal) = self.look_again(&held.domain, undone, &job.left_out())
        {
            tracing::warn!("what is left to undo is unknown: {refusal}");
        }
        let within = |refusal| self.within(refusal, &held.limits);
        let refusal = refusal.map(within);
        let mut result = match job.result(output, within) {
            Ok(result) => result,
            Err(refusal) => return Outcome::refused(refusal),
        };
        match put(&mut result, "receipt", receipt) {
            Ok(()) => Outcome { result, refusal },
            Err(refusal) => Outcome::refused(refusal),
        }
    }

    fn recovered(&self, record: &Record) -> Result<Option<Undo>, Refusal> {
        let kept = self.with_receipts(&record.domain, |receipts| {
            Ok(receipts
                .all()
                .iter()
                .find(|receipt| receipt.token == record.token)
                .and_then(|receipt| receipts.undo(receipt)))
        })?;
        match kept {
            Some(undo) => self.store(&record.domain)?.recover(undo),
            None => Ok(None),
        }
    }

    fn look_again(&self, of: &str, token: &str, left_out: &HashSet<String>) -> Result<(), Refusal> {
        let found = self.with_receipts(of, |receipts| {
            Ok(receipts
                .find(token)
                .and_then(|receipt| receipt.undo.clone()))
        })?;
        let Some(undo) = found else {
            return Ok(());
        };
        let store = self.store(of)?;
        let looks = undo
            .touched
            .iter()
            .zip(undo.left)
            .map(
                |((reference, properties), left)| match left_out.contains(reference) {
                    true => Ok(left),
                    false => look(&*store, reference, properties),
                },
            )
            .collect::<Result<Vec<_>, Refusal>>()?;
        self.with_receipts(of, |receipts| {
            receipts
                .look_again(token, looks)
                .map_err(|error| unstored(&error))
        })
    }

    fn keep_steps(&self, record: &Record, steps: &[Undo], step: Step) -> Result<(), Refusal> {
        let add = |step: Option<&Undo>| {
            self.with_receipts(&record.domain, |receipts| {
                receipts
                    .add_step(&record.token, step)
                    .map_err(|error| unstored(&error))
            })
        };
        match (step, steps) {
            (Step::Kept, [first]) => {
                self.store_receipt(record, State::Begun, Some((first.clone(), Vec::new())))
            }
            (Step::Kept, steps) => add(steps.last()),
            (Step::Left, []) if record.asked.is_empty() => {
                self.with_receipts(&record.domain, |receipts| {
                    receipts
                        .remove(&record.token)
                        .map_err(|error| unstored(&error))
                })
            }
            (Step::Left, []) => self.store_receipt(record, State::Begun, None),
            (Step::Left, _) => add(None),
        }
    }

    fn store_receipt(
        &self,
        record: &Record,
        state: State,
        undo: Option<(Undo, Vec<String>)>,
    ) -> Result<(), Refusal> {
        self.with_receipts(&record.domain, |receipts| {
            let undo: Option<KeptUndo> = undo
                .map(|(undo, left)| receipts.kept(&record.token, undo, left))
                .transpose()
                .map_err(|error| unstored(&error))?;
            receipts
                .keep(Receipt {
                    token: record.token.clone(),
                    plan: record.plan.clone(),
                    intent: record.intent.clone(),
                    summary: record.summary.clone(),
                    references: record.references.clone(),
                    agent: record.agent.clone(),
                    made: now(),
                    state,
                    asked: record.asked.clone(),
                    undo,
                })
                .map_err(|error| unstored(&error))
        })
    }

    fn perform(
        self: &Arc<Self>,
        caller: &Caller,
        plan: &str,
    ) -> Result<(OwnedObjectPath, String, Dict), Refusal> {
        let held = self.take_plan(caller, plan)?;
        let record = Record {
            token: token()?,
            plan: plan.to_owned(),
            domain: held.domain.clone(),
            intent: held.intent.clone(),
            summary: held.ready.summary.clone(),
            references: held.ready.references.clone(),
            agent: held.limits.agent().map(str::to_owned),
            asked: held
                .ready
                .effects
                .iter()
                .copied()
                .filter(|effect| {
                    matches!(effect, Effect::Send | Effect::Disrupt | Effect::Critical)
                })
                .collect(),
        };
        let path = format!("{PATH}/job/{}", token()?);
        let running = Running::new(self.connection.clone(), caller.sender.clone(), path.clone());
        let reporter = Arc::clone(&running);
        let keeper = Arc::clone(self);
        let keeping = (!held.unkept).then(|| record.clone());
        let job = Job::new(
            held.limits.agent().map(str::to_owned),
            held.ready.effects.clone(),
            move |steps, step| {
                if let Some(record) = &keeping {
                    keeper.keep_steps(record, steps, step)?;
                }
                reached("kept");
                Ok(())
            },
            move || reporter.progressed(),
        );
        running.attach(Arc::clone(&job));
        if let Err(refusal) = self.hold_job(&path, &running) {
            self.state().plans.insert(plan.to_owned(), held);
            return Err(refusal);
        }
        let (ended, end) = mpsc::channel();
        let shared = Arc::clone(self);
        let worker = Arc::clone(&running);
        let plan = plan.to_owned();
        let spawned = std::thread::Builder::new()
            .name("intent".to_owned())
            .spawn(move || {
                let outcome = shared.run(&plan, held, &job, &record);
                worker.end(outcome, job.was_stopped());
                let abandoned = !shared.state().watched.contains(worker.sender());
                if abandoned {
                    shared.forget_job(worker.path());
                }
                shared.changed.notify_all();
                let _ = ended.send(());
            });
        if let Err(error) = spawned {
            self.state().jobs.remove(&path);
            return Err(Refusal::failed(error.to_string()));
        }
        if end.recv_timeout(AT_ONCE).is_ok()
            && let Some(answer) = running.answer_at_once()
        {
            self.state().jobs.remove(&path);
            let none = OwnedObjectPath::try_from("/").map_err(zbus::Error::from)?;
            let (state, result) = answer?;
            return Ok((none, state, result));
        }
        self.connection
            .object_server()
            .at(path.as_str(), JobObject(Arc::clone(&running)))?;
        running.exported();
        let state = running.word().to_owned();
        let path = OwnedObjectPath::try_from(path).map_err(zbus::Error::from)?;
        Ok((path, state, running.result()?))
    }

    fn hold_job(&self, path: &str, running: &Arc<Running>) -> Result<(), Refusal> {
        let mut state = self.state();
        let jobs = state
            .jobs
            .values()
            .filter(|job| {
                job.sender() == running.sender()
                    && job.agent() == running.agent()
                    && !job.has_ended()
            })
            .count();
        if jobs >= MOST_JOBS {
            return Err(Refusal::new(
                Reason::Limited,
                tr!("Too much is being done. Try again when some of it has ended."),
            ));
        }
        state.jobs.insert(path.to_owned(), Arc::clone(running));
        Ok(())
    }

    fn open_files(
        &self,
        references: &[String],
        access: &str,
        limits: &Limits,
    ) -> Result<FilesAnswered, Refusal> {
        let files = self.files.as_ref().ok_or_else(expired)?()?;
        let access = Access::from_word(access).ok_or_else(|| {
            Refusal::invalid(
                "access",
                [Access::Read, Access::Folder]
                    .map(|it| it.word().to_owned())
                    .to_vec(),
            )
        })?;
        if references.len() > MOST_OPENED {
            return Err(Refusal::invalid("references", Vec::new()));
        }
        let mut opened = Vec::new();
        let mut effects = BTreeSet::new();
        for reference in references {
            limits.check(domain(reference), &[Effect::Read])?;
            let (file, caused) = files.open(reference, access, limits)?;
            limits.check(domain(reference), &caused)?;
            let mut about = Dict::new();
            put(&mut about, "name", file.name)?;
            put(&mut about, "media_type", file.media_type)?;
            put(&mut about, "size", file.size)?;
            opened.push((file.file.into(), file.reference, about));
            effects.extend(caused);
        }
        let words = effects.iter().map(|effect| effect.word().to_owned());
        Ok((opened, words.collect()))
    }

    fn register_files(
        &self,
        folder: zvariant::OwnedFd,
        written: Vec<(String, String, String)>,
        limits: &Limits,
    ) -> Result<Vec<String>, Refusal> {
        let files = self.files.as_ref().ok_or_else(expired)?()?;
        if written.len() > LISTED {
            return Err(Refusal::invalid("files", Vec::new()));
        }
        let written = written
            .into_iter()
            .map(|(temporary, name, replaces)| Written {
                temporary,
                name,
                replaces: Some(replaces).filter(|reference| !reference.is_empty()),
            })
            .collect();
        files.register(folder.into(), written, limits)
    }

    fn receipts(&self, limits: &Limits, count: u32) -> Result<Vec<Dict>, Refusal> {
        let mut domains: Vec<&str> = self.entries.keys().map(|id| domain(id)).collect();
        domains.dedup();
        let mut found: Vec<(u64, Dict)> = Vec::new();
        for of in domains.into_iter().filter(|of| limits.grants(of)) {
            self.with_receipts(of, |receipts| {
                for receipt in receipts.all() {
                    let shown = receipts.find(&receipt.token).is_some()
                        && (limits.agent().is_none() || limits.agent() == receipt.agent.as_deref());
                    if shown {
                        found.push((receipt.made, receipt.wire(of)?));
                    }
                }
                Ok(())
            })?;
        }
        found.reverse();
        found.sort_by_key(|(made, _)| std::cmp::Reverse(*made));
        Ok(found
            .into_iter()
            .take(count as usize)
            .map(|(_, receipt)| receipt)
            .collect())
    }
}

fn look(store: &dyn Store, reference: &str, properties: &[String]) -> Result<String, Refusal> {
    match store.look(reference, properties) {
        Ok(look) => Ok(digest(&look)),
        Err(refusal) if refusal.reason == Reason::NotFound => Ok(GONE.to_owned()),
        Err(refusal) => Err(refusal),
    }
}

fn undo_arguments(definition: &Definition, undo: &Undo) -> Result<Dict, Refusal> {
    undo.arguments
        .as_object()
        .ok_or_else(expired)
        .and_then(|arguments| definition.arguments_from_json(&undo.intent, arguments))
        .map_err(|refusal| {
            tracing::warn!("a receipt cannot be used: {refusal}");
            expired()
        })
}

fn text_in(value: &zvariant::Value<'_>) -> usize {
    match value {
        zvariant::Value::Str(text) => text.len(),
        zvariant::Value::Value(inner) => text_in(inner),
        zvariant::Value::Array(items) => items.iter().map(text_in).sum(),
        _ => 0,
    }
}

#[derive(Clone)]
struct Record {
    token: String,
    plan: String,
    domain: String,
    intent: String,
    summary: String,
    references: Vec<String>,
    agent: Option<String>,
    asked: Vec<Effect>,
}

fn effects_of(things: &[Dict], matched_foreign: bool) -> Vec<String> {
    let foreign = matched_foreign
        || things.iter().any(|thing| {
            thing.get("foreign").is_some_and(
                |foreign| matches!(&**foreign, zvariant::Value::Array(names) if !names.is_empty()),
            )
        });
    [(Effect::Read, true), (Effect::ReadForeign, foreign)]
        .into_iter()
        .filter(|(_, caused)| *caused)
        .map(|(effect, _)| effect.word().to_owned())
        .collect()
}

#[cfg(debug_assertions)]
fn reached(point: &str) {
    if std::env::var("BANDE_INTENTS_CRASH").is_ok_and(|crash| crash == point) {
        std::process::abort();
    }
}

#[cfg(not(debug_assertions))]
fn reached(_point: &str) {}

struct Intents(Arc<Shared>);

impl Intents {
    async fn admit(&self, header: &Header<'_>, limits: &Limits) -> Result<Caller, Refusal> {
        let sender = header
            .sender()
            .ok_or_else(|| Refusal::new(Reason::AccessDenied, tr!("The caller is unknown")))?
            .to_string();
        let bus = self.0.connection.inner();
        let program = self.0.callers.admit(bus, &sender).await?;
        if limits.agent().is_some() {
            let manifests = blocking::unblock(Manifests::installed).await;
            self.0.callers.passes_on(bus, &sender, &manifests).await?;
        }
        Ok(Caller { sender, program })
    }

    async fn watch(&self, sender: &str) -> Result<(), Refusal> {
        if !self.0.state().watched.insert(sender.to_owned()) {
            return Ok(());
        }
        let bus = self.0.connection.inner().clone();
        let watched = async {
            let proxy = zbus::fdo::DBusProxy::new(&bus).await?;
            let changes = proxy
                .receive_name_owner_changed_with_args(&[(0, sender)])
                .await?;
            let name = zbus::names::BusName::try_from(sender.to_owned())?;
            Ok::<_, zbus::Error>((proxy.name_has_owner(name).await?, changes))
        }
        .await;
        let (there, mut changes) = watched.inspect_err(|_| {
            self.0.state().watched.remove(sender);
        })?;
        let shared = Arc::clone(&self.0);
        let sender = sender.to_owned();
        let left = async move {
            if there {
                changes.next().await;
            }
            let ended = {
                let mut state = shared.state();
                state.forget_caller(&sender);
                state.ended_jobs(|job| job.sender() == sender)
            };
            for path in ended {
                let shared = Arc::clone(&shared);
                blocking::unblock(move || shared.forget_job(&path)).await;
            }
            shared.changed.notify_all();
        };
        bus.executor().spawn(left, "a caller's plans").detach();
        Ok(())
    }

    async fn answer<T: Send + 'static>(
        &self,
        header: &Header<'_>,
        limits: Dict,
        ask: impl FnOnce(&Arc<Shared>, &Caller, &Limits) -> Result<T, Refusal> + Send + 'static,
    ) -> Result<T, Refusal> {
        let call = Call::begin(&self.0);
        let limits = Limits::decode(limits)?;
        let caller = self.admit(header, &limits).await?;
        self.watch(&caller.sender).await?;
        let shared = Arc::clone(&self.0);
        let answered = blocking::unblock(move || {
            let old = shared.state().ended_jobs(Running::is_old);
            for path in old {
                shared.forget_job(&path);
            }
            ask(&shared, &caller, &limits).map_err(|refusal| shared.within(refusal, &limits))
        })
        .await;
        drop(call);
        answered
    }
}

impl Waiting {
    fn ended_jobs(&self, which: impl Fn(&Running) -> bool) -> Vec<String> {
        self.jobs
            .iter()
            .filter(|(_, job)| job.has_ended() && which(job))
            .map(|(path, _)| path.clone())
            .collect()
    }
}

#[zbus::interface(name = "org.bande.Intents")]
impl Intents {
    #[zbus(out_args("things", "cursor", "set", "set_cut", "effects"))]
    async fn find(
        &self,
        intent: String,
        arguments: Dict,
        limits: Dict,
        cursor: String,
        count: u32,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Answered, Refusal> {
        self.answer(&header, limits, move |shared, caller, limits| {
            shared.find(caller, &intent, arguments, limits, &cursor, count)
        })
        .await
    }

    #[zbus(out_args("things", "effects"))]
    async fn get(
        &self,
        intent: String,
        references: Vec<String>,
        properties: Vec<String>,
        arguments: Dict,
        limits: Dict,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(Vec<Dict>, Vec<String>), Refusal> {
        self.answer(&header, limits, move |shared, caller, limits| {
            shared.get(caller, &intent, references, properties, arguments, limits)
        })
        .await
    }

    #[zbus(out_args("plan", "description"))]
    async fn prepare(
        &self,
        intent: String,
        arguments: Dict,
        limits: Dict,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(String, Dict), Refusal> {
        self.answer(&header, limits, move |shared, caller, limits| {
            shared.prepare(caller, &intent, arguments, limits)
        })
        .await
    }

    #[zbus(out_args("job", "state", "result"))]
    async fn perform(
        &self,
        plan: String,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(OwnedObjectPath, String, Dict), Refusal> {
        self.answer(&header, Dict::new(), move |shared, caller, _| {
            shared.perform(caller, &plan)
        })
        .await
    }

    #[zbus(out_args("receipts"))]
    async fn receipts(
        &self,
        limits: Dict,
        count: u32,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Vec<Dict>, Refusal> {
        self.answer(&header, limits, move |shared, _, limits| {
            shared.receipts(limits, count)
        })
        .await
    }

    #[zbus(out_args("plan", "description"))]
    async fn prepare_undo(
        &self,
        receipt: String,
        limits: Dict,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(String, Dict), Refusal> {
        self.answer(&header, limits, move |shared, caller, limits| {
            shared.prepare_undo(caller, &receipt, limits)
        })
        .await
    }
}

struct FilesObject(Intents);

#[zbus::interface(name = "org.bande.Intents.Files")]
impl FilesObject {
    #[zbus(out_args("files", "effects"))]
    async fn open(
        &self,
        references: Vec<String>,
        access: String,
        limits: Dict,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<FilesAnswered, Refusal> {
        self.0
            .answer(&header, limits, move |shared, _, limits| {
                shared.open_files(&references, &access, limits)
            })
            .await
    }

    #[zbus(out_args("references"))]
    async fn register(
        &self,
        folder: zvariant::OwnedFd,
        files: Vec<(String, String, String)>,
        limits: Dict,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Vec<String>, Refusal> {
        self.0
            .answer(&header, limits, move |shared, _, limits| {
                shared.register_files(folder, files, limits)
            })
            .await
    }
}
