use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

use bande_search::{CACHE_TAG, Stamp, is_excluded, skipped_name};
use bande_watch::FolderWatch;
use rustix::fs::{AtFlags, CWD, FileType, StatxFlags, statx};
use tantivy::schema::TantivyDocument;
use tantivy::{Index, IndexWriter, Term, doc};

use crate::import::{MAX_TEXT, open_quietly, text_of};
use crate::store::{Entry, Fields, entry_document};

const WRITER_MEMORY: usize = 32_000_000;
const COMMIT_EVERY_BYTES: usize = 128 << 20;
const ENCRYPTED_ATTRIBUTE: u64 = 0x800;
const IDLE_IO_CLASS: i32 = 3;
const IO_CLASS_SHIFT: i32 = 13;
const IO_WHO_THREAD: i32 = 1;

pub struct Inspected {
    pub folder: bool,
    pub regular: bool,
    pub mount: u64,
    pub encrypted: bool,
    pub stamp: Stamp,
}

pub fn inspect(path: &Path) -> Option<Inspected> {
    let stat = statx(
        CWD,
        path,
        AtFlags::SYMLINK_NOFOLLOW,
        StatxFlags::BASIC_STATS | StatxFlags::MNT_ID,
    )
    .ok()?;
    let kind = FileType::from_raw_mode(u32::from(stat.stx_mode));
    Some(Inspected {
        folder: kind == FileType::Directory,
        regular: kind == FileType::RegularFile,
        mount: stat.stx_mnt_id,
        encrypted: stat.stx_attributes.bits() & ENCRYPTED_ATTRIBUTE != 0,
        stamp: Stamp {
            inode: stat.stx_ino,
            modified: stat
                .stx_mtime
                .tv_sec
                .saturating_mul(1_000_000_000)
                .saturating_add(i64::from(stat.stx_mtime.tv_nsec)),
            size: stat.stx_size,
            imported: false,
        },
    })
}

pub fn admitted(
    path: &Path,
    inspected: &Inspected,
    excluded: &[PathBuf],
    root_mount: Option<u64>,
) -> bool {
    match inspected.folder {
        true => {
            !is_excluded(path, excluded)
                && !inspected.encrypted
                && Some(inspected.mount) == root_mount
                && !path.join(CACHE_TAG).exists()
        }
        false => inspected.regular && !inspected.encrypted,
    }
}

