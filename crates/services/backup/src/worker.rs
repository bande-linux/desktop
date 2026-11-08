use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command as Process, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rustix::process::{Pid, Signal, kill_process};
use zbus::blocking::Connection;

use crate::client::{Status, phase, problem, setup_error};
use crate::disk::{self, BROWSE_DIR, LOCAL_BROWSE_DIR, Space};
use crate::erase::erase;
use crate::local;
use crate::restic::{Exit, Finished, Repository, Snapshot, StopReason, Stopper, damaged_packs};
use crate::service::{DAY, HOUR, emit_status};
use crate::source::{self, SnapshotView};
use crate::store::{self, Destination, SecretFile, State};

const IDLE_EXIT: Duration = Duration::from_secs(60);
const EARLY_START: u64 = 5 * 60;
const PAUSE_LASTS: u64 = DAY;
const OVERDUE_AFTER: u64 = 14 * DAY;
const PRUNE_EVERY: u64 = 7 * DAY;
const CHECK_EVERY: u64 = DAY;
const CHECK_SLICES: u32 = 30;
const FORGET_DAMAGE_AFTER: u64 = 7 * DAY;
const FAILURES_BEFORE_TELLING: u32 = 3;
const MIB: u64 = 1024 * 1024;
const SPACE_ATTEMPTS: u32 = 16;
const TIME_TEMPLATE: &str = "2006-01-02T15:04:05";

pub enum Command {
    Wake,
    BackUpNow,
    Pause,
    Resume,
    Choose {
        destination: Destination,
        share_password: String,
        password: String,
    },
    EraseAndChoose {
        block: String,
        password: String,
    },
    SetExcluded(Vec<String>),
    Release(Sender<()>),
    StopUsing,
    OpenBrowser(String),
    CloseBrowser(String),
    ClientGone(String),
    BrowserReady,
    BrowserExited,
    JobFinished,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

#[derive(Default)]
struct Live {
    phase: String,
    percent: f64,
    remaining_seconds: u64,
    setup_error: String,
    space: Space,
    browse_root: String,
}

pub struct Shared {
    state: Mutex<State>,
    live: Mutex<Live>,
    pub stopper: Stopper,
    browsers: AtomicUsize,
    stay_on_battery: AtomicBool,
    bus: Mutex<Option<Connection>>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(store::load()),
            live: Mutex::new(Live::default()),
            stopper: Stopper::default(),
            browsers: AtomicUsize::new(0),
            stay_on_battery: AtomicBool::new(false),
            bus: Mutex::new(None),
        })
    }

    pub fn attach_bus(&self, bus: Connection) {
        *locked(&self.bus) = Some(bus);
    }

    pub fn status(&self) -> Status {
        let state = locked(&self.state);
        let live = locked(&self.live);
        let phase = if live.phase.is_empty() {
            match &state.destination {
                None => phase::NO_DISK,
                Some(_) => phase::IDLE,
            }
            .to_owned()
        } else {
            live.phase.clone()
        };
        Status {
            phase,
            problem: state.problem.clone(),
            setup_error: live.setup_error.clone(),
            disk_name: state
                .destination
                .as_ref()
                .map_or_else(String::new, |destination| destination.name().to_owned()),
            on_network: state
                .destination
                .as_ref()
                .is_some_and(Destination::is_network),
            excluded: state.excluded.clone(),
            last_success: state.last_success,
            paused_until: state.paused_until,
            oldest: state.oldest,
            versions: state.versions,
            percent: live.percent,
            remaining_seconds: live.remaining_seconds,
            disk_free: live.space.available,
            disk_total: live.space.total,
            on_battery: state.on_battery,
            browse_root: live.browse_root.clone(),
        }
    }

    fn publish(&self) {
        let (problem, name, since) = {
            let state = locked(&self.state);
            (
                state.problem.clone(),
                state
                    .destination
                    .as_ref()
                    .map_or_else(String::new, |destination| destination.name().to_owned()),
                state.problem_since,
            )
        };
        store::write_alert(&problem, &name, since);
        if let Some(bus) = locked(&self.bus).as_ref() {
            emit_status(bus);
        }
    }

    fn set_phase(&self, phase: &str) {
        {
            let mut live = locked(&self.live);
            live.phase = phase.to_owned();
            live.percent = 0.0;
            live.remaining_seconds = 0;
        }
        self.publish();
    }

    fn change(&self, change: impl FnOnce(&mut State)) {
        {
            let mut state = locked(&self.state);
            let problem_before = state.problem.clone();
            change(&mut state);
            if state.problem != problem_before {
                state.problem_since = now();
            }
            if let Err(error) = store::save(&state) {
                tracing::error!("cannot save the backup state: {error}");
            }
        }
        self.publish();
    }

    fn state(&self) -> State {
        locked(&self.state).clone()
    }

    pub fn set_on_battery(&self, allowed: bool) {
        self.change(|state| state.on_battery = allowed);
    }
}

