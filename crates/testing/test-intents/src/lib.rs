#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! What every owner of Bande Intents must stand: ends at any moment, a full disk, callers that
//! leave, plans of others, limits, and being idle.

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use bande_intents::{Client, Dict, Effect, Manifests, Prepared, Reason, Refusal};
use bande_test_bus::SessionBus;
use tempfile::TempDir;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedValue, Value};

const STARTED: Duration = Duration::from_secs(10);
const IDLE: Duration = Duration::from_secs(1);
const STARTING_TICKS: u64 = 10;
const OWN_MEMORY: u64 = 10 * 1024;
const PLANS: usize = 16;
const CRASH_POINTS: [&str; 3] = ["performing", "kept", "performed"];

/// An owner's program on a private bus, with a folder for its receipts and its manifest.
pub struct Owned {
    pub bus: SessionBus,
    pub manifests: Manifests,
    program: PathBuf,
    name: String,
    state: TempDir,
    running: Option<Child>,
}

fn owned(value: impl Into<Value<'static>>) -> OwnedValue {
    value.into().try_into().expect("a value")
}

impl Owned {
    /// Writes the manifest of `program`, and starts it.
    pub fn start(program: impl Into<PathBuf>) -> Self {
        let program = program.into();
        let folder = tempfile::tempdir().expect("a folder");
        let written = Command::new(&program)
            .arg("--write-intents")
            .arg(folder.path())
            .status()
            .expect("the owner");
        assert!(written.success(), "the manifest is written");
        let manifests = Manifests::read(folder.path());
        let name = manifests.iter().next().expect("a manifest").bus.clone();
        let mut owned = Self {
            bus: SessionBus::start().expect("dbus-daemon"),
            manifests,
            program,
            name,
            state: tempfile::tempdir().expect("a folder"),
            running: None,
        };
        owned.run(None);
        owned
    }

    fn run(&mut self, crash: Option<&str>) {
        let mut command = Command::new(&self.program);
        command
            .env("DBUS_SESSION_BUS_ADDRESS", self.bus.address())
            .env("XDG_STATE_HOME", self.state.path())
            .env_remove("BANDE_INTENTS_CRASH");
        if let Some(point) = crash {
            command.env("BANDE_INTENTS_CRASH", point);
        }
        self.running = Some(command.spawn().expect("the owner"));
        self.wait_for_name(true);
    }

    fn wait_for_name(&self, there: bool) {
        let connection = self.connect();
        let proxy = zbus::blocking::fdo::DBusProxy::new(&connection).expect("the bus");
        let until = Instant::now() + STARTED;
        while proxy
            .name_has_owner(self.name.as_str().try_into().expect("a name"))
            .expect("an answer")
            != there
        {
            assert!(Instant::now() < until, "the owner starts and ends");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn kill(&mut self) {
        if let Some(mut running) = self.running.take() {
            let _ = running.kill();
            let _ = running.wait();
        }
        self.wait_for_name(false);
    }

    /// Ends the program at once and starts it again.
    pub fn restart(&mut self) {
        self.kill();
        self.run(None);
    }

    pub fn connect(&self) -> Connection {
        self.bus.connect().expect("a connection")
    }

    pub fn client(&self) -> Client {
        Client::new(&self.connect())
    }

    /// The folder the program keeps its receipts in.
    pub fn receipts(&self) -> PathBuf {
        self.state.path().join("bande/intents")
    }

    /// Where to find the owner.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The arguments `words` make, in the order of the intent's parameters.
    pub fn arguments(&self, intent: &str, words: &[&str]) -> Dict {
        let (_, definition) = self.manifests.intent(intent).expect("the intent");
        definition
            .parameters
            .iter()
            .zip(words)
            .map(|(parameter, word)| {
                let value = parameter.kind.from_words(&[(*word).to_owned()]);
                (parameter.name.clone(), value.expect("an argument"))
            })
            .collect()
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        if let Some(running) = &mut self.running {
            let _ = running.kill();
            let _ = running.wait();
        }
    }
}

/// One change an owner can make.
pub struct Change<'a> {
    /// A `find` intent that finds something without arguments.
    pub find: &'a str,
    /// An intent with the effect `change` and an undo, and its arguments.
    pub intent: &'a str,
    pub words: &'a [&'a str],
}

