use std::cell::RefCell;
use std::io;
use std::os::fd::BorrowedFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Instant;

use accesskit::TreeUpdate;
use bande_app_menu::{AppMenu, KEY_ACTION, Standard, shortcut_action};
use bande_toolkit::ui::{Event, parse_shortcut, shortcut_pressed};
use bande_toolkit::{
    AppConnection, AppRequest, Rect, Scene, Surfaces, Window, WindowError, WindowEvent,
};
use rustix::event::{PollFd, PollFlags, Timespec, poll};

use crate::cx::Content;
use crate::instance::{self, Claim, Instance, Request};
use crate::menus::assemble;
use crate::{Cx, Error, Info, Program, WindowId};

#[derive(Debug, Clone, Copy)]
enum Source {
    Menu,
    Instance,
    Program(usize),
    View(WindowId, usize),
}

struct Runner<P: Program> {
    program: P,
    cx: Cx<P>,
    menu: Option<AppMenu>,
    instance: Option<Instance>,
    sources: RefCell<Vec<Source>>,
}

pub(crate) fn run<P: Program>(
    info: &'static Info,
    arguments: Vec<String>,
    display: Option<UnixStream>,
) -> Result<(), Error> {
    let session = bande_bus::session()
        .inspect_err(|error| {
            tracing::warn!("no session bus, so no menu bar and no single instance: {error}");
        })
        .ok();
    let claimed = session
        .as_ref()
        .map(|session| instance::claim(session, info.id))
        .transpose()
        .inspect_err(|error| tracing::warn!("cannot claim {}: {error}", info.id))
        .ok()
        .flatten();
    let instance = match (claimed, &session) {
        (Some(Claim::Running), Some(session)) => {
            return instance::forward(session, info.id, Request::from_launch(arguments)?);
        }
        (Some(Claim::First(instance)), _) => Some(instance),
        _ => None,
    };
    let link = match display {
        Some(display) => AppConnection::connect_on(display, info.id)?,
        None => AppConnection::connect(info.id)?,
    };
    let mut cx = Cx::new(info, link, session);
    let program = P::start(&mut cx, arguments)?;
    if cx.windows().next().is_none() {
        return Ok(());
    }
    let menu = cx.session().cloned().and_then(|session| {
        AppMenu::serve(session, assemble(info, program.menus(&cx), true))
            .inspect_err(|error| tracing::warn!("{error}"))
            .ok()
    });
    cx.menus_changed = false;
    let mut runner = Runner {
        program,
        cx,
        menu,
        instance,
        sources: RefCell::new(Vec::new()),
    };
    runner.run()
}

fn release_freed_memory() {
    unsafe {
        libc::malloc_trim(0);
    }
}

fn timeout(deadline: Instant) -> Timespec {
    let wait = deadline.saturating_duration_since(Instant::now());
    Timespec {
        tv_sec: i64::try_from(wait.as_secs()).unwrap_or(i64::MAX),
        tv_nsec: i64::from(wait.subsec_nanos()),
    }
}

impl<P: Program> Runner<P> {
    fn run(&mut self) -> Result<(), Error> {
        loop {
            if !self.cx.windows.is_empty() {
                bande_toolkit::run_surfaces(self)?;
            }
            if self.cx.quitting || !self.cx.link.keeps_running() {
                return Ok(());
            }
            if self.cx.windows.is_empty() {
                release_freed_memory();
            }
            while self.cx.windows.is_empty() && !self.cx.quitting {
                self.wait_without_windows()?;
            }
        }
    }

