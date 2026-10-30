use std::io;
use std::os::fd::BorrowedFd;
use std::time::{Duration, Instant};

use crate::ops::{Answer, Change, Conflict, Job, Outcome, Progress, Report, Task};
use crate::trash::Trash;
use crate::undo::{Action, History, Step};

const REVEAL_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Do(Action),
    Undo(Action),
    Redo(Action),
    Erase,
}

#[derive(Debug)]
pub struct Running {
    pub job: Job,
    pub purpose: Purpose,
    pub started: Instant,
    pub progress: Progress,
    pub conflict: Option<Conflict>,
    pub items: usize,
}

impl Running {
    pub fn visible(&self, now: Instant) -> bool {
        now.duration_since(self.started) >= REVEAL_DELAY || self.conflict.is_some()
    }
}

#[derive(Debug)]
pub struct Finished {
    pub purpose: Purpose,
    pub outcome: Outcome,
}

#[derive(Debug)]
pub struct Operations {
    trash: Trash,
    history: History,
    running: Vec<Running>,
}

fn items_in(task: &Task) -> usize {
    match task {
        Task::Copy { sources, .. }
        | Task::Move { sources, .. }
        | Task::Duplicate { sources }
        | Task::Trash { sources }
        | Task::Delete { sources } => sources.len(),
        Task::Restore { items } | Task::Erase { items } => items.len(),
        Task::Revert { changes } => changes.len(),
        Task::EmptyTrash => 0,
    }
}

impl Operations {
    pub fn new(trash: Trash) -> Self {
        Self {
            trash,
            history: History::default(),
            running: Vec::new(),
        }
    }

    pub fn running(&self) -> &[Running] {
        &self.running
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    pub fn start(&mut self, task: Task, purpose: Purpose) -> io::Result<()> {
        let items = items_in(&task);
        let job = Job::start(task, self.trash.clone())?;
        self.running.push(Running {
            job,
            purpose,
            started: Instant::now(),
            progress: Progress::default(),
            conflict: None,
            items,
        });
        Ok(())
    }

    pub fn record(&mut self, action: Action, change: Change) {
        self.history.record(action, vec![change]);
    }

    fn revert(&mut self, step: Step, purpose: Purpose) -> io::Result<()> {
        self.start(
            Task::Revert {
                changes: step.changes,
            },
            purpose,
        )
    }

    pub fn undo(&mut self) -> io::Result<bool> {
        let Some(step) = self.history.take_undo() else {
            return Ok(false);
        };
        let action = step.action;
        self.revert(step, Purpose::Undo(action)).map(|()| true)
    }

    pub fn redo(&mut self) -> io::Result<bool> {
        let Some(step) = self.history.take_redo() else {
            return Ok(false);
        };
        let action = step.action;
        self.revert(step, Purpose::Redo(action)).map(|()| true)
    }

    pub fn fds(&self) -> Vec<BorrowedFd<'_>> {
        self.running
            .iter()
            .map(|running| running.job.fd())
            .collect()
    }

    pub fn conflict(&self) -> Option<(usize, &Running)> {
        self.running
            .iter()
            .enumerate()
            .find(|(_, running)| running.conflict.is_some())
    }

    pub fn answer(&mut self, index: usize, answer: Answer) {
        if let Some(running) = self.running.get_mut(index) {
            running.conflict = None;
            running.job.answer(answer);
        }
    }

    pub fn cancel(&mut self, index: usize) {
        if let Some(running) = self.running.get_mut(index) {
            running.conflict = None;
            running.job.cancel();
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        let now = Instant::now();
        self.running
            .iter()
            .filter(|running| !running.visible(now))
            .map(|running| running.started + REVEAL_DELAY)
            .min()
    }

    pub fn poll(&mut self, index: usize) -> Option<Finished> {
        let running = self.running.get_mut(index)?;
        let mut outcome = None;
        for report in running.job.take_reports() {
            match report {
                Report::Progress(progress) => running.progress = progress,
                Report::Conflict(conflict) => running.conflict = Some(conflict),
                Report::Finished(finished) => outcome = Some(finished),
            }
        }
        let outcome = outcome?;
        let running = self.running.remove(index);
        let step = |action| Step {
            action,
            changes: outcome.changes.clone(),
        };
        match running.purpose {
            Purpose::Do(action) => self.history.record(action, outcome.changes.clone()),
            Purpose::Undo(action) => self.history.undone(step(action)),
            Purpose::Redo(action) => self.history.redone(step(action)),
            Purpose::Erase => {}
        }
        Some(Finished {
            purpose: running.purpose,
            outcome,
        })
    }
}