struct Browser {
    child: Option<Child>,
    local: bool,
    clients: Vec<String>,
}

pub struct Worker {
    shared: Arc<Shared>,
    commands: Sender<Command>,
    busy: bool,
    pending: Option<Command>,
    run_again: bool,
    browser: Option<Browser>,
    releases: Vec<Sender<()>>,
}

impl Worker {
    pub fn new(shared: Arc<Shared>, commands: Sender<Command>) -> Self {
        Self {
            shared,
            commands,
            busy: false,
            pending: None,
            run_again: false,
            browser: None,
            releases: Vec::new(),
        }
    }

    pub fn run(mut self, commands: &Receiver<Command>) {
        self.reconcile(false);
        loop {
            let waited = if self.busy || self.browser.is_some() {
                commands.recv().map_err(|_| RecvTimeoutError::Disconnected)
            } else {
                commands.recv_timeout(IDLE_EXIT)
            };
            match waited {
                Ok(command) => self.handle(command),
                Err(_) => return,
            }
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Wake => self.reconcile(false),
            Command::BackUpNow => {
                self.shared.change(|state| state.paused_until = 0);
                self.run_now();
            }
            Command::Pause => {
                let until = now() + PAUSE_LASTS;
                self.run_again = false;
                self.shared.change(|state| state.paused_until = until);
                if self.busy {
                    self.shared.stopper.stop(StopReason::Paused);
                } else {
                    self.shared.set_phase(phase::PAUSED);
                }
            }
            Command::Resume => {
                self.shared.change(|state| state.paused_until = 0);
                self.run_now();
            }
            Command::SetExcluded(folders) => {
                self.shared.change(|state| state.excluded = folders);
            }
            Command::Release(done) => self.release(done),
            choose @ (Command::Choose { .. }
            | Command::EraseAndChoose { .. }
            | Command::StopUsing) => {
                if self.busy {
                    self.shared.stopper.stop(StopReason::Reconfigured);
                    self.pending = Some(choose);
                } else {
                    self.start_reconfiguring(choose);
                }
            }
            Command::OpenBrowser(client) => self.open_browser(client),
            Command::CloseBrowser(client) | Command::ClientGone(client) => {
                self.close_browser(&client);
            }
            Command::BrowserReady => {
                locked(&self.shared.live).browse_root = BROWSE_DIR.to_owned();
                self.shared.publish();
            }
            Command::BrowserExited => self.browser_exited(),
            Command::JobFinished => {
                self.busy = false;
                if self.browser.is_none() {
                    disk::unmount();
                    self.answer_releases();
                }
                if let Some(pending) = self.pending.take() {
                    self.start_reconfiguring(pending);
                } else if std::mem::take(&mut self.run_again) {
                    self.reconcile(true);
                }
            }
        }
    }

    fn release(&mut self, done: Sender<()>) {
        self.run_again = false;
        self.releases.push(done);
        if self.busy {
            self.shared.stopper.stop(StopReason::Released);
        }
        self.end_browser();
        if !self.busy && self.browser.is_none() {
            disk::unmount();
            self.answer_releases();
        }
    }

    fn answer_releases(&mut self) {
        for done in self.releases.drain(..) {
            let _ = done.send(());
        }
    }

    fn run_now(&mut self) {
        if self.busy {
            self.run_again = true;
        } else {
            self.reconcile(true);
        }
    }