pub enum Edit {
    Put(Entry),
    Remove(u32),
    PutItem(Donated),
    ForgetItem(String),
    ForgetDomain(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Donated {
    pub key: String,
    pub domain: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub target: String,
    pub body: String,
}

pub struct Child {
    pub name: String,
    pub folder: bool,
    pub stamp: Stamp,
}

pub enum Job {
    Walk {
        id: u32,
        path: PathBuf,
        excluded: Arc<[PathBuf]>,
    },
    Unwatch(Vec<PathBuf>),
    Import {
        id: u32,
        path: PathBuf,
        name: String,
    },
    Apply(Vec<Edit>),
    Commit,
    Purge,
    Rest,
}

pub enum Reply {
    Listing {
        id: u32,
        path: PathBuf,
        children: Vec<Child>,
    },
    Imported {
        id: u32,
        stamp: Option<Stamp>,
    },
    Committed,
    Done,
}

pub struct Background {
    pub index: Index,
    pub fields: Fields,
    pub root_mount: Option<u64>,
    pub watcher: Option<Arc<FolderWatch>>,
    pub writer: Option<IndexWriter>,
    pub written: usize,
    pub watch_failed: bool,
}

fn lower_own_priority() {
    let parameter = libc::sched_param { sched_priority: 0 };
    let scheduled = unsafe { libc::sched_setscheduler(0, libc::SCHED_IDLE, &raw const parameter) };
    let io = unsafe {
        libc::syscall(
            libc::SYS_ioprio_set,
            IO_WHO_THREAD,
            0,
            IDLE_IO_CLASS << IO_CLASS_SHIFT,
        )
    };
    if scheduled != 0 || io != 0 {
        tracing::warn!(
            "cannot lower the indexer's priority: {}",
            std::io::Error::last_os_error()
        );
    }
}

impl Background {
    pub fn run(mut self, jobs: Receiver<Job>, replies: Sender<Reply>) {
        lower_own_priority();
        while let Ok(job) = jobs.recv() {
            let reply = self.work(job);
            if replies.send(reply).is_err() {
                return;
            }
        }
        if let Some(writer) = self.writer.take() {
            let _ = writer.wait_merging_threads();
        }
    }

    fn writer(&mut self) -> Option<&mut IndexWriter> {
        if self.writer.is_none() {
            match self.index.writer_with_num_threads(1, WRITER_MEMORY) {
                Ok(writer) => self.writer = Some(writer),
                Err(error) => tracing::warn!("cannot write the search index: {error}"),
            }
        }
        self.writer.as_mut()
    }

    fn work(&mut self, job: Job) -> Reply {
        match job {
            Job::Walk { id, path, excluded } => {
                let children = self.list(&path, &excluded);
                Reply::Listing { id, path, children }
            }
            Job::Unwatch(folders) => {
                if let Some(watcher) = &self.watcher {
                    for folder in folders {
                        watcher.unwatch(&folder);
                    }
                }
                Reply::Done
            }
            Job::Import { id, path, name } => self.import(id, &path, &name),
            Job::Apply(edits) => {
                self.apply(edits);
                Reply::Done
            }
            Job::Commit => {
                self.commit();
                Reply::Committed
            }
            Job::Purge => {
                self.purge();
                Reply::Committed
            }
            Job::Rest => {
                self.commit();
                if let Some(writer) = self.writer.take()
                    && let Err(error) = writer.wait_merging_threads()
                {
                    tracing::warn!("cannot finish merging the search index: {error}");
                }
                unsafe { libc::malloc_trim(0) };
                Reply::Committed
            }
        }
    }

    fn commit(&mut self) {
        self.written = 0;
        if let Some(writer) = self.writer()
            && let Err(error) = writer.commit()
        {
            tracing::warn!("cannot save the search index: {error}");
        }
    }

    fn purge(&mut self) {
        self.commit();
        let segments = self.index.searchable_segment_ids().unwrap_or_default();
        if !segments.is_empty()
            && let Some(writer) = self.writer()
        {
            let merged = writer.merge(&segments).wait();
            if let Err(error) = merged {
                tracing::warn!("cannot rewrite the search index: {error}");
            }
            let collected = writer.garbage_collect_files().wait();
            if let Err(error) = collected {
                tracing::warn!("cannot clear old search index files: {error}");
            }
        }
    }

    fn apply(&mut self, edits: Vec<Edit>) {
        let fields = self.fields;
        let Some(writer) = self.writer() else {
            return;
        };
        for edit in edits {
            match edit {
                Edit::Put(entry) => {
                    writer.delete_term(Term::from_field_u64(fields.entry, u64::from(entry.id)));
                    add(writer, entry_document(&fields, &entry));
                }
                Edit::Remove(id) => {
                    writer.delete_term(Term::from_field_u64(fields.entry, u64::from(id)));
                    writer.delete_term(Term::from_field_u64(fields.content, u64::from(id)));
                }
                Edit::PutItem(item) => {
                    writer.delete_term(Term::from_field_text(fields.item, &item.key));
                    add(
                        writer,
                        doc!(
                            fields.item => item.key,
                            fields.domain => item.domain,
                            fields.kind => item.kind,
                            fields.title => item.title.as_str(),
                            fields.detail => item.detail,
                            fields.target => item.target,
                            fields.body => format!("{}\n{}", item.title, item.body),
                        ),
                    );
                }
                Edit::ForgetItem(key) => {
                    writer.delete_term(Term::from_field_text(fields.item, &key));
                }
                Edit::ForgetDomain(domain) => {
                    writer.delete_term(Term::from_field_text(fields.domain, &domain));
                }
            }
        }
    }

    fn list(&mut self, path: &Path, excluded: &[PathBuf]) -> Vec<Child> {
        if let Some(watcher) = &self.watcher
            && let Err(error) = watcher.rewatch(path)
            && !self.watch_failed
        {
            self.watch_failed = true;
            tracing::warn!("cannot watch {} and perhaps others: {error}", path.display());
        }
        let Ok(listing) = fs::read_dir(path) else {
            return Vec::new();
        };
        let mut children = Vec::new();
        for child in listing.flatten() {
            let name = child.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if skipped_name(name) {
                continue;
            }
            let path = child.path();
            let Some(inspected) = inspect(&path) else {
                continue;
            };
            if !admitted(&path, &inspected, excluded, self.root_mount) {
                continue;
            }
            children.push(Child {
                name: name.to_owned(),
                folder: inspected.folder,
                stamp: inspected.stamp,
            });
        }
        children
    }

    fn import(&mut self, id: u32, path: &Path, name: &str) -> Reply {
        let Some(stamp) = inspect(path)
            .filter(|inspected| inspected.regular && !inspected.encrypted)
            .map(|inspected| inspected.stamp)
        else {
            return Reply::Imported { id, stamp: None };
        };
        let text = open_quietly(path)
            .ok()
            .and_then(|file| text_of(path, file))
            .filter(|text| !text.trim().is_empty());
        let fields = self.fields;
        let Some(writer) = self.writer() else {
            return Reply::Imported { id, stamp: None };
        };
        writer.delete_term(Term::from_field_u64(fields.content, u64::from(id)));
        let mut size = 0;
        if let Some(mut text) = text {
            text.truncate(text.floor_char_boundary(MAX_TEXT));
            size = text.len();
            add(
                writer,
                doc!(fields.content => u64::from(id), fields.body => format!("{name}\n{text}")),
            );
        }
        self.written += size;
        if self.written >= COMMIT_EVERY_BYTES {
            self.commit();
        }
        Reply::Imported {
            id,
            stamp: Some(stamp),
        }
    }
}

fn add(writer: &IndexWriter, document: TantivyDocument) {
    if let Err(error) = writer.add_document(document) {
        tracing::warn!("cannot add to the search index: {error}");
    }
}
