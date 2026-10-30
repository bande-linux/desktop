use std::io;
use std::os::fd::BorrowedFd;
use std::time::Instant;

use accesskit::TreeUpdate;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use unbound_graphics::{Rect, Scene};
use wayland_client::backend::{ReadEventsGuard, WaylandError};

use crate::window::{Window, WindowError, WindowEvent};

pub trait App {
    fn scene(&mut self, window: &mut Window) -> Scene;

    fn event(&mut self, window: &mut Window, event: WindowEvent);

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        Vec::new()
    }

    fn source_ready(&mut self, _window: &mut Window, _index: usize) {}

    fn deadline(&self) -> Option<Instant> {
        None
    }

    fn tick(&mut self, _window: &mut Window, _now: Instant) {}

    fn finished(&self) -> bool {
        false
    }

    fn should_close(&mut self, _window: &mut Window) -> bool {
        true
    }

    fn should_quit(&mut self, window: &mut Window) -> bool {
        self.should_close(window)
    }

    fn animating(&self) -> bool {
        false
    }

    fn text_input_area(&self) -> Option<Rect> {
        None
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        None
    }
}

const BUILT_IN_SOURCES: usize = 3;

fn earliest(deadlines: impl IntoIterator<Item = Option<Instant>>) -> Option<Instant> {
    deadlines.into_iter().flatten().min()
}

fn read_events(guard: ReadEventsGuard) -> Result<(), WindowError> {
    match guard.read() {
        Ok(_) => Ok(()),
        Err(WaylandError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn run(window: &mut Window, app: &mut impl App) -> Result<(), WindowError> {
    loop {
        let guard = loop {
            match window.queue.prepare_read() {
                Some(guard) => break guard,
                None => window.dispatch_pending()?,
            }
        };
        read_events(guard)?;
        window.dispatch_pending()?;
        window.fire_repeats(Instant::now());
        for event in window.take_events() {
            match event {
                WindowEvent::CloseRequested => {
                    if app.should_close(window) {
                        window.close();
                    }
                    window.closed_by_button = false;
                    continue;
                }
                WindowEvent::QuitRequested => {
                    if app.should_quit(window) {
                        window.close();
                    }
                    continue;
                }
                WindowEvent::Focus(focused) => window.accessibility.set_window_focused(focused),
                _ => {}
            }
            app.event(window, event);
        }
        if window.is_closed() || app.finished() {
            return Ok(());
        }
        let frame_drawn = window.wants_frame();
        if frame_drawn {
            let scene = app.scene(window);
            window.draw(&scene)?;
            if app.animating() {
                window.request_redraw();
            }
        }
        window.set_text_input_area(app.text_input_area());
        if window.accessibility.wants_tree(frame_drawn)
            && let Some(tree) = app.accessibility()
        {
            window.publish_accessibility(tree);
        }
        window.flush()?;
        let Some(guard) = window.queue.prepare_read() else {
            continue;
        };

        let window_deadline = window.deadline();
        let app_deadline = app.deadline();
        let deadline = earliest([
            window.state.input.repeat_deadline(),
            window_deadline,
            app_deadline,
        ]);
        let (wayland_ready, accessibility_ready, appearance_ready, pastes_ready, sources_ready) = {
            let connection = guard.connection_fd();
            let accessibility = window.accessibility.fd();
            let appearance = window.appearance.fd();
            let pastes = window.transfer_fds();
            let sources = app.sources();
            let mut fds = Vec::with_capacity(BUILT_IN_SOURCES + pastes.len() + sources.len());
            fds.push(PollFd::new(&connection, PollFlags::IN));
            fds.push(PollFd::new(&accessibility, PollFlags::IN));
            fds.push(PollFd::new(&appearance, PollFlags::IN));
            for fd in pastes.iter().chain(&sources) {
                fds.push(PollFd::new(fd, PollFlags::IN));
            }
            let timeout = deadline.map(|deadline| {
                let wait = deadline.saturating_duration_since(Instant::now());
                Timespec {
                    tv_sec: i64::try_from(wait.as_secs()).unwrap_or(i64::MAX),
                    tv_nsec: i64::from(wait.subsec_nanos()),
                }
            });
            match poll(&mut fds, timeout.as_ref()) {
                Ok(_) | Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(io::Error::from(error).into()),
            }
            let ready = |fd: &PollFd| !fd.revents().is_empty();
            let wayland_ready = ready(&fds[0]);
            let accessibility_ready = ready(&fds[1]);
            let appearance_ready = ready(&fds[2]);
            let pastes_ready = (0..pastes.len())
                .filter(|index| ready(&fds[BUILT_IN_SOURCES + index]))
                .collect::<Vec<_>>();
            let sources_ready = (0..sources.len())
                .filter(|index| ready(&fds[BUILT_IN_SOURCES + pastes.len() + index]))
                .collect::<Vec<_>>();
            (
                wayland_ready,
                accessibility_ready,
                appearance_ready,
                pastes_ready,
                sources_ready,
            )
        };
        if wayland_ready {
            read_events(guard)?;
        } else {
            drop(guard);
        }
        if accessibility_ready {
            for request in window.accessibility.take_requests() {
                if !window.perform_window_action(&request) {
                    app.event(window, WindowEvent::Accessibility(request));
                }
            }
        }
        if appearance_ready && let Some(appearance) = window.appearance.take_change() {
            window.request_redraw();
            app.event(window, WindowEvent::AppearanceChanged(appearance));
        }
        for index in pastes_ready.into_iter().rev() {
            window.read_transfer(index);
        }
        for index in sources_ready {
            app.source_ready(window, index);
        }
        let now = Instant::now();
        if window_deadline.is_some_and(|deadline| deadline <= now) {
            window.tick(now);
        }
        if app_deadline.is_some_and(|deadline| deadline <= now) {
            app.tick(window, now);
        }
    }
}
