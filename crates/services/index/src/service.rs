use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use bande_search::{
    CACHE_TAG, Donation, Found, Policy, Query, ROOT, Stamp, Tree, Usage, follow_policy,
    is_excluded, skipped_name, usage_path,
};
use bande_watch::{Changes, FolderWatch};

use crate::background::{Background, Child, Donated, Edit, Job, Reply, admitted, inspect};
use crate::bus;
use crate::store::{Entry, Hit, Store};

const SETTLE: Duration = Duration::from_millis(300);
const FIRST_SAVE: Duration = Duration::from_secs(2);
const SAVE_SPACING: Duration = Duration::from_secs(10);
const LOW_MEMORY_RETRY: Duration = Duration::from_secs(60);
const LOW_MEMORY_BYTES: u64 = 512 << 20;
const EDITS_PER_BATCH: usize = 2048;
const CONTENT_SCORE: u32 = 500;
const CONTENT_RELEVANCE_RANGE: f32 = 90.0;
const OVERFETCH: usize = 8;
const DOMAIN_SEPARATOR: char = '\u{1f}';

pub enum Message {
    Find {
        text: String,
        within: String,
        limit: usize,
        reply: Sender<Vec<Found>>,
    },
    Donate {
        domain: String,
        items: Vec<Donation>,
        cursor: String,
    },
    Cursor {
        domain: String,
        reply: Sender<String>,
    },
    ForgetAll {
        domain: String,
    },
    Forget {
        domain: String,
        ids: Vec<String>,
    },
    Changed(Vec<PathBuf>),
    Overflowed,
    Policy(Policy),
    Worked(Reply),
}

#[derive(Default)]
struct Reconcile {
    by_inode: HashMap<u64, u32>,
    vanished: Vec<u32>,
    walks: usize,
}

struct Service {
    home: PathBuf,
    store: Store,
    tree: Tree,
    policy: Policy,
    jobs: Sender<Job>,
    busy: bool,
    urgent: VecDeque<Job>,
    backlog: VecDeque<Job>,
    edits: Vec<Edit>,
    unsaved: bool,
    purge: bool,
    writer_open: bool,
    reconcile: Option<Reconcile>,
    changed: BTreeSet<PathBuf>,
    settle_at: Option<Instant>,
    save_at: Option<Instant>,
    saved_at: Option<Instant>,
    paused_until: Option<Instant>,
    usage_file: Option<PathBuf>,
    usage: Option<(SystemTime, Usage)>,
    cursors: HashMap<String, String>,
    policy_known: bool,
    root_mount: Option<u64>,
}

pub fn run(home: PathBuf, folder: PathBuf) -> io::Result<()> {
    let store = Store::open(&folder).map_err(io::Error::other)?;
    let (messages, received) = channel();
    let changes = messages.clone();
    let overflows = messages.clone();
    let watcher = FolderWatch::spawn_noting_overflow(
        Changes::Any,
        move |paths| {
            let _ = changes.send(Message::Changed(paths));
        },
        move || {
            let _ = overflows.send(Message::Overflowed);
        },
    )
    .inspect_err(|error| tracing::warn!("cannot watch the home folder: {error}"))
    .ok()
    .map(Arc::new);
    let (jobs, work) = channel();
    let (done, replies) = channel();
    let root_mount = inspect(&home).map(|inspected| inspected.mount);
    let background = Background {
        index: store.index.clone(),
        reader: store.reader(),
        fields: store.fields,
        root_mount,
        watcher,
        writer: None,
        written: 0,
        watch_failed: false,
    };
    thread::Builder::new()
        .name("index-background".to_owned())
        .spawn(move || background.run(work, done))?;
    let worked = messages.clone();
    thread::Builder::new()
        .name("index-replies".to_owned())
        .spawn(move || {
            while let Ok(reply) = replies.recv() {
                if worked.send(Message::Worked(reply)).is_err() {
                    return;
                }
            }
        })?;
    let (restored, orphans) = Tree::restored(store.entries().map_err(io::Error::other)?);
    let mut service = Service {
        usage_file: usage_path(bande_xdg::state_home().as_deref(), Some(&home)),
        home: home.clone(),
        store,
        tree: restored,
        policy: Policy::default(),
        jobs,
        busy: false,
        urgent: VecDeque::new(),
        backlog: VecDeque::new(),
        edits: orphans.into_iter().map(Edit::Remove).collect(),
        unsaved: false,
        purge: false,
        writer_open: false,
        reconcile: None,
        changed: BTreeSet::new(),
        settle_at: None,
        save_at: None,
        saved_at: None,
        paused_until: None,
        usage: None,
        cursors: HashMap::new(),
        policy_known: false,
        root_mount,
    };
    let policy_messages = messages.clone();
    follow_policy(home, move |policy| {
        policy_messages.send(Message::Policy(policy)).is_ok()
    });
    let connection = bus::start(messages).map_err(io::Error::other)?;
    thread::Builder::new()
        .name("index-bus".to_owned())
        .spawn(move || {
            bande_bus::serve_until_closed(&connection);
            std::process::exit(0);
        })?;
    service.run(received);
    Ok(())
}