    fn spawn_job(&mut self, job: impl FnOnce(&Shared) + Send + 'static) {
        self.busy = true;
        let shared = self.shared.clone();
        let commands = self.commands.clone();
        thread::spawn(move || {
            job(&shared);
            let _ = shared.stopper.take_reason();
            if commands.send(Command::JobFinished).is_err() {
                tracing::warn!("the backup service stopped before a job ended");
            }
        });
    }

    fn reconcile(&mut self, forced: bool) {
        if self.busy {
            return;
        }
        let browsing = self.browser.is_some();
        self.spawn_job(move |shared| back_up(shared, forced, browsing));
    }

    fn start_reconfiguring(&mut self, command: Command) {
        match command {
            Command::Choose {
                destination,
                share_password,
                password,
            } => {
                self.run_again = true;
                self.spawn_job(move |shared| {
                    choose(shared, &destination, &share_password, &password);
                });
            }
            Command::EraseAndChoose { block, password } => {
                self.run_again = true;
                self.spawn_job(move |shared| {
                    begin_setup(shared);
                    disk::unmount();
                    match erase(&block) {
                        Ok(destination) => choose(shared, &destination, "", &password),
                        Err(error) => {
                            tracing::error!("cannot erase the disk: {error}");
                            end_setup_with(shared, setup_error::ERASE_FAILED);
                        }
                    }
                });
            }
            Command::StopUsing => {
                self.spawn_job(move |shared| {
                    store::set_timer(false);
                    shared.change(|state| *state = State::default());
                    shared.set_phase(phase::NO_DISK);
                });
            }
            _ => {}
        }
    }

    fn open_browser(&mut self, client: String) {
        if let Some(browser) = &mut self.browser {
            browser.clients.push(client);
            return;
        }
        let state = self.shared.state();
        let Some(destination) = state.destination else {
            return;
        };
        let local = local::open_view();
        let child = match open_mount(&destination, self.commands.clone()) {
            Ok(child) => Some(child),
            Err(error) => {
                tracing::info!(
                    "the backups can't be browsed on {}: {error}",
                    destination.name()
                );
                None
            }
        };
        if child.is_none() && !local {
            self.shared.set_phase(phase::DISK_AWAY);
            return;
        }
        self.shared.browsers.fetch_add(1, Ordering::SeqCst);
        self.browser = Some(Browser {
            child,
            local,
            clients: vec![client],
        });
        if self
            .browser
            .as_ref()
            .is_some_and(|browser| browser.child.is_none())
        {
            locked(&self.shared.live).browse_root = LOCAL_BROWSE_DIR.to_owned();
            self.shared.publish();
        }
    }

    fn close_browser(&mut self, client: &str) {
        let Some(browser) = &mut self.browser else {
            return;
        };
        browser.clients.retain(|known| known != client);
        if browser.clients.is_empty() {
            self.end_browser();
        }
    }

    fn end_browser(&mut self) {
        let Some(browser) = &mut self.browser else {
            return;
        };
        browser.clients.clear();
        match &browser.child {
            Some(child) => {
                let target = i32::try_from(child.id()).ok().and_then(Pid::from_raw);
                if let Some(target) = target
                    && let Err(error) = kill_process(target, Signal::INT)
                {
                    tracing::warn!("cannot stop browsing the backups: {error}");
                }
            }
            None => self.browser_exited(),
        }
    }

    fn browser_exited(&mut self) {
        if let Some(browser) = self.browser.take() {
            if let Some(mut child) = browser.child
                && let Err(error) = child.wait()
            {
                tracing::warn!("cannot collect the backup browser: {error}");
            }
            if browser.local {
                local::close_view();
            }
            self.shared.browsers.fetch_sub(1, Ordering::SeqCst);
        }
        if let Err(error) = Process::new("umount")
            .args(["-l", BROWSE_DIR])
            .stderr(Stdio::null())
            .status()
        {
            tracing::debug!("{BROWSE_DIR} stays mounted: {error}");
        }
        locked(&self.shared.live).browse_root.clear();
        self.shared.publish();
        if !self.busy {
            disk::unmount();
            self.answer_releases();
        }
    }
}

fn host() -> io::Result<String> {
    store::machine_id()
}