    fn wait_without_windows(&mut self) -> Result<(), WindowError> {
        self.publish_menus();
        self.cx.link.flush()?;
        let deadline = Surfaces::deadline(self);
        let (link_ready, ready) = {
            let link = self.cx.link.fd();
            let sources = Surfaces::sources(self);
            let mut fds = Vec::with_capacity(1 + sources.len());
            fds.push(PollFd::new(&link, PollFlags::IN));
            for fd in &sources {
                fds.push(PollFd::new(fd, PollFlags::IN | PollFlags::PRI));
            }
            match poll(&mut fds, deadline.map(timeout).as_ref()) {
                Ok(_) | Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(io::Error::from(error).into()),
            }
            let ready: Vec<usize> = (0..sources.len())
                .filter(|at| !fds[1 + at].revents().is_empty())
                .collect();
            (!fds[0].revents().is_empty(), ready)
        };
        if link_ready {
            let requests = self.cx.link.read()?;
            self.serve_requests(requests);
        }
        for index in ready {
            Surfaces::source_ready(self, index);
        }
        let now = Instant::now();
        if deadline.is_some_and(|deadline| deadline <= now) {
            Surfaces::tick(self, now);
        }
        Ok(())
    }

    fn serve_requests(&mut self, requests: Vec<AppRequest>) {
        for request in requests {
            match request {
                AppRequest::Reopen(token) => self.activated(Request {
                    arguments: Vec::new(),
                    directory: None,
                    token: Some(token),
                }),
                AppRequest::Key(input) => self.key_without_window(&input),
                AppRequest::Quit => self.quit(),
            }
        }
        self.publish_menus();
    }

    fn publish_menus(&mut self) {
        if !std::mem::take(&mut self.cx.menus_changed) {
            return;
        }
        let Some(menu) = &self.menu else {
            return;
        };
        let windowed = !self.cx.windows.is_empty();
        let entries = assemble(self.cx.info, self.program.menus(&self.cx), windowed);
        if let Err(error) = menu.update(entries) {
            tracing::warn!("cannot update the menus: {error}");
        }
    }

    fn is_settings(&self, window: WindowId) -> bool {
        self.cx
            .index(window)
            .is_some_and(|index| matches!(self.cx.views[index].1, Content::Settings(_)))
    }

    fn close(&mut self, window: WindowId) {
        if self.is_settings(window) || self.program.should_close(&mut self.cx, window) {
            self.cx.close(window);
        }
    }

    fn press(&mut self, window: WindowId, shortcut: &str) {
        let (Some(index), Some((key, modifiers))) =
            (self.cx.index(window), parse_shortcut(shortcut))
        else {
            return;
        };
        self.deliver(
            index,
            window,
            WindowEvent::Input(Event::Key { key, modifiers }),
        );
    }

    fn quit(&mut self) {
        if self.program.should_quit(&mut self.cx) {
            self.cx.quit();
        }
    }

    fn perform(&mut self, window: Option<WindowId>, action: &str) {
        if let Some(shortcut) = action.strip_prefix(KEY_ACTION) {
            if let Some(window) = window {
                self.press(window, shortcut);
            }
            return;
        }
        let Some(standard) = Standard::parse(action) else {
            self.program.command(&mut self.cx, window, action);
            self.cx.menus_changed = true;
            return;
        };
        let shown = window
            .and_then(|window| self.cx.index(window))
            .map(|index| &mut self.cx.windows[index]);
        let result = match (standard, shown) {
            (Standard::Hide | Standard::Minimize, Some(window)) => window.minimize(),
            (Standard::Zoom, Some(window)) => window.toggle_zoom(),
            (Standard::Quit, _) => {
                self.quit();
                Ok(())
            }
            (Standard::CloseWindow, Some(_)) => {
                if let Some(window) = window
                    && (self.is_settings(window) || !self.program.close_tab(&mut self.cx, window))
                {
                    self.close(window);
                }
                Ok(())
            }
            (Standard::Settings, _) => {
                self.cx.open_settings();
                Ok(())
            }
            (_, None) => Ok(()),
        };
        if let Err(error) = result {
            tracing::warn!("cannot {action}: {error}");
        }
    }