fn limits(entries: [(&str, Value<'static>); 1]) -> Dict {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), owned(value)))
        .collect()
}

fn granted(domain: &str, effects: &[&str], within: Option<Vec<String>>) -> Dict {
    let mut grant = HashMap::from([(
        "effects",
        Value::from(
            effects
                .iter()
                .map(|word| (*word).to_owned())
                .collect::<Vec<_>>(),
        ),
    )]);
    if let Some(within) = within {
        grant.insert("within", Value::from(within));
    }
    limits([(
        "domains",
        Value::from(HashMap::from([(domain.to_owned(), grant)])),
    )])
}

fn reason<T: std::fmt::Debug>(answer: Result<T, Refusal>) -> Reason {
    answer.expect_err("a refusal").reason
}

/// Runs everything an owner must stand. `serve` starts what the owner needs on its bus, and gives
/// the state the change changes, read past the owner.
pub fn stands<S: Fn() -> String>(
    program: &Path,
    serve: impl FnOnce(&SessionBus) -> S,
    change: &Change<'_>,
) {
    let mut owned = Owned::start(program);
    let state = serve(&owned.bus);
    let state: &dyn Fn() -> String = &state;
    let before = state();
    rests_when_idle(&owned, change);
    takes_only_whole_changes_through_crashes(&mut owned, change, state, &before);
    ends_plans_with_their_callers(&mut owned, change, state, &before);
    keeps_to_limits(&owned, change);
    reads_past_bad_receipts(&mut owned, change);
    does_without_undo_only_when_told(&mut owned, change, state, &before);
}

fn prepare(owned: &Owned, client: &Client, change: &Change<'_>) -> Result<Prepared, Refusal> {
    let arguments = owned.arguments(change.intent, change.words);
    client.prepare(owned.name(), change.intent, &arguments)
}

fn undo_all(owned: &Owned) {
    let client = owned.client();
    for receipt in client.all_receipts(&owned.manifests) {
        client
            .undo(&owned.manifests, &receipt.receipt)
            .expect("an undo");
    }
}

fn own_memory(process: u32) -> u64 {
    fs::read_to_string(format!("/proc/{process}/smaps_rollup"))
        .expect("memory")
        .lines()
        .find_map(|line| line.strip_prefix("Anonymous:"))
        .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
        .expect("a number")
}

fn wakes(process: u32) -> u64 {
    fs::read_dir(format!("/proc/{process}/task"))
        .expect("threads")
        .filter_map(|thread| fs::read_to_string(thread.ok()?.path().join("status")).ok())
        .flat_map(|status| {
            status
                .lines()
                .filter_map(|line| line.strip_prefix("voluntary_ctxt_switches:"))
                .filter_map(|count| count.trim().parse::<u64>().ok())
                .collect::<Vec<_>>()
        })
        .sum()
}

fn ticks(process: u32) -> u64 {
    let stat = fs::read_to_string(format!("/proc/{process}/stat")).expect("times");
    let fields: Vec<&str> = stat
        .rsplit(')')
        .next()
        .expect("fields")
        .split_whitespace()
        .collect();
    fields[11].parse::<u64>().expect("a time") + fields[12].parse::<u64>().expect("a time")
}

fn rests_when_idle(owned: &Owned, change: &Change<'_>) {
    let client = owned.client();
    let found = client
        .find(owned.name(), change.find, &Dict::new(), "", 1)
        .expect("things");
    assert!(!found.things.is_empty(), "{} finds something", change.find);
    let process = owned.running.as_ref().expect("the owner").id();
    assert!(
        ticks(process) <= STARTING_TICKS,
        "starting and answering took {} ticks",
        ticks(process)
    );
    std::thread::sleep(IDLE);
    let woken = wakes(process);
    std::thread::sleep(IDLE);
    assert_eq!(wakes(process), woken, "an idle owner does not wake");
    let memory = own_memory(process);
    assert!(memory <= OWN_MEMORY, "an idle owner holds {memory} kB");
}

fn takes_only_whole_changes_through_crashes(
    owned: &mut Owned,
    change: &Change<'_>,
    state: &dyn Fn() -> String,
    before: &str,
) {
    let client = owned.client();
    let prepared = prepare(owned, &client, change).expect("a plan");
    owned.restart();
    assert_eq!(reason(client.perform(prepared)), Reason::Expired);
    assert_eq!(state(), before, "a plan is lost with its owner");
    for point in CRASH_POINTS {
        owned.kill();
        owned.run(Some(point));
        let prepared = prepare(owned, &client, change).expect("a plan");
        assert_eq!(reason(client.perform(prepared)), Reason::Interrupted);
        owned.kill();
        owned.run(None);
        let receipts = owned.client().all_receipts(&owned.manifests);
        let changed = state() != before;
        assert_eq!(
            receipts.len(),
            usize::from(changed),
            "ended at “{point}”, it has a receipt if it was done: {receipts:?}"
        );
        undo_all(owned);
        assert_eq!(state(), before, "ended at “{point}” and undone");
    }
}

fn ends_plans_with_their_callers(
    owned: &mut Owned,
    change: &Change<'_>,
    state: &dyn Fn() -> String,
    before: &str,
) {
    let (first, second) = (owned.client(), owned.client());
    let prepared = prepare(owned, &first, change).expect("a plan");
    assert_eq!(reason(second.perform(prepared)), Reason::AccessDenied);
    let gone = owned.client();
    let prepared = prepare(owned, &gone, change).expect("a plan");
    drop(gone);
    let refused = reason(second.perform(prepared));
    assert!(matches!(refused, Reason::Expired | Reason::AccessDenied));
    assert_eq!(state(), before, "no other program performs a plan");
    let many = owned.client();
    let plans: Vec<_> = (0..PLANS)
        .map(|_| prepare(owned, &many, change).expect("a plan"))
        .collect();
    assert_eq!(reason(prepare(owned, &many, change)), Reason::Limited);
    drop(plans);
}

fn keeps_to_limits(owned: &Owned, change: &Change<'_>) {
    let domain = bande_intents::domain(change.intent);
    let limited = |limits: Dict| Client::new(&owned.connect()).limited(limits);
    let find = |client: &Client| client.find(owned.name(), change.find, &Dict::new(), "", 1);
    let elsewhere = limited(granted("elsewhere", &["read", "change"], None));
    assert_eq!(reason(find(&elsewhere)), Reason::NotGranted);
    assert_eq!(
        reason(prepare(owned, &elsewhere, change)),
        Reason::NotGranted
    );
    let reads = limited(granted(domain, &["read"], None));
    assert!(!find(&reads).expect("things").things.is_empty());
    assert_eq!(reason(prepare(owned, &reads, change)), Reason::NotGranted);
    let nothing = limited(granted(domain, &["read", "change"], Some(Vec::new())));
    assert!(find(&nothing).expect("no things").things.is_empty());
    let refused = reason(prepare(owned, &nothing, change));
    assert!(matches!(
        refused,
        Reason::NotFound | Reason::InvalidArgument
    ));
    let odd = limited(limits([("anything", Value::from(true))]));
    assert_eq!(reason(find(&odd)), Reason::InvalidArgument);
    let posing = limited(limits([("agent", Value::from("Organiser"))]));
    assert_eq!(reason(find(&posing)), Reason::AccessDenied);
}

fn reads_past_bad_receipts(owned: &mut Owned, change: &Change<'_>) {
    owned.kill();
    let domain = bande_intents::domain(change.intent);
    let path = owned.receipts().join(format!("{domain}.json"));
    for bad in ["[{\"made\": \"long ago\"}]", "{", ""] {
        fs::write(&path, bad).expect("a bad receipt");
        owned.run(None);
        assert_eq!(owned.client().all_receipts(&owned.manifests), []);
        owned.kill();
    }
    owned.run(None);
}

fn does_without_undo_only_when_told(
    owned: &mut Owned,
    change: &Change<'_>,
    state: &dyn Fn() -> String,
    before: &str,
) {
    owned.kill();
    let folder = owned.receipts();
    let set = |mode: u32| {
        fs::set_permissions(&folder, fs::Permissions::from_mode(mode)).expect("permissions");
    };
    set(0o500);
    owned.run(None);
    let agents = owned.connect();
    agents.request_name("org.bande.Agents").expect("a name");
    let agent = Client::new(&agents).limited(limits([("agent", Value::from("Organiser"))]));
    assert_eq!(reason(prepare(owned, &agent, change)), Reason::Unavailable);
    let client = owned.client();
    let prepared = prepare(owned, &client, change).expect("a plan");
    assert!(
        prepared
            .rows
            .iter()
            .any(|row| bande_intents::client::text(row, "kind") == "loss"),
        "the plan says that it cannot be undone: {:?}",
        prepared.rows
    );
    assert!(!prepared.effects.contains(&Effect::Critical));
    let performed = client.perform(prepared).expect("the change");
    assert_eq!(performed.receipt(), None);
    assert_ne!(state(), before, "the person’s own change is made");
    set(0o700);
}
