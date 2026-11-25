use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use zbus::blocking::Connection;

use crate::plan::{locked, thing_dict};
use crate::{
    Definition, Describe, Dict, Effect, Files, Find, IN_SET, Intent, Job, Limits, Output, Perform,
    Ref, Refusal, Returns, Row, Store, Thing, ThingDefinition, Undo, Wanted, domain,
};

type Finder = dyn Fn(Dict, &Limits) -> Result<Matches, Refusal> + Send + Sync;
type Describer = dyn Fn(&str, &Wanted, &Limits) -> Result<(Dict, bool), Refusal> + Send + Sync;
type Preparer = dyn Fn(Dict, &Limits) -> Result<Ready, Refusal> + Send + Sync;
type Opener = dyn Fn() -> Result<Arc<dyn Store>, Refusal> + Send + Sync;
pub(crate) type FilesOpener = dyn Fn() -> Result<Arc<dyn Files>, Refusal> + Send + Sync;
pub(crate) type Connect = dyn Fn(&Connection) + Send + Sync;
type Opens<O> = dyn Fn(&Connection) -> Result<O, Refusal> + Send + Sync;
type Run = dyn FnOnce(&Job, &Limits) -> Result<(Dict, Option<Undo>), Refusal> + Send;

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

pub(crate) struct Finds {
    pub(crate) kind: String,
    pub(crate) finder: Box<Finder>,
}

pub(crate) struct Gets {
    pub(crate) types: Vec<&'static str>,
    pub(crate) check: Box<dyn Fn(Dict) -> Result<(), Refusal> + Send + Sync>,
}

pub(crate) struct Entry {
    pub(crate) definition: Definition,
    pub(crate) public: bool,
    pub(crate) finds: Option<Finds>,
    pub(crate) gets: Option<Gets>,
    pub(crate) preparer: Option<Box<Preparer>>,
    pub(crate) store: Arc<Opener>,
}

pub(crate) struct ThingEntry {
    pub(crate) definition: ThingDefinition,
    pub(crate) references: &'static [&'static str],
    pub(crate) describe: Arc<Describer>,
}

/// The intents on one store: which there are, and how the store is opened when the first is
/// called.
pub struct Domain<O> {
    store: Arc<Lazy<O>>,
    pub(crate) entries: BTreeMap<String, Entry>,
    pub(crate) things: BTreeMap<String, ThingEntry>,
    pub(crate) files: Option<Arc<FilesOpener>>,
    pub(crate) connect: Box<Connect>,
}

/// The thing types a `get` gives: one, or a tuple of them.
pub trait Things<O> {
    #[doc(hidden)]
    const TYPES: &'static [&'static str];

    #[doc(hidden)]
    fn add(domain: Domain<O>) -> Domain<O>;
}

impl<O: Store + Describe<T>, T: Thing> Things<O> for T {
    const TYPES: &'static [&'static str] = &[T::TYPE];

    fn add(domain: Domain<O>) -> Domain<O> {
        domain.thing::<T>()
    }
}

impl<O, A: Thing, B: Thing> Things<O> for (A, B)
where
    O: Store + Describe<A> + Describe<B>,
{
    const TYPES: &'static [&'static str] = &[A::TYPE, B::TYPE];

    fn add(domain: Domain<O>) -> Domain<O> {
        domain.thing::<A>().thing::<B>()
    }
}

impl<O, A: Thing, B: Thing, C: Thing> Things<O> for (A, B, C)
where
    O: Store + Describe<A> + Describe<B> + Describe<C>,
{
    const TYPES: &'static [&'static str] = &[A::TYPE, B::TYPE, C::TYPE];

    fn add(domain: Domain<O>) -> Domain<O> {
        domain.thing::<A>().thing::<B>().thing::<C>()
    }
}

struct Lazy<O> {
    open: Box<Opens<O>>,
    connection: OnceLock<Connection>,
    opened: Mutex<Option<Arc<O>>>,
}

impl<O> Lazy<O> {
    fn get(&self) -> Result<Arc<O>, Refusal> {
        let mut opened = locked(&self.opened);
        match &*opened {
            Some(store) => Ok(Arc::clone(store)),
            None => {
                let connection = self
                    .connection
                    .get()
                    .ok_or_else(|| Refusal::unavailable("not served"))?;
                let store = Arc::new((self.open)(connection)?);
                *opened = Some(Arc::clone(&store));
                Ok(store)
            }
        }
    }
}