    fn deliver(&mut self, index: usize, window: WindowId, event: WindowEvent) -> bool {
        let handled = self.cx.views[index]
            .1
            .view()
            .event(&mut self.cx.windows[index], event);
        if handled {
            self.cx.windows[index].request_redraw();
            self.changed(index, window);
        }
        handled
    }

    fn changed(&mut self, index: usize, window: WindowId) {
        if matches!(self.cx.views[index].1, Content::App(_)) {
            self.program.changed(&mut self.cx, window);
        }
    }

    fn key(&mut self, index: usize, window: WindowId, input: Event) {
        let settings = self.cx.info.settings.is_some();
        if let Some(standard) = self.cx.info.keys.standard(&input, settings) {
            return self.perform(Some(window), standard.action());
        }
        if self.deliver(index, window, WindowEvent::Input(input.clone())) {
            return;
        }
        self.menu_shortcut(Some(window), &input);
    }

    fn key_without_window(&mut self, input: &Event) {
        let settings = self.cx.info.settings.is_some();
        match self.cx.info.keys.standard(input, settings) {
            Some(standard) => self.perform(self.cx.focused(), standard.action()),
            None => self.menu_shortcut(self.cx.focused(), input),
        }
    }

    fn menu_shortcut(&mut self, window: Option<WindowId>, input: &Event) {
        let windowed = !self.cx.windows.is_empty();
        let menus = assemble(self.cx.info, self.program.menus(&self.cx), windowed);
        if let Some(action) = shortcut_action(&menus, &|shortcut| shortcut_pressed(input, shortcut))
        {
            self.perform(window, &action);
        }
    }

    fn activated(&mut self, request: Request) {
        let newest = self.cx.views.last().map(|(id, _)| *id);
        if !request.arguments.is_empty() {
            let directory = request.directory.as_deref().unwrap_or(Path::new("/"));
            self.program
                .open(&mut self.cx, request.arguments.clone(), directory);
        } else if self.cx.windows.is_empty() {
            self.program.reopen(&mut self.cx);
        }
        let opened = self
            .cx
            .views
            .last()
            .map(|(id, _)| *id)
            .filter(|id| Some(*id) != newest);
        if let Some(window) = opened.or_else(|| self.cx.focused()) {
            self.cx.raise(window, request.token);
        }
    }

    fn focus(&mut self, index: usize, window: WindowId, focused: bool) {
        if focused {
            self.cx.focused = Some(window);
            self.cx.menus_changed = true;
        }
        self.cx.views[index].1.view().set_active(focused);
        self.cx.windows[index].request_redraw();
    }
}

impl<P: Program> Surfaces for Runner<P> {
    fn windows(&self) -> &[Window] {
        &self.cx.windows
    }

    fn windows_mut(&mut self) -> &mut [Window] {
        &mut self.cx.windows
    }

    fn scene(&mut self, index: usize) -> Scene {
        self.cx.views[index]
            .1
            .view()
            .scene(&mut self.cx.windows[index])
    }

    fn event(&mut self, index: usize, event: WindowEvent) {
        if self.cx.windows[index].is_closed() {
            return;
        }
        let window = self.cx.views[index].0;
        match event {
            WindowEvent::CloseRequested => self.close(window),
            WindowEvent::QuitRequested => self.quit(),
            WindowEvent::Focus(focused) => self.focus(index, window, focused),
            WindowEvent::AppearanceChanged(appearance) => {
                self.cx.views[index].1.view().set_appearance(appearance);
                self.cx.windows[index].request_redraw();
            }
            WindowEvent::Input(input @ Event::Key { .. }) => self.key(index, window, input),
            WindowEvent::MenuAction(action) => match Standard::parse(&action) {
                Some(_) => self.perform(Some(window), &action),
                None => {
                    if !self.deliver(index, window, WindowEvent::MenuAction(action.clone())) {
                        self.perform(Some(window), &action);
                    }
                }
            },
            event => {
                self.deliver(index, window, event);
            }
        }
        self.publish_menus();
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        let mut fds = Vec::new();
        let mut sources = Vec::new();
        if let Some(menu) = &self.menu {
            fds.push(menu.fd());
            sources.push(Source::Menu);
        }
        if let Some(instance) = &self.instance {
            fds.push(instance.fd());
            sources.push(Source::Instance);
        }
        for (index, fd) in self.program.sources().into_iter().enumerate() {
            fds.push(fd);
            sources.push(Source::Program(index));
        }
        for ((window, content), shown) in self.cx.views.iter().zip(&self.cx.windows) {
            if shown.is_closed() {
                continue;
            }
            for (index, fd) in content.view_ref().sources().into_iter().enumerate() {
                fds.push(fd);
                sources.push(Source::View(*window, index));
            }
        }
        *self.sources.borrow_mut() = sources;
        fds
    }

