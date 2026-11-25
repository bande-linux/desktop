use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::plan::{locked, thing_dict};
use crate::{
    Definition, Describe, Dict, Effect, Find, IN_SET, Intent, Job, Kind, Limits, Output, Perform,
    Ref, Refusal, Returns, Row, Store, Thing, ThingDefinition, Undo, Wanted,
};

type Finder = dyn Fn(Dict, &Limits) -> Result<Matches, Refusal> + Send + Sync;
type Describer = dyn Fn(&str, &Wanted, &Limits) -> Result<Dict, Refusal> + Send + Sync;
type Preparer = dyn Fn(Dict, &Limits) -> Result<Ready, Refusal> + Send + Sync;
type Opener = dyn Fn() -> Result<Arc<dyn Store>, Refusal> + Send + Sync;
type Run = dyn FnOnce(&Job) -> Result<(Dict, Option<Undo>), Refusal> + Send;

pub(crate) struct Matches {
    pub(crate) references: Vec<String>,
    pub(crate) foreign: bool,
    pub(crate) cut: bool,
}

pub(crate) struct Ready {
    pub(crate) effects: Vec<Effect>,
    pub(crate) summary: String,
    pub(crate) rows: Vec<Row>,
    pub(crate) references: Vec<String>,
    pub(crate) run: Box<Run>,
}

pub(crate) enum Action {
    Find(String, Box<Finder>),
    Get,
    Perform(Box<Preparer>),
}

pub(crate) struct Entry {
    pub(crate) definition: Definition,
    pub(crate) public: bool,
    pub(crate) action: Action,
    pub(crate) store: Arc<Opener>,
}

pub(crate) struct ThingEntry {
    pub(crate) definition: ThingDefinition,
    pub(crate) describe: Arc<Describer>,
}

/// The intents on one store: which there are, and how the store is opened when the first is
/// called.
pub struct Domain<O> {
    store: Arc<Lazy<O>>,
    pub(crate) entries: BTreeMap<String, Entry>,
    pub(crate) things: BTreeMap<String, ThingEntry>,
}

struct Lazy<O> {
    open: Box<dyn Fn() -> Result<O, Refusal> + Send + Sync>,
    opened: Mutex<Option<Arc<O>>>,
}

impl<O> Lazy<O> {
    fn get(&self) -> Result<Arc<O>, Refusal> {
        let mut opened = locked(&self.opened);
        match &*opened {
            Some(store) => Ok(Arc::clone(store)),
            None => {
                let store = Arc::new((self.open)()?);
                *opened = Some(Arc::clone(&store));
                Ok(store)
            }
        }
    }
}

impl<O: Store> Domain<O> {
    /// `open` is called again after it failed.
    pub fn new(open: impl Fn() -> Result<O, Refusal> + Send + Sync + 'static) -> Self {
        Self {
            store: Arc::new(Lazy {
                open: Box::new(open),
                opened: Mutex::new(None),
            }),
            entries: BTreeMap::new(),
            things: BTreeMap::new(),
        }
    }

    fn add<I: Intent>(&mut self, public: bool, returns: Option<Returns>, action: Action) {
        let store = Arc::clone(&self.store);
        self.entries.insert(
            I::ID.to_owned(),
            Entry {
                definition: Definition {
                    returns,
                    ..I::definition()
                },
                public,
                action,
                store: Arc::new(move || Ok(store.get()? as Arc<dyn Store>)),
            },
        );
    }

    /// Serves things of this type to `find` and `get`.
    pub fn thing<T: Thing>(mut self) -> Self
    where
        O: Describe<T>,
    {
        let store = Arc::clone(&self.store);
        self.things.insert(
            T::TYPE.to_owned(),
            ThingEntry {
                definition: T::definition(),
                describe: Arc::new(move |reference, wanted, limits| {
                    let reference = Ref::<T>::parse(reference)
                        .ok_or_else(|| Refusal::not_found(reference.to_owned()))?;
                    let thing = store.get()?.describe(&reference, wanted, limits)?;
                    thing_dict(thing, |name| wanted.has(name))
                }),
            },
        );
        self
    }

    /// Serves an intent that finds things.
    pub fn find<I: Find<O>>(mut self) -> Self
    where
        O: Describe<I::Thing>,
    {
        let store = Arc::clone(&self.store);
        let finder = move |arguments: Dict, limits: &Limits| {
            let found = I::decode(arguments)?.find(&*store.get()?, limits)?;
            let cut = found.references.len() > IN_SET;
            Ok(Matches {
                references: found
                    .references
                    .into_iter()
                    .take(IN_SET)
                    .map(|reference| reference.as_str().to_owned())
                    .collect(),
                foreign: found.matched_foreign,
                cut,
            })
        };
        let kind = <I::Thing as Thing>::TYPE.to_owned();
        self.add::<I>(
            true,
            Some(Returns::List(kind.clone())),
            Action::Find(kind, Box::new(finder)),
        );
        self.thing::<I::Thing>()
    }

    /// Serves an intent that gives properties of known things: its first parameter is the
    /// [`crate::Refs`], the other `properties`.
    pub fn get<I: Intent>(mut self) -> Self {
        let returns = I::definition().parameters.first().and_then(|parameter| {
            let Kind::List { items, .. } = &parameter.kind else {
                return None;
            };
            match &**items {
                Kind::Ref { of } => of.first().cloned().map(Returns::List),
                _ => None,
            }
        });
        self.add::<I>(true, returns, Action::Get);
        self
    }

    /// Serves an intent that does something.
    pub fn perform<I: Perform<O>>(mut self) -> Self {
        let preparer = self.preparer::<I>();
        self.add::<I>(true, I::Output::returns(), preparer);
        self
    }

    /// Adds an intent that only an [`Undo`] can name; no manifest lists it.
    pub fn private<I: Perform<O>>(mut self) -> Self {
        let preparer = self.preparer::<I>();
        self.add::<I>(false, None, preparer);
        self
    }

    fn preparer<I: Perform<O>>(&self) -> Action {
        let store = Arc::clone(&self.store);
        Action::Perform(Box::new(move |arguments, limits| {
            let owner = store.get()?;
            let plan = I::decode(arguments)?.prepare(&owner, limits)?;
            let prepared = plan.prepared;
            Ok(Ready {
                effects: plan.effects,
                summary: plan.summary,
                rows: plan.rows,
                references: plan.references,
                run: Box::new(move |job| {
                    let done = I::perform(prepared, &owner, job)?;
                    Ok((done.output.into_result()?, done.undo))
                }),
            })
        }))
    }
}
