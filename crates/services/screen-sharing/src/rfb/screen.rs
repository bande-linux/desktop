use std::io;
use std::os::fd::OwnedFd;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use rustix::event::{EventfdFlags, eventfd};

use bande_rfb::encode::Frame;

#[derive(Default)]
struct Shown {
    frame: Option<Arc<Frame>>,
    serial: u64,
    ended: bool,
    copied: Option<String>,
    copy_serial: u64,
    wakers: Vec<Weak<OwnedFd>>,
}

/// The latest picture of the screen, shared by every viewer of it.
#[derive(Default)]
pub struct Screen {
    shown: Mutex<Shown>,
    changed: Condvar,
}

pub(crate) struct Latest {
    pub frame: Option<Arc<Frame>>,
    pub serial: u64,
    pub ended: bool,
    pub copied: Option<String>,
    pub copy_serial: u64,
}

impl Screen {
    fn lock(&self) -> MutexGuard<'_, Shown> {
        self.shown.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wake(shown: &mut Shown) {
        shown.wakers.retain(|waker| match waker.upgrade() {
            Some(waker) => {
                if let Err(error) = rustix::io::write(&*waker, &1u64.to_ne_bytes()) {
                    tracing::debug!("cannot wake a viewer: {error}");
                }
                true
            }
            None => false,
        });
    }

    /// Shows viewers a new picture of the screen.
    pub fn show(&self, frame: Frame) {
        let mut shown = self.lock();
        shown.frame = Some(Arc::new(frame));
        shown.serial += 1;
        Self::wake(&mut shown);
        self.changed.notify_all();
    }

    /// Wakes viewers to look at what changed besides the picture.
    pub fn nudge(&self) {
        Self::wake(&mut self.lock());
    }

    /// Gives viewers the text copied on this computer.
    pub fn copy(&self, text: String) {
        let mut shown = self.lock();
        shown.copied = Some(text);
        shown.copy_serial += 1;
        Self::wake(&mut shown);
    }

    /// Ends viewing: every viewer is disconnected.
    pub fn end(&self) {
        let mut shown = self.lock();
        shown.ended = true;
        Self::wake(&mut shown);
        self.changed.notify_all();
    }

    /// Whether viewing ended.
    pub fn is_ended(&self) -> bool {
        self.lock().ended
    }

    pub(crate) fn latest(&self) -> Latest {
        let shown = self.lock();
        Latest {
            frame: shown.frame.clone(),
            serial: shown.serial,
            ended: shown.ended,
            copied: shown.copied.clone(),
            copy_serial: shown.copy_serial,
        }
    }

    pub(crate) fn waker(&self) -> io::Result<Arc<OwnedFd>> {
        let waker = Arc::new(eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?);
        self.lock().wakers.push(Arc::downgrade(&waker));
        Ok(waker)
    }

    pub(crate) fn first_frame(&self, timeout: Duration) -> Option<Arc<Frame>> {
        let shown = self.lock();
        let (shown, _) = self
            .changed
            .wait_timeout_while(shown, timeout, |shown| {
                shown.frame.is_none() && !shown.ended
            })
            .unwrap_or_else(PoisonError::into_inner);
        shown.frame.clone().filter(|_| !shown.ended)
    }
}