fn network_environment(
    destination: &Destination,
    share_password: &str,
) -> Vec<(&'static str, String)> {
    let Destination::Network { user, .. } = destination else {
        return Vec::new();
    };
    if destination.is_rest() {
        vec![
            ("RESTIC_REST_USERNAME", user.clone()),
            ("RESTIC_REST_PASSWORD", share_password.to_owned()),
        ]
    } else {
        vec![("GODEBUG", "asyncpreemptoff=1".to_owned())]
    }
}

fn repository_on(destination: &Destination, mount_point: &Path) -> io::Result<Repository> {
    Ok(Repository::new(
        destination.repository_location(mount_point, &host()?),
        &store::key_path(),
    )
    .with_environment(network_environment(destination, &store::share_password())))
}

fn open_mount(destination: &Destination, commands: Sender<Command>) -> io::Result<Child> {
    if !destination.is_present() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the backup location is away",
        ));
    }
    let mount_point = destination.mount()?;
    let repository = repository_on(destination, &mount_point)?;
    fs::create_dir_all(BROWSE_DIR)?;
    let mut child = repository
        .process()
        .args([
            "mount",
            "--allow-other",
            "--path-template",
            "versions/%T",
            "--time-template",
            TIME_TEMPLATE,
            BROWSE_DIR,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("restic mount has no output"))?;
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.contains("Now serving") && commands.send(Command::BrowserReady).is_err() {
                return;
            }
        }
        if commands.send(Command::BrowserExited).is_err() {
            tracing::debug!("the backup service stopped before the browser");
        }
    });
    Ok(child)
}

fn begin_setup(shared: &Shared) {
    locked(&shared.live).setup_error.clear();
    shared.set_phase(phase::PREPARING);
}

fn end_setup_with(shared: &Shared, code: &str) {
    locked(&shared.live).setup_error = code.to_owned();
    let phase = if shared.state().destination.is_some() {
        phase::IDLE
    } else {
        phase::NO_DISK
    };
    shared.set_phase(phase);
}

fn choose(shared: &Shared, destination: &Destination, share_password: &str, password: &str) {
    begin_setup(shared);
    let outcome = prepare_repository(shared, destination, share_password, password);
    match outcome {
        Ok(()) => {
            shared.change(|state| {
                *state = State {
                    destination: Some(destination.clone()),
                    excluded: std::mem::take(&mut state.excluded),
                    chosen_at: now(),
                    ..State::default()
                };
            });
            store::set_timer(true);
            shared.set_phase(phase::IDLE);
        }
        Err(code) => end_setup_with(shared, code),
    }
}

fn prepare_repository(
    shared: &Shared,
    destination: &Destination,
    share_password: &str,
    password: &str,
) -> Result<(), &'static str> {
    let failed = |error: io::Error| {
        tracing::error!("cannot prepare the backup location: {error}");
        setup_error::FAILED
    };
    if !destination.is_present() {
        return Err(setup_error::DISK_GONE);
    }
    disk::unmount();
    if destination.is_network() {
        store::save_share_password(share_password).map_err(failed)?;
    }
    let mount_point = destination.mount().map_err(failed)?;
    let key = store::ensure_key().map_err(failed)?;
    let id = host().map_err(failed)?;
    let path = destination.repository_location(&mount_point, &id);
    let secret = SecretFile::new(password).map_err(failed)?;
    let environment = network_environment(destination, share_password);
    let by_person =
        Repository::new(path.clone(), secret.path()).with_environment(environment.clone());
    let by_machine = Repository::new(path.clone(), &key).with_environment(environment);
    let stopper = &shared.stopper;
    let exists = match by_person.opens(stopper).map_err(failed)? {
        Exit::Done => true,
        Exit::WrongPassword => return Err(setup_error::WRONG_PASSWORD),
        Exit::NoRepository => false,
        other => {
            tracing::error!("restic cannot open the repository: {other:?}");
            return Err(setup_error::FAILED);
        }
    };
    if exists {
        if by_machine.opens(stopper).map_err(failed)? != Exit::Done {
            add_key(&by_person, stopper, &key).map_err(failed)?;
        }
    } else {
        if !destination.is_rest()
            && let Some(parent) = path.parent()
        {
            fs::create_dir_all(parent).map_err(failed)?;
        }
        let created = by_person
            .run(stopper, &["init", "--repository-version", "2"])
            .map_err(failed)?;
        if created.exit != Exit::Done {
            tracing::error!("restic init: {}", created.output);
            return Err(setup_error::FAILED);
        }
        if let Err(error) = add_key(&by_person, stopper, &key) {
            if !destination.is_rest()
                && let Err(removing) = fs::remove_dir_all(&path)
            {
                tracing::error!("cannot remove the new repository: {removing}");
            }
            return Err(failed(error));
        }
    }
    if !destination.is_rest()
        && let Some(parent) = path.parent()
    {
        let label = parent.join(format!("{id}.name"));
        if let Ok(name) = fs::read_to_string("/etc/hostname")
            && let Err(error) = fs::write(&label, name.trim())
        {
            tracing::warn!("cannot name the backups on the disk: {error}");
        }
    }
    Ok(())
}

