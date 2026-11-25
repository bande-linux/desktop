use std::cell::RefCell;
use std::collections::HashSet;
use std::os::fd::{AsFd, BorrowedFd};

use bande_bus::HeldProcess;

thread_local! {
    static KNOWN: RefCell<Vec<AppProcess>> = const { RefCell::new(Vec::new()) };
    static ENDED: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
}

#[derive(Clone)]
struct AppProcess {
    app_id: Option<String>,
    process: HeldProcess,
}

pub struct AppProcesses(Vec<AppProcess>);

impl AppProcesses {
    pub fn known() -> Self {
        Self(KNOWN.with_borrow(Clone::clone))
    }

    fn share(&self) {
        KNOWN.set(self.0.clone());
    }

    pub fn follow(&mut self, app_id: &str, process: &HeldProcess) {
        if ENDED.with_borrow(|ended| ended.contains(&process.pid)) {
            return;
        }
        match self.0.iter_mut().find(|known| known.process == *process) {
            Some(known) if known.app_id.as_deref().is_none_or(|known| known == app_id) => return,
            Some(known) => known.app_id = None,
            None => self.0.push(AppProcess {
                app_id: Some(app_id.to_owned()),
                process: process.clone(),
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

    pub fn is_of(&self, process: &HeldProcess, app_id: &str) -> bool {
        self.0
            .iter()
            .any(|known| known.process == *process && known.app_id.as_deref() == Some(app_id))
    }

    pub fn fds(&self) -> impl Iterator<Item = BorrowedFd<'_>> {
        self.0.iter().map(|known| known.process.handle.as_fd())
    }

    pub fn ended(&mut self, index: usize) {
        if index < self.0.len() {
            let ended = self.0.remove(index).process.pid;
            ENDED.with_borrow_mut(|pids| pids.insert(ended));
            self.share();
        }
    }
}
