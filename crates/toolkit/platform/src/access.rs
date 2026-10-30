use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender, channel};

use accesskit::{ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, TreeUpdate};
use accesskit_unix::Adapter;

enum Signal {
    Activated,
    Deactivated,
    Action(ActionRequest),
}

struct Notifier {
    signals: Sender<Signal>,
    wake: UnixStream,
}

impl Notifier {
    fn new(signals: &Sender<Signal>, wake: &UnixStream) -> io::Result<Self> {
        Ok(Self {
            signals: signals.clone(),
            wake: wake.try_clone()?,
        })
    }

    fn send(&mut self, signal: Signal) {
        if self.signals.send(signal).is_ok() {
            let _ = self.wake.write_all(&[0]);
        }
    }
}

impl ActivationHandler for Notifier {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        self.send(Signal::Activated);
        None
    }
}

impl ActionHandler for Notifier {
    fn do_action(&mut self, request: ActionRequest) {
        self.send(Signal::Action(request));
    }
}

impl DeactivationHandler for Notifier {
    fn deactivate_accessibility(&mut self) {
        self.send(Signal::Deactivated);
    }
}

pub(crate) struct Accessibility {
    adapter: Adapter,
    signals: Receiver<Signal>,
    woken: UnixStream,
    _wake: UnixStream,
    active: bool,
    tree_requested: bool,
}

impl Accessibility {
    pub(crate) fn new() -> io::Result<Self> {
        let (sender, signals) = channel();
        let (wake, woken) = UnixStream::pair()?;
        woken.set_nonblocking(true)?;
        let adapter = Adapter::new(
            Notifier::new(&sender, &wake)?,
            Notifier::new(&sender, &wake)?,
            Notifier::new(&sender, &wake)?,
        );
        Ok(Self {
            adapter,
            signals,
            woken,
            _wake: wake,
            active: false,
            tree_requested: false,
        })
    }

    pub(crate) fn fd(&self) -> BorrowedFd<'_> {
        self.woken.as_fd()
    }

    pub(crate) fn take_requests(&mut self) -> Vec<ActionRequest> {
        let mut drained = [0; 64];
        while matches!(self.woken.read(&mut drained), Ok(read) if read > 0) {}
        let mut requests = Vec::new();
        for signal in self.signals.try_iter() {
            match signal {
                Signal::Activated => {
                    self.active = true;
                    self.tree_requested = true;
                }
                Signal::Deactivated => self.active = false,
                Signal::Action(request) => requests.push(request),
            }
        }
        requests
    }

    pub(crate) fn wants_tree(&self, frame_drawn: bool) -> bool {
        self.active && (frame_drawn || self.tree_requested)
    }

    pub(crate) fn publish(&mut self, tree: TreeUpdate) {
        self.tree_requested = false;
        self.adapter.update_if_active(|| tree);
    }

    pub(crate) fn request_tree(&mut self) {
        self.tree_requested = true;
    }

    pub(crate) fn set_window_focused(&mut self, focused: bool) {
        self.adapter.update_window_focus_state(focused);
    }
}