fn cursor_key(domain: &str) -> String {
    format!("{domain}{DOMAIN_SEPARATOR}{DOMAIN_SEPARATOR}cursor")
}

fn memory_is_low() -> bool {
    let Ok(meminfo) = fs::read_to_string("/proc/meminfo") else {
        return false;
    };
    meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))
        .and_then(|line| line.split_whitespace().next()?.parse::<u64>().ok())
        .is_some_and(|kilobytes| kilobytes * 1024 < LOW_MEMORY_BYTES)
}

impl Service {
    fn run(&mut self, messages: Receiver<Message>) {
        loop {
            self.feed();
            let message = match self.next_deadline() {
                Some(deadline) => {
                    match messages.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    {
                        Ok(message) => Some(message),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                None => match messages.recv() {
                    Ok(message) => Some(message),
                    Err(_) => return,
                },
            };
            if let Some(message) = message {
                self.handle(message);
            }
            self.settle();
        }
    }

    fn next_deadline(&self) -> Option<Instant> {
        let mut deadlines = vec![self.settle_at];
        if !self.busy {
            deadlines.push(self.save_at);
            if !self.urgent.is_empty() || !self.backlog.is_empty() {
                deadlines.push(self.paused_until);
            }
        }
        deadlines.into_iter().flatten().min()
    }

    fn handle(&mut self, message: Message) {
        match message {
            Message::Find {
                text,
                within,
                limit,
                reply,
            } => {
                let _ = reply.send(self.find(&text, &within, limit));
            }
            Message::Donate {
                domain,
                items,
                cursor,
            } => self.donate(domain, items, cursor),
            Message::Cursor { domain, reply } => {
                let _ = reply.send(self.cursor(&domain));
            }
            Message::ForgetAll { domain } => {
                self.cursors.remove(&domain);
                self.edits.push(Edit::ForgetDomain(domain));
                self.note_unsaved();
            }
            Message::Forget { domain, ids } => {
                for id in ids {
                    self.edits
                        .push(Edit::ForgetItem(format!("{domain}{DOMAIN_SEPARATOR}{id}")));
                }
                self.note_unsaved();
            }
            Message::Changed(paths) => {
                self.changed.extend(paths);
                self.settle_at
                    .get_or_insert_with(|| Instant::now() + SETTLE);
            }
            Message::Overflowed => {
                tracing::info!("too many changes at once: checking the home folder again");
                self.begin_reconcile();
            }
            Message::Policy(policy) => self.apply_policy(policy),
            Message::Worked(reply) => self.worked(reply),
        }
    }

    fn settle(&mut self) {
        if self.settle_at.is_some_and(|at| at <= Instant::now()) {
            self.settle_at = None;
            let changed = std::mem::take(&mut self.changed);
            self.apply_changes(changed);
        }
    }

    fn feed(&mut self) {
        if self.busy {
            return;
        }
        let now = Instant::now();
        if self.edits.len() >= EDITS_PER_BATCH {
            return self.send_edits();
        }
        if self.unsaved && self.save_at.is_some_and(|at| at <= now) {
            if !self.edits.is_empty() {
                return self.send_edits();
            }
            self.save_at = None;
            self.unsaved = false;
            return self.send(Job::Commit);
        }
        while let Some(job) = self.urgent.pop_front() {
            if self.wanted(&job) {
                return self.send(job);
            }
        }
        if self.paused_until.is_some_and(|until| until > now) {
            return;
        }
        self.paused_until = None;
        while let Some(job) = self.backlog.pop_front() {
            if matches!(job, Job::Import { .. }) && memory_is_low() {
                self.backlog.push_front(job);
                self.paused_until = Some(now + LOW_MEMORY_RETRY);
                return;
            }
            if self.wanted(&job) {
                return self.send(job);
            }
        }
        if !self.edits.is_empty() {
            return self.send_edits();
        }
        if self.purge {
            self.purge = false;
            self.save_at = None;
            self.unsaved = false;
            return self.send(Job::Purge);
        }
        if !self.unsaved && self.writer_open {
            self.writer_open = false;
            self.send(Job::Rest);
        }
    }

    fn wanted(&mut self, job: &Job) -> bool {
        match job {
            Job::Import { id, path, .. } => {
                self.tree.path(&self.home, *id) == *path
                    && !self.tree.is_folder(*id)
                    && self.tree.stamp(*id).is_some_and(|stamp| !stamp.imported)
            }
            Job::Walk { id, path, .. } => {
                let wanted = self.tree.path(&self.home, *id) == *path && self.tree.is_folder(*id);
                if !wanted {
                    self.walk_finished();
                }
                wanted
            }
            _ => true,
        }
    }

    fn send(&mut self, job: Job) {
        if matches!(
            job,
            Job::Import { .. } | Job::Apply(_) | Job::Commit | Job::Purge
        ) {
            self.writer_open = true;
        }
        if self.jobs.send(job).is_err() {
            tracing::error!("the indexing thread has stopped");
            std::process::exit(1);
        }
        self.busy = true;
    }

    fn send_edits(&mut self) {
        let edits = std::mem::take(&mut self.edits);
        self.note_unsaved();
        self.send(Job::Apply(edits));
    }

    fn note_unsaved(&mut self) {
        self.unsaved = true;
        if self.save_at.is_none() {
            let now = Instant::now();
            let spaced = self.saved_at.map(|saved| saved + SAVE_SPACING);
            self.save_at =
                Some(spaced.map_or(now + FIRST_SAVE, |spaced| spaced.max(now + FIRST_SAVE)));
        }
    }

    fn worked(&mut self, reply: Reply) {
        self.busy = false;
        match reply {
            Reply::Listing { id, path, children } => self.listed(id, &path, children),
            Reply::Imported { id, stamp } => self.imported(id, stamp),
            Reply::Committed => {
                self.saved_at = Some(Instant::now());
            }
            Reply::Done => {}
        }
    }

    fn put(&mut self, id: u32) {
        let Some(stamp) = self.tree.stamp(id) else {
            return;
        };
        let parent = self.tree.parent(id).unwrap_or(ROOT);
        self.edits.push(Edit::Put(Entry {
            id,
            parent,
            name: self.tree.name(id).to_owned(),
            folder: self.tree.is_folder(id),
            stamp,
        }));
        self.note_unsaved();
    }

    fn remove(&mut self, id: u32) {
        let folders: Vec<PathBuf> = self
            .tree
            .subtree(id)
            .into_iter()
            .filter(|&at| self.tree.is_folder(at))
            .map(|at| self.tree.path(&self.home, at))
            .collect();
        if !folders.is_empty() {
            self.urgent.push_back(Job::Unwatch(folders));
        }
        for gone in self.tree.remove(id) {
            self.edits.push(Edit::Remove(gone));
        }
        self.note_unsaved();
    }

    fn queue_import(&mut self, id: u32, urgent: bool) {
        let job = Job::Import {
            id,
            path: self.tree.path(&self.home, id),
            name: self.tree.name(id).to_owned(),
        };
        match urgent {
            true => self.urgent.push_back(job),
            false => self.backlog.push_back(job),
        }
    }

    fn queue_walk(&mut self, id: u32, urgent: bool) {
        let job = Job::Walk {
            id,
            path: self.tree.path(&self.home, id),
            excluded: Arc::from(self.policy.excluded.as_slice()),
        };
        if let Some(reconcile) = &mut self.reconcile {
            reconcile.walks += 1;
        }
        match urgent {
            true => self.urgent.push_back(job),
            false => self.backlog.push_front(job),
        }
    }

    fn begin_reconcile(&mut self) {
        if !self.policy.files {
            return;
        }
        let by_inode = self
            .tree
            .ids()
            .filter_map(|id| Some((self.tree.stamp(id)?.inode, id)))
            .collect();
        self.reconcile = Some(Reconcile {
            by_inode,
            ..Reconcile::default()
        });
        self.queue_walk(ROOT, true);
    }

    fn listed(&mut self, id: u32, path: &Path, children: Vec<Child>) {
        if self.tree.path(&self.home, id) != path {
            self.walk_finished();
            return;
        }
        let reconciling = self.reconcile.is_some();
        let listed: BTreeSet<&str> = children.iter().map(|child| child.name.as_str()).collect();
        let vanished: Vec<u32> = self
            .tree
            .children(id)
            .filter(|&child| !listed.contains(self.tree.name(child)))
            .collect();
        for child in vanished {
            match &mut self.reconcile {
                Some(reconcile) => reconcile.vanished.push(child),
                None => self.remove(child),
            }
        }
        for child in children {
            self.settle_child(id, path, child, reconciling);
        }
        self.walk_finished();
    }

    fn walk_finished(&mut self) {
        let finished = match &mut self.reconcile {
            Some(reconcile) => {
                reconcile.walks = reconcile.walks.saturating_sub(1);
                reconcile.walks == 0
            }
            None => false,
        };
        if finished && let Some(reconcile) = self.reconcile.take() {
            for id in reconcile.vanished {
                let Some(inode) = self.tree.stamp(id).map(|stamp| stamp.inode) else {
                    continue;
                };
                let still_there = inspect(&self.tree.path(&self.home, id))
                    .is_some_and(|inspected| inspected.stamp.inode == inode);
                if !still_there {
                    self.remove(id);
                }
            }
        }
    }

    fn settle_child(&mut self, parent: u32, parent_path: &Path, child: Child, reconciling: bool) {
        let known = self.tree.child(parent, &child.name);
        if let Some(id) = known
            && self.tree.is_folder(id) != child.folder
        {
            self.remove(id);
            return self.settle_child(parent, parent_path, child, reconciling);
        }
        let id = match known {
            Some(id) => id,
            None => match self.moved_here(&child, parent, parent_path) {
                Some(id) => id,
                None => {
                    let Some(id) = self.tree.add(parent, &child.name, child.folder) else {
                        return;
                    };
                    id
                }
            },
        };
        let was = self.tree.stamp(id).unwrap_or_default();
        let imported = was.imported && was.same_file(&child.stamp);
        let stamp = Stamp {
            imported,
            ..child.stamp
        };
        if was != stamp || known.is_none() {
            self.tree.set_stamp(id, stamp);
            self.put(id);
        }
        match child.folder {
            true => self.queue_walk(id, !reconciling),
            false if !imported => self.queue_import(id, !reconciling),
            false => {}
        }
    }

    fn moved_here(&mut self, child: &Child, parent: u32, parent_path: &Path) -> Option<u32> {
        let id = *self.reconcile.as_ref()?.by_inode.get(&child.stamp.inode)?;
        if self.tree.is_folder(id) != child.folder
            || self.tree.stamp(id)?.inode != child.stamp.inode
        {
            return None;
        }
        let old = self.tree.path(&self.home, id);
        if old == parent_path.join(&child.name)
            || inspect(&old).is_some_and(|inspected| inspected.stamp.inode == child.stamp.inode)
        {
            return None;
        }
        if !self.relocate(id, parent, &child.name) {
            return None;
        }
        if let Some(reconcile) = &mut self.reconcile {
            reconcile.vanished.retain(|&gone| gone != id);
        }
        self.put(id);
        Some(id)
    }

    fn relocate(&mut self, id: u32, parent: u32, name: &str) -> bool {
        let renamed = self.tree.name(id) != name;
        if self.tree.relocate(id, parent, name).is_none() {
            return false;
        }
        if renamed
            && !self.tree.is_folder(id)
            && let Some(stamp) = self.tree.stamp(id)
        {
            self.tree.set_stamp(
                id,
                Stamp {
                    imported: false,
                    ..stamp
                },
            );
        }
        true
    }

    fn imported(&mut self, id: u32, stamp: Option<Stamp>) {
        let Some(stamp) = stamp else {
            return;
        };
        let Some(current) = self.tree.stamp(id) else {
            return;
        };
        if current.same_file(&stamp) && !current.imported {
            self.tree.set_stamp(
                id,
                Stamp {
                    imported: true,
                    ..current
                },
            );
            self.put(id);
        }
    }

    fn indexed(&self, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(&self.home) else {
            return false;
        };
        self.policy.files
            && !is_excluded(path, &self.policy.excluded)
            && relative
                .components()
                .all(|component| !component.as_os_str().to_str().is_none_or(skipped_name))
    }

    fn apply_changes(&mut self, changed: BTreeSet<PathBuf>) {
        let mut gone: HashMap<u64, u32> = HashMap::new();
        let mut present = Vec::new();
        for path in changed {
            if !self.indexed(&path) || path == self.home {
                continue;
            }
            if path.file_name().and_then(|name| name.to_str()) == Some(CACHE_TAG) {
                if let Some(id) = path.parent().and_then(|at| self.tree.find(&self.home, at))
                    && id != ROOT
                {
                    self.remove(id);
                }
                continue;
            }
            match inspect(&path) {
                Some(inspected) => present.push((path, inspected)),
                None => {
                    if let Some(id) = self.tree.find(&self.home, &path)
                        && let Some(stamp) = self.tree.stamp(id)
                    {
                        gone.insert(stamp.inode, id);
                    }
                }
            }
        }
        for (path, inspected) in present {
            let (Some(parent), Some(name)) = (
                path.parent().and_then(|at| self.tree.find(&self.home, at)),
                path.file_name().and_then(|name| name.to_str()),
            ) else {
                continue;
            };
            if !admitted(&path, &inspected, &self.policy.excluded, self.root_mount) {
                if let Some(id) = self.tree.child(parent, name) {
                    self.remove(id);
                }
                continue;
            }
            if self.tree.child(parent, name).is_none()
                && let Some(id) = gone.remove(&inspected.stamp.inode)
                && self.tree.is_folder(id) == inspected.folder
                && self.relocate(id, parent, name)
            {
                self.put(id);
                if inspected.folder {
                    self.queue_walk(id, true);
                } else if self.tree.stamp(id).is_some_and(|stamp| !stamp.imported) {
                    self.queue_import(id, true);
                }
                continue;
            }
            self.settle_child(
                parent,
                path.parent().unwrap_or(&self.home).to_path_buf().as_path(),
                Child {
                    name: name.to_owned(),
                    folder: inspected.folder,
                    stamp: inspected.stamp,
                },
                false,
            );
        }
        for id in gone.into_values() {
            self.remove(id);
        }
    }

    fn apply_policy(&mut self, policy: Policy) {
        let first = !self.policy_known;
        self.policy_known = true;
        let previous = std::mem::replace(&mut self.policy, policy);
        if !self.policy.files {
            if self.tree.live() > 0 {
                self.clear_files();
            }
            return;
        }
        if !previous.files {
            self.begin_reconcile();
            return;
        }
        let excluded = self.policy.excluded.clone();
        let mut privacy_changed = false;
        for folder in &excluded {
            if !previous.excluded.contains(folder)
                && let Some(id) = self.tree.find(&self.home, folder)
                && id != ROOT
            {
                self.remove(id);
                privacy_changed = true;
            }
        }
        self.purge |= privacy_changed;
        if first || previous.excluded != excluded {
            self.begin_reconcile();
        }
    }

    fn clear_files(&mut self) {
        let top: Vec<u32> = self.tree.children(ROOT).collect();
        for id in top {
            self.remove(id);
        }
        self.backlog.clear();
        self.reconcile = None;
        self.purge = true;
    }

    fn cursor(&self, domain: &str) -> String {
        self.cursors.get(domain).cloned().unwrap_or_else(|| {
            self.store
                .stored_title(&cursor_key(domain))
                .unwrap_or_default()
        })
    }

    fn donate(&mut self, domain: String, items: Vec<Donation>, cursor: String) {
        for item in items {
            self.edits.push(Edit::PutItem(Donated {
                key: format!("{domain}{DOMAIN_SEPARATOR}{}", item.id),
                domain: domain.clone(),
                kind: item.kind,
                title: item.title,
                detail: item.detail,
                target: item.target,
                body: item.body,
            }));
        }
        if !cursor.is_empty() {
            self.edits.push(Edit::PutItem(Donated {
                key: cursor_key(&domain),
                domain: domain.clone(),
                kind: String::new(),
                title: cursor.clone(),
                detail: String::new(),
                target: String::new(),
                body: String::new(),
            }));
            self.cursors.insert(domain, cursor);
        }
        self.note_unsaved();
    }

    fn boosts(&mut self) -> HashMap<u32, u32> {
        let Some(file) = &self.usage_file else {
            return HashMap::new();
        };
        let modified = fs::metadata(file)
            .and_then(|metadata| metadata.modified())
            .ok();
        let stale = match (&self.usage, modified) {
            (Some((seen, _)), Some(modified)) => *seen != modified,
            (None, Some(_)) => true,
            _ => false,
        };
        if stale && let Some(modified) = modified {
            self.usage = Some((modified, Usage::load(file.clone())));
        }
        let Some((_, usage)) = &self.usage else {
            return HashMap::new();
        };
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        usage
            .keys()
            .filter_map(|key| {
                let path = key.strip_prefix("file:")?;
                let id = self.tree.find(&self.home, Path::new(path))?;
                Some((id, usage.boost(key, now)))
            })
            .collect()
    }

    fn scope(&self, within: &str) -> Option<(PathBuf, Arc<Vec<bool>>)> {
        let within = PathBuf::from(within);
        if within.as_os_str().is_empty() || within == self.home {
            return None;
        }
        let mut allowed = Vec::new();
        if let Some(id) = self.tree.find(&self.home, &within) {
            for at in self.tree.descendants(id) {
                if allowed.len() <= at as usize {
                    allowed.resize(at as usize + 1, false);
                }
                if let Some(slot) = allowed.get_mut(at as usize) {
                    *slot = true;
                }
            }
        }
        Some((within, Arc::new(allowed)))
    }

    fn find(&mut self, text: &str, within: &str, limit: usize) -> Vec<Found> {
        let Some(query) = Query::new(text) else {
            return Vec::new();
        };
        let boosts = self.boosts();
        let scope = self.scope(within);
        let names = self.tree.search(
            &self.home,
            &query,
            scope.as_ref().map(|(path, _)| path.as_path()),
            limit,
            &boosts,
        );
        let mut found: Vec<Found> = names
            .into_iter()
            .map(|hit| Found {
                domain: String::new(),
                kind: match hit.folder {
                    true => "folder".to_owned(),
                    false => "file".to_owned(),
                },
                title: String::new(),
                detail: String::new(),
                target: hit.path.to_string_lossy().into_owned(),
                score: hit.score,
            })
            .collect();
        let hits = self
            .store
            .search(
                text,
                limit.saturating_mul(OVERFETCH),
                scope.map(|(_, allowed)| allowed),
            )
            .inspect_err(|error| tracing::warn!("cannot search the index: {error}"))
            .unwrap_or_default();
        let best = hits
            .iter()
            .map(|hit| match hit {
                Hit::File { relevance, .. } => *relevance,
                Hit::Item(item) => item.relevance,
            })
            .fold(f32::MIN_POSITIVE, f32::max);
        let score =
            |relevance: f32| CONTENT_SCORE + (relevance / best * CONTENT_RELEVANCE_RANGE) as u32;
        let named: BTreeSet<String> = found.iter().map(|hit| hit.target.clone()).collect();
        for hit in hits {
            match hit {
                Hit::File { id, relevance } => {
                    if self.tree.stamp(id).is_none() || self.tree.is_folder(id) {
                        continue;
                    }
                    let path = self
                        .tree
                        .path(&self.home, id)
                        .to_string_lossy()
                        .into_owned();
                    if named.contains(&path) {
                        continue;
                    }
                    found.push(Found {
                        domain: String::new(),
                        kind: "file".to_owned(),
                        title: String::new(),
                        detail: String::new(),
                        target: path,
                        score: score(relevance) + boosts.get(&id).copied().unwrap_or(0),
                    });
                }
                Hit::Item(item) => found.push(Found {
                    score: score(item.relevance),
                    domain: item.domain,
                    kind: item.kind,
                    title: item.title,
                    detail: item.detail,
                    target: item.target,
                }),
            }
        }
        found.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.target.cmp(&b.target)));
        found.truncate(limit);
        found
    }
}