fn add_key(by_person: &Repository, stopper: &Stopper, key: &Path) -> io::Result<()> {
    let added = by_person.run(
        stopper,
        &[
            "key".as_ref(),
            "add".as_ref(),
            "--new-password-file".as_ref(),
            key.as_os_str(),
        ],
    )?;
    if added.exit == Exit::Done {
        Ok(())
    } else {
        Err(io::Error::other(added.output))
    }
}

fn note_overdue(shared: &Shared, state: &State) {
    let since = state.last_success.max(state.chosen_at);
    if since > 0 && now().saturating_sub(since) > OVERDUE_AFTER && state.problem.is_empty() {
        shared.change(|state| state.problem = problem::OVERDUE.to_owned());
    }
}

#[zbus::proxy(
    interface = "org.freedesktop.UPower",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower",
    gen_async = false
)]
trait UPower {
    #[zbus(property)]
    fn on_battery(&self) -> zbus::Result<bool>;
}

pub fn stop_when_unplugged(shared: Arc<Shared>) {
    thread::spawn(move || {
        let Ok(system) = Connection::system() else {
            return;
        };
        let Ok(upower) = UPowerProxy::new(&system) else {
            return;
        };
        for change in upower.receive_on_battery_changed() {
            if change.get().unwrap_or(false)
                && !shared.state().on_battery
                && !shared.stay_on_battery.load(Ordering::SeqCst)
            {
                shared.stopper.stop(StopReason::OnBattery);
            }
        }
    });
}

fn on_battery() -> bool {
    Connection::system()
        .and_then(|system| UPowerProxy::new(&system)?.on_battery())
        .unwrap_or(false)
}

fn back_up(shared: &Shared, forced: bool, browsing: bool) {
    shared.stay_on_battery.store(forced, Ordering::SeqCst);
    let state = shared.state();
    let Some(destination) = state.destination.clone() else {
        shared.set_phase(phase::NO_DISK);
        return;
    };
    if state.paused_until > now() {
        local::take_hourly();
        shared.set_phase(phase::PAUSED);
        note_overdue(shared, &state);
        return;
    }
    if state.paused_until > 0 {
        shared.change(|state| state.paused_until = 0);
    }
    if !destination.is_present() {
        local::take_hourly();
        shared.set_phase(phase::DISK_AWAY);
        note_overdue(shared, &state);
        return;
    }
    if !forced && !state.on_battery && on_battery() {
        local::take_hourly();
        shared.set_phase(phase::WAITING_FOR_POWER);
        note_overdue(shared, &state);
        return;
    }
    let waiting_for_room = state.problem == problem::DISK_FULL && state.full_at + DAY > now();
    if !forced && (waiting_for_room || state.last_success + HOUR > now() + EARLY_START) {
        shared.set_phase(phase::IDLE);
        return;
    }
    let mount_point = match destination.mount() {
        Ok(mount_point) => mount_point,
        Err(error) => {
            tracing::warn!("cannot reach the backup location: {error}");
            shared.set_phase(if destination.is_present() {
                phase::IDLE
            } else {
                phase::DISK_AWAY
            });
            note_failure(shared);
            return;
        }
    };
    let repository = match repository_on(&destination, &mount_point) {
        Ok(repository) => repository,
        Err(error) => {
            tracing::error!("cannot find this computer's identity: {error}");
            shared.set_phase(phase::IDLE);
            note_failure(shared);
            return;
        }
    };
    let mut run = Run {
        shared,
        destination: &destination,
        mount_point: &mount_point,
        repository,
        browsing,
    };
    run.go();
}

