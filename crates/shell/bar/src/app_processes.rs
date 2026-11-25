use std::cell::RefCell;
use std::collections::HashSet;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::Arc;

thread_local! {
    static KNOWN: RefCell<Vec<AppProcess>> = const { RefCell::new(Vec::new()) };
    static ENDED: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
}

#[derive(Clone)]
struct AppProcess {
    app_id: Option<String>,
    pid: u32,
    handle: Arc<OwnedFd>,
}

pub struct AppProcesses(Vec<AppProcess>);

impl AppProcesses {
    pub fn known() -> Self {
        Self(KNOWN.with_borrow(Clone::clone))
    }

    fn share(&self) {
        KNOWN.set(self.0.clone());
    }

    pub fn follow(&mut self, app_id: &str, pid: u32, handle: &Arc<OwnedFd>) {
        if ENDED.with_borrow(|ended| ended.contains(&pid)) {
            return;
        }
        match self.0.iter_mut().find(|known| known.pid == pid) {
            Some(known) if known.app_id.as_deref().is_none_or(|known| known == app_id) => return,
            Some(known) => known.app_id = None,
            None => self.0.push(AppProcess {
                app_id: Some(app_id.to_owned()),
                pid,
                handle: Arc::clone(handle),
            }),
        }
        self.share();
    }

    pub fn app_ids(&self) -> impl Iterator<Item = &str> {
        self.0
            .iter()
            .filter_map(|process| process.app_id.as_deref())
    }

    pub fn run(&self, app_id: &str) -> bool {
        self.app_ids().any(|known| known == app_id)
    }

    pub fn is_of(&self, pid: u32, app_id: &str) -> bool {
        self.0
            .iter()
            .any(|process| process.pid == pid && process.app_id.as_deref() == Some(app_id))
    }

    pub fn fds(&self) -> impl Iterator<Item = BorrowedFd<'_>> {
        self.0.iter().map(|process| process.handle.as_fd())
    }

    pub fn ended(&mut self, index: usize) {
        if index < self.0.len() {
            let process = self.0.remove(index);
            ENDED.with_borrow_mut(|ended| ended.insert(process.pid));
            self.share();
        }
    }
}
