use std::fs;
use std::time::{Duration, Instant};

use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use crate::state::State;

const TYPING_PAUSE: Duration = Duration::from_millis(1500);
const MAX_ANCESTRY: usize = 64;

fn parent_of(pid: i32) -> Option<i32> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, after_name) = stat.rsplit_once(')')?;
    after_name.split_whitespace().nth(1)?.parse().ok()
}

fn descends_from(pid: i32, ancestor: i32) -> bool {
    std::iter::successors(parent_of(pid), |&pid| parent_of(pid))
        .take(MAX_ANCESTRY)
        .take_while(|&pid| pid > 0)
        .any(|pid| pid == ancestor)
}

impl State {
    pub(crate) fn note_typing(&mut self) {
        if self
            .focused()
            .is_some_and(|focused| self.window_index(&focused).is_some())
        {
            self.typed_at = Some(Instant::now());
        }
    }

    pub(crate) fn would_interrupt_typing(&self, surface: &WlSurface, focused: &WlSurface) -> bool {
        if self
            .typed_at
            .is_none_or(|typed| typed.elapsed() >= TYPING_PAUSE)
        {
            return false;
        }
        let pid = |surface: &WlSurface| {
            surface
                .client()
                .and_then(|client| client.get_credentials(&self.display_handle).ok())
                .map(|credentials| credentials.pid)
        };
        match (pid(surface), pid(focused)) {
            (Some(opening), Some(typing)) => opening != typing && !descends_from(opening, typing),
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_descends_from_its_parents_but_not_from_itself_or_strangers() {
        let own = std::process::id() as i32;
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .expect("spawn sleep");
        let child_pid = child.id() as i32;
        assert!(descends_from(child_pid, own));
        assert!(!descends_from(own, own));
        assert!(!descends_from(own, child_pid));
        child.kill().expect("kill sleep");
        child.wait().expect("reap sleep");
    }
}