impl<O: Store> Domain<O> {
    /// `open` gets the connection the owner serves on, the one to call other owners with, and is
    /// called again after it failed.
    pub fn new(open: impl Fn(&Connection) -> Result<O, Refusal> + Send + Sync + 'static) -> Self {
        let store = Arc::new(Lazy {
            open: Box::new(open),
            connection: OnceLock::new(),
            opened: Mutex::new(None),
        });
        let connected = Arc::clone(&store);
        Self {
            store,
            entries: BTreeMap::new(),
            things: BTreeMap::new(),
            files: None,
            connect: Box::new(move |connection| {
                connected.connection.get_or_init(|| connection.clone());
            }),
        }
    }

    fn entry<I: Intent>(&mut self, public: bool, returns: Option<Returns>) -> &mut Entry {
        let store = Arc::clone(&self.store);
        let entry = self
            .entries
            .entry(I::ID.to_owned())
            .or_insert_with(|| Entry {
                definition: I::definition(),
                public,
                finds: None,
                gets: None,
                preparer: None,
                store: Arc::new(move || Ok(store.get()? as Arc<dyn Store>)),
            });
        entry.definition.returns = returns;
        entry
    }

    /// Serves `org.bande.Intents.Files` for the things of this store.
    pub fn files(mut self) -> Self
    where
        O: Files,
    {
        let store = Arc::clone(&self.store);
        self.files = Some(Arc::new(move || Ok(store.get()? as Arc<dyn Files>)));
        self
    }

    /// Serves things of this type; `find` and `get` do it for theirs.
    pub fn thing<T: Thing>(mut self) -> Self
    where
        O: Describe<T>,
    {
        let store = Arc::clone(&self.store);
        self.things.insert(
            T::TYPE.to_owned(),
            ThingEntry {
                definition: T::definition(),
                references: T::TYPES,
                describe: Arc::new(move |reference, wanted, limits| {
                    let reference = Ref::<T>::parse(reference)
                        .ok_or_else(|| Refusal::not_found(reference.to_owned()))?;
                    let thing = store.get()?.describe(&reference, wanted, limits)?;
                    let foreign = limits.allows(domain(T::TYPE), Effect::ReadForeign);
                    thing_dict(thing, |name| wanted.has(name), foreign)
                }),
            },
        );
        self
    }

    /// Serves an intent that finds things. One whose arguments can cause more than reading is also
    /// added with [`Self::perform`], and its `find` refuses those arguments.
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
        let returns = Returns::List(kind.clone());
        self.entry::<I>(true, Some(returns)).finds = Some(Finds {
            kind,
            finder: Box::new(finder),
        });
        self.thing::<I::Thing>()
    }

    /// Serves an intent that gives properties of known things of the types `T`. Its first parameter
    /// is the [`crate::Refs`] or a [`Ref`]; without one it gives the thing `:only`. Then come
    /// `properties`, if any, and what [`Wanted::arguments`] tells [`Describe`].
    pub fn get<I: Intent, T: Things<O>>(mut self) -> Self {
        let returns = T::TYPES
            .first()
            .map(|kind| Returns::List((*kind).to_owned()));
        self.entry::<I>(true, returns).gets = Some(Gets {
            types: T::TYPES.to_vec(),
            check: Box::new(|arguments| I::decode(arguments).map(drop)),
        });
        T::add(self)
    }

    /// Serves an intent that does something.
    pub fn perform<I: Perform<O>>(mut self) -> Self {
        let preparer = self.preparer::<I>();
        self.entry::<I>(true, I::Output::returns()).preparer = Some(preparer);
        self
    }

    /// Adds an intent that only an [`Undo`] can name; no manifest lists it.
    pub fn private<I: Perform<O>>(mut self) -> Self {
        let preparer = self.preparer::<I>();
        self.entry::<I>(false, None).preparer = Some(preparer);
        self
    }

    fn preparer<I: Perform<O>>(&self) -> Box<Preparer> {
        let store = Arc::clone(&self.store);
        Box::new(move |arguments, limits| {
            let owner = store.get()?;
            let plan = I::decode(arguments)?.prepare(&owner, limits)?;
            let prepared = plan.prepared;
            Ok(Ready {
                effects: plan.effects,
                summary: plan.summary,
                rows: plan.rows,
                references: plan.references,
                run: Box::new(move |job, limits| {
                    let done = I::perform(prepared, &owner, job)?;
                    Ok((done.output.into_result(limits)?, done.undo))
                }),
            })
        })
    }
}