struct Run<'a> {
    shared: &'a Shared,
    destination: &'a Destination,
    mount_point: &'a Path,
    repository: Repository,
    browsing: bool,
}

enum Saved {
    Yes,
    No,
}

impl Run<'_> {
    fn stopper(&self) -> &Stopper {
        &self.shared.stopper
    }

    fn exclusive_allowed(&self) -> bool {
        self.shared.browsers.load(Ordering::SeqCst) == 0
    }

    fn go(&mut self) {
        match self.back_up_with_room() {
            Saved::Yes => {
                self.tidy();
                self.verify();
                self.count_versions();
                if !matches!(
                    self.shared.status().phase.as_str(),
                    phase::PAUSED | phase::DISK_AWAY | phase::WAITING_FOR_POWER
                ) {
                    self.shared.set_phase(phase::IDLE);
                }
            }
            Saved::No => {}
        }
    }

    fn refresh_space(&self) -> Space {
        if self.destination.is_rest() {
            return Space::default();
        }
        let space = disk::space(self.mount_point).unwrap_or_default();
        locked(&self.shared.live).space = space;
        space
    }

    fn clear_locks(&self) {
        let args: &[&str] = if self.browsing {
            &["unlock"]
        } else {
            &["unlock", "--remove-all"]
        };
        if let Err(error) = self.repository.run(self.stopper(), args) {
            tracing::warn!("cannot clear old locks: {error}");
        }
    }

    fn back_up_with_room(&self) -> Saved {
        self.clear_locks();
        self.recover_interrupted_run();
        for _ in 0..SPACE_ATTEMPTS {
            let space = self.refresh_space();
            let reserve = disk::reserved_for(space.total);
            if space.available < reserve
                && !self.make_room()
                && self.refresh_space().available < reserve
            {
                return self.report_full();
            }
            match self.attempt(reserve) {
                Attempt::Saved => return Saved::Yes,
                Attempt::Stopped(reason) => {
                    self.note_stop(reason);
                    return Saved::No;
                }
                Attempt::OutOfRoom => {
                    if !self.make_room() {
                        return self.report_full();
                    }
                }
                Attempt::CannotOpen => {
                    self.shared.set_phase(phase::IDLE);
                    self.shared
                        .change(|state| state.problem = problem::CANNOT_OPEN.to_owned());
                    return Saved::No;
                }
                Attempt::Failed => {
                    self.shared.set_phase(phase::IDLE);
                    note_failure(self.shared);
                    return Saved::No;
                }
            }
        }
        tracing::error!("the backup did not fit after removing old versions");
        self.shared.set_phase(phase::IDLE);
        note_failure(self.shared);
        Saved::No
    }

    fn recover_interrupted_run(&self) {
        if !self.shared.state().interrupted || !self.exclusive_allowed() {
            return;
        }
        match self.repository.run(self.stopper(), &["repair", "index"]) {
            Ok(finished) if finished.exit == Exit::Done => {}
            Ok(finished) => tracing::warn!("restic repair index: {}", finished.output),
            Err(error) => tracing::warn!("cannot pick up the interrupted backup: {error}"),
        }
    }

    fn note_stop(&self, reason: StopReason) {
        match reason {
            StopReason::Paused => self.shared.set_phase(phase::PAUSED),
            StopReason::DiskGone | StopReason::Released => {
                self.shared.set_phase(phase::DISK_AWAY);
                note_overdue(self.shared, &self.shared.state());
            }
            StopReason::OnBattery => self.shared.set_phase(phase::WAITING_FOR_POWER),
            StopReason::NoSpace | StopReason::Reconfigured => self.shared.set_phase(phase::IDLE),
        }
    }

    fn attempt(&self, reserve: u64) -> Attempt {
        self.shared.set_phase(phase::BACKING_UP);
        self.shared.change(|state| state.interrupted = true);
        let Ok(id) = host() else {
            return Attempt::Failed;
        };
        let sources = source::sources();
        let args = source::backup_args(&id, &sources, &self.shared.state().excluded);
        let destination = self.destination;
        let view = SnapshotView::take(&sources);
        let reader = match &view {
            Some(view) => self.repository.inside(view.wrapper()),
            None => self.repository.clone(),
        };
        let finished = reader.follow_progress(self.stopper(), &args, |progress| {
            if self.stopper().is_stopping() {
                return;
            }
            if !destination.is_present() {
                self.stopper().stop(StopReason::DiskGone);
                self.shared.set_phase(phase::DISK_AWAY);
                if destination.is_network() {
                    disk::unmount();
                }
                return;
            }
            let space = self.refresh_space();
            if space.available < reserve {
                self.stopper().stop(StopReason::NoSpace);
            }
            let mut live = locked(&self.shared.live);
            live.percent = progress.percent_done;
            live.remaining_seconds = progress.seconds_remaining;
            drop(live);
            self.shared.publish();
        });
        let reason = self.stopper().take_reason();
        match reason {
            Some(StopReason::NoSpace) => return Attempt::OutOfRoom,
            Some(other) => return Attempt::Stopped(other),
            None => {}
        }
        let finished = match finished {
            Ok(finished) => finished,
            Err(error) => {
                tracing::error!("cannot run restic: {error}");
                return Attempt::Failed;
            }
        };
        self.classify(finished)
    }

    fn classify(&self, finished: Finished) -> Attempt {
        if finished.exit.saved_a_snapshot() {
            self.shared.change(|state| {
                state.last_success = now();
                state.failures_in_a_row = 0;
                state.interrupted = false;
                let forget_damage = state.problem == problem::DAMAGED
                    && now().saturating_sub(state.damaged_at) > FORGET_DAMAGE_AFTER;
                if state.problem != problem::DAMAGED || forget_damage {
                    state.problem.clear();
                }
            });
            return Attempt::Saved;
        }
        if !self.destination.is_present() {
            return Attempt::Stopped(StopReason::DiskGone);
        }
        match finished.exit {
            Exit::NoRepository | Exit::WrongPassword => Attempt::CannotOpen,
            _ if finished.output.contains("no space left on device") => Attempt::OutOfRoom,
            _ => {
                tracing::error!("restic backup failed: {}", finished.output);
                Attempt::Failed
            }
        }
    }

    fn snapshots(&self) -> Vec<Snapshot> {
        let Ok(id) = host() else {
            return Vec::new();
        };
        self.repository
            .snapshots(self.stopper(), &id)
            .unwrap_or_else(|error| {
                tracing::warn!("cannot list the backups: {error}");
                Vec::new()
            })
    }

    fn make_room(&self) -> bool {
        if !self.exclusive_allowed() {
            return false;
        }
        self.shared.set_phase(phase::TIDYING);
        let snapshots = self.snapshots();
        let removable = snapshots.len().saturating_sub(1);
        if removable > 0 {
            let oldest = removable.div_ceil(4);
            let mut args = vec!["forget".to_owned()];
            args.extend(
                snapshots
                    .iter()
                    .take(oldest)
                    .map(|snapshot| snapshot.id.clone()),
            );
            if let Err(error) = self.repository.run(self.stopper(), &args) {
                tracing::warn!("cannot forget old backups: {error}");
                return false;
            }
        }
        if let Err(error) = self
            .repository
            .run(self.stopper(), &["prune", "--max-unused", "unlimited"])
        {
            tracing::warn!("cannot prune the backups: {error}");
            return false;
        }
        self.refresh_space();
        removable > 0
    }

    fn report_full(&self) -> Saved {
        tracing::warn!("the backup disk is too small");
        self.shared.set_phase(phase::IDLE);
        self.shared.change(|state| {
            state.problem = problem::DISK_FULL.to_owned();
            state.full_at = now();
        });
        Saved::No
    }

    fn tidy(&self) {
        if !self.exclusive_allowed() {
            return;
        }
        let Ok(id) = host() else {
            return;
        };
        self.shared.set_phase(phase::TIDYING);
        let forgot = self.repository.run(
            self.stopper(),
            &[
                "forget",
                "--host",
                &id,
                "--group-by",
                "host",
                "--keep-within-hourly",
                "24h",
                "--keep-within-daily",
                "30d",
                "--keep-weekly",
                "-1",
                "--keep-last",
                "1",
            ],
        );
        if let Err(error) = forgot {
            tracing::warn!("cannot forget old backups: {error}");
            return;
        }
        if now().saturating_sub(self.shared.state().last_prune) < PRUNE_EVERY {
            return;
        }
        let room = self.refresh_space().available / MIB / 2;
        let pruned = self.repository.run(
            self.stopper(),
            &[
                "prune".to_owned(),
                "--max-unused".to_owned(),
                "5%".to_owned(),
                "--max-repack-size".to_owned(),
                format!("{room}M"),
            ],
        );
        match pruned {
            Ok(finished) if finished.exit == Exit::Done => {
                self.shared.change(|state| state.last_prune = now());
            }
            Ok(finished) => tracing::warn!("restic prune: {}", finished.output),
            Err(error) => tracing::warn!("cannot prune the backups: {error}"),
        }
    }

    fn verify(&self) {
        let state = self.shared.state();
        if !self.exclusive_allowed() || now().saturating_sub(state.last_check) < CHECK_EVERY {
            return;
        }
        self.shared.set_phase(phase::VERIFYING);
        let slice = state.check_slice % CHECK_SLICES + 1;
        let subset = format!("--read-data-subset={slice}/{CHECK_SLICES}");
        let checked = self
            .repository
            .run(self.stopper(), &["check", subset.as_str()]);
        let finished = match checked {
            Ok(finished) => finished,
            Err(error) => {
                tracing::warn!("cannot verify the backups: {error}");
                return;
            }
        };
        match finished.exit {
            Exit::Done => self.shared.change(|state| {
                state.last_check = now();
                state.check_slice = slice;
            }),
            Exit::Failed if self.destination.is_present() && !self.stopper().is_stopping() => {
                tracing::error!("the backups are damaged: {}", finished.output);
                self.heal(&finished.output);
                self.check_everything_after_damage();
                self.shared.change(|state| state.last_check = now());
            }
            other => tracing::warn!("restic check: {other:?} {}", finished.output),
        }
    }

    fn check_everything_after_damage(&self) {
        match self
            .repository
            .run(self.stopper(), &["check", "--read-data"])
        {
            Ok(finished) if finished.exit == Exit::Failed && !self.stopper().is_stopping() => {
                self.heal(&finished.output);
            }
            Ok(_) => {}
            Err(error) => tracing::warn!("cannot verify the backups: {error}"),
        }
    }

    fn heal(&self, check_output: &str) {
        let damaged = damaged_packs(check_output);
        self.shared.change(|state| {
            state.problem = problem::DAMAGED.to_owned();
            state.damaged_at = now();
        });
        if !damaged.is_empty() {
            let mut args = vec!["repair".to_owned(), "packs".to_owned()];
            args.extend(damaged);
            if let Err(error) = self.repository.run(self.stopper(), &args) {
                tracing::error!("cannot repair the damaged data: {error}");
                return;
            }
        }
        for args in [
            ["repair", "index"].as_slice(),
            ["repair", "snapshots", "--forget"].as_slice(),
        ] {
            match self.repository.run(self.stopper(), args) {
                Ok(finished) if finished.exit == Exit::Done => {}
                Ok(finished) => tracing::error!("restic repair: {}", finished.output),
                Err(error) => tracing::error!("cannot repair the backups: {error}"),
            }
        }
    }

    fn count_versions(&self) {
        let snapshots = self.snapshots();
        let oldest = snapshots.first().map_or(0, Snapshot::seconds);
        let count = u32::try_from(snapshots.len()).unwrap_or(u32::MAX);
        self.shared.change(|state| {
            state.oldest = oldest;
            state.versions = count;
        });
    }
}

enum Attempt {
    Saved,
    Stopped(StopReason),
    OutOfRoom,
    CannotOpen,
    Failed,
}

fn note_failure(shared: &Shared) {
    shared.change(|state| {
        state.failures_in_a_row += 1;
        if state.failures_in_a_row >= FAILURES_BEFORE_TELLING && state.problem.is_empty() {
            state.problem = problem::FAILING.to_owned();
        }
    });
}
