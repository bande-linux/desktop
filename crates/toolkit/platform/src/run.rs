use std::io;
use std::os::fd::BorrowedFd;
use std::time::Instant;

use accesskit::TreeUpdate;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use unbound_graphics::{Rect, Scene};
use unbound_widgets::{set_frame_time, take_wake};
use wayland_client::backend::{ReadEventsGuard, WaylandError};
use wayland_client::protocol::wl_surface::WlSurface;

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
const SOURCE_EVENTS: PollFlags = PollFlags::IN.union(PollFlags::PRI);

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
        set_frame_time(window.frame_time());
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
            set_frame_time(window.plan_frame());
            let scene = app.scene(window);
            window.draw(scene)?;
            window.wake_ui_at(take_wake());
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
            let appearance = window.appearance.borrow();
            let appearance = appearance.fd();
            let pastes = window.transfer_fds();
            let sources = app.sources();
            let mut fds = Vec::with_capacity(BUILT_IN_SOURCES + pastes.len() + sources.len());
            fds.push(PollFd::new(&connection, PollFlags::IN));
            fds.push(PollFd::new(&accessibility, PollFlags::IN));
            fds.push(PollFd::new(&appearance, PollFlags::IN));
            for fd in &pastes {
                fds.push(PollFd::new(fd, PollFlags::IN));
            }
            for fd in &sources {
                fds.push(PollFd::new(fd, SOURCE_EVENTS));
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
        set_frame_time(window.frame_time());
        if accessibility_ready {
            for request in window.accessibility.take_requests() {
                if !window.perform_window_action(&request) {
                    app.event(window, WindowEvent::Accessibility(request));
                }
            }
        }
        let change = appearance_ready
            .then(|| window.appearance.borrow_mut().take_change())
            .flatten();
        if let Some(appearance) = change {
            window.request_redraw();
            let appearance = window.for_role(appearance);
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

pub trait Surfaces {
    fn windows(&self) -> &[Window];

    fn windows_mut(&mut self) -> &mut [Window];

    fn scene(&mut self, index: usize) -> Scene;

    fn event(&mut self, index: usize, event: WindowEvent);

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        Vec::new()
    }

    fn source_ready(&mut self, _index: usize) {}

    fn deadline(&self) -> Option<Instant> {
        None
    }

    fn tick(&mut self, _now: Instant) {}

    fn animating(&self, _index: usize) -> bool {
        false
    }

    fn accessibility(&self, _index: usize) -> Option<TreeUpdate> {
        None
    }

    fn text_input_area(&self, _index: usize) -> Option<Rect> {
        None
    }

    fn closed(&mut self) -> bool {
        self.windows().iter().any(Window::is_closed)
    }
}

fn surfaces(app: &impl Surfaces) -> Vec<WlSurface> {
    app.windows()
        .iter()
        .map(|window| window.surface.clone())
        .collect()
}

fn position(app: &impl Surfaces, surface: &WlSurface) -> Option<usize> {
    app.windows()
        .iter()
        .position(|window| window.surface == *surface)
}

fn deliver(app: &mut impl Surfaces) -> Result<bool, WindowError> {
    let now = Instant::now();
    for surface in surfaces(app) {
        let Some(index) = position(app, &surface) else {
            continue;
        };
        let window = &mut app.windows_mut()[index];
        window.dispatch_pending()?;
        window.fire_repeats(now);
        set_frame_time(window.frame_time());
        for event in window.take_events() {
            let Some(index) = position(app, &surface) else {
                break;
            };
            if let WindowEvent::Focus(focused) = event {
                app.windows_mut()[index]
                    .accessibility
                    .set_window_focused(focused);
            }
            app.event(index, event);
        }
    }
    Ok(app.closed() || app.windows().is_empty())
}

struct Readiness {
    wayland: bool,
    appearance: bool,
    accessibility: Vec<usize>,
    pastes: Vec<(usize, usize)>,
    sources: Vec<usize>,
}

fn wait(
    app: &impl Surfaces,
    guard: &ReadEventsGuard,
    deadline: Option<Instant>,
) -> Result<Readiness, WindowError> {
    let windows = app.windows();
    let count = windows.len();
    let connection = guard.connection_fd();
    let appearance = windows[0].appearance.borrow();
    let appearance = appearance.fd();
    let accessibility: Vec<BorrowedFd<'_>> = windows
        .iter()
        .map(|window| window.accessibility.fd())
        .collect();
    let pastes: Vec<(usize, BorrowedFd<'_>)> = windows
        .iter()
        .enumerate()
        .flat_map(|(index, window)| window.transfer_fds().into_iter().map(move |fd| (index, fd)))
        .collect();
    let sources = app.sources();
    let mut fds = Vec::with_capacity(2 + count + pastes.len() + sources.len());
    fds.push(PollFd::new(&connection, PollFlags::IN));
    fds.push(PollFd::new(&appearance, PollFlags::IN));
    for fd in accessibility.iter().chain(pastes.iter().map(|(_, fd)| fd)) {
        fds.push(PollFd::new(fd, PollFlags::IN));
    }
    for fd in &sources {
        fds.push(PollFd::new(fd, SOURCE_EVENTS));
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
    let ready = |at: usize| !fds[at].revents().is_empty();
    let mut readiness = Readiness {
        wayland: ready(0),
        appearance: ready(1),
        accessibility: (0..count).filter(|at| ready(2 + at)).collect(),
        pastes: Vec::new(),
        sources: (0..sources.len())
            .filter(|at| ready(2 + count + pastes.len() + at))
            .collect(),
    };
    let mut seen = vec![0; count];
    for (at, (index, _)) in pastes.iter().enumerate() {
        if ready(2 + count + at) {
            readiness.pastes.push((*index, seen[*index]));
        }
        seen[*index] += 1;
    }
    Ok(readiness)
}

pub fn run_surfaces(app: &mut impl Surfaces) -> Result<(), WindowError> {
    loop {
        if app.windows().is_empty() {
            return Ok(());
        }
        if deliver(app)? {
            return Ok(());
        }
        for index in 0..app.windows().len() {
            let frame_drawn = app.windows()[index].wants_frame();
            if frame_drawn {
                set_frame_time(app.windows_mut()[index].plan_frame());
                let scene = app.scene(index);
                let animating = app.animating(index);
                let window = &mut app.windows_mut()[index];
                window.draw(scene)?;
                window.wake_ui_at(take_wake());
                if animating {
                    window.request_redraw();
                }
            }
            let area = app.text_input_area(index);
            app.windows_mut()[index].set_text_input_area(area);
            if app.windows()[index].accessibility.wants_tree(frame_drawn)
                && let Some(tree) = app.accessibility(index)
            {
                app.windows_mut()[index].publish_accessibility(tree);
            }
        }
        app.windows_mut()[0].flush()?;
        let mut waiting = false;
        for window in app.windows_mut() {
            window.dispatch_pending()?;
            waiting |= window.has_events();
        }
        if waiting {
            continue;
        }
        let Some(guard) = app.windows_mut()[0].queue.prepare_read() else {
            continue;
        };
        let window_deadlines: Vec<Option<Instant>> =
            app.windows().iter().map(Window::deadline).collect();
        let app_deadline = app.deadline();
        let deadline = earliest(
            app.windows()
                .iter()
                .map(|window| window.state.input.repeat_deadline())
                .chain(window_deadlines.iter().copied())
                .chain([app_deadline]),
        );
        let readiness = wait(app, &guard, deadline)?;
        if readiness.wayland {
            read_events(guard)?;
        } else {
            drop(guard);
        }
        set_frame_time(app.windows()[0].frame_time());
        for index in readiness.accessibility {
            let Some(window) = app.windows_mut().get_mut(index) else {
                continue;
            };
            let surface = window.surface.clone();
            for request in window.accessibility.take_requests() {
                let Some(index) = position(app, &surface) else {
                    break;
                };
                if !app.windows_mut()[index].perform_window_action(&request) {
                    app.event(index, WindowEvent::Accessibility(request));
                }
            }
        }
        if readiness.appearance {
            let change = app
                .windows()
                .first()
                .and_then(|window| window.appearance.borrow_mut().take_change());
            if let Some(appearance) = change {
                for surface in surfaces(app) {
                    let Some(index) = position(app, &surface) else {
                        continue;
                    };
                    app.windows_mut()[index].request_redraw();
                    let appearance = app.windows()[index].for_role(appearance);
                    app.event(index, WindowEvent::AppearanceChanged(appearance));
                }
            }
        }
        for (index, paste) in readiness.pastes.into_iter().rev() {
            if let Some(window) = app.windows_mut().get_mut(index) {
                window.read_transfer(paste);
            }
        }
        for source in readiness.sources {
            app.source_ready(source);
        }
        let now = Instant::now();
        for (index, deadline) in window_deadlines.into_iter().enumerate() {
            if deadline.is_some_and(|deadline| deadline <= now)
                && let Some(window) = app.windows_mut().get_mut(index)
            {
                window.tick(now);
            }
        }
        if app_deadline.is_some_and(|deadline| deadline <= now) {
            app.tick(now);
        }
    }
}