    fn source_ready(&mut self, index: usize) {
        let source = self.sources.borrow().get(index).copied();
        match source {
            Some(Source::Menu) => {
                let actions = self
                    .menu
                    .as_mut()
                    .map(AppMenu::take_actions)
                    .unwrap_or_default();
                for action in actions {
                    self.perform(self.cx.focused(), &action);
                }
            }
            Some(Source::Instance) => {
                let requests = self
                    .instance
                    .as_mut()
                    .map(Instance::take_requests)
                    .unwrap_or_default();
                for request in requests {
                    self.activated(request);
                }
            }
            Some(Source::Program(index)) => self.program.source_ready(&mut self.cx, index),
            Some(Source::View(window, source)) => {
                if let Some(index) = self.cx.index(window)
                    && self.cx.views[index]
                        .1
                        .view()
                        .source_ready(&mut self.cx.windows[index], source)
                {
                    self.cx.windows[index].request_redraw();
                    self.changed(index, window);
                }
            }
            None => {}
        }
        self.publish_menus();
    }

    fn deadline(&self) -> Option<Instant> {
        self.cx
            .views
            .iter()
            .zip(&self.cx.windows)
            .filter(|(_, window)| !window.is_closed())
            .filter_map(|((_, content), _)| content.view_ref().deadline())
            .chain(self.program.deadline())
            .min()
    }

    fn tick(&mut self, now: Instant) {
        if self
            .program
            .deadline()
            .is_some_and(|deadline| deadline <= now)
        {
            self.program.tick(&mut self.cx, now);
        }
        let due: Vec<WindowId> = self
            .cx
            .views
            .iter()
            .filter(|(_, content)| {
                content
                    .view_ref()
                    .deadline()
                    .is_some_and(|deadline| deadline <= now)
            })
            .map(|(window, _)| *window)
            .collect();
        for window in due {
            if let Some(index) = self.cx.index(window)
                && self.cx.views[index]
                    .1
                    .view()
                    .tick(&mut self.cx.windows[index], now)
            {
                self.cx.windows[index].request_redraw();
                self.changed(index, window);
            }
        }
        self.publish_menus();
    }

    fn animating(&self, index: usize) -> bool {
        self.cx.views[index].1.view_ref().animating()
    }

    fn accessibility(&self, index: usize) -> Option<TreeUpdate> {
        self.cx.views[index].1.view_ref().accessibility()
    }

    fn text_input_area(&self, index: usize) -> Option<Rect> {
        self.cx.views[index].1.view_ref().text_input_area()
    }

    fn closed(&mut self) -> bool {
        match self.cx.link.take_requests() {
            Ok(requests) => self.serve_requests(requests),
            Err(error) => tracing::warn!("cannot read the app's requests: {error}"),
        }
        for (window, view) in self.cx.remove_closed() {
            self.program.window_closed(&mut self.cx, window, view);
        }
        self.publish_menus();
        self.cx.quitting || self.cx.windows.is_empty()
    }
}
