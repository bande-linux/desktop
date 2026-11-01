use std::cell::RefCell;
use std::os::fd::BorrowedFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Instant;

use accesskit::TreeUpdate;
use bande_app_menu::{AppMenu, Standard, shortcut_action};
use bande_toolkit::ui::{Event, shortcut_pressed};
use bande_toolkit::{Rect, Scene, Surfaces, Window, WindowEvent};

use crate::instance::{self, Claim, Instance, Request};
use crate::menus::assemble;
use crate::{Cx, Error, Info, Program, View, WindowId};

#[derive(Debug, Clone, Copy)]
enum Source {
    Menu,
    Instance,
    Program(usize),
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
    let mut cx = Cx::new(info, display, session);
    let program = P::start(&mut cx, arguments)?;
    if cx.windows().next().is_none() {
        return Ok(());
    }
    let menu = cx.session().cloned().and_then(|session| {
        AppMenu::serve(session, info.id, assemble(info, program.menus(&cx)))
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
    bande_toolkit::run_surfaces(&mut runner)?;
    Ok(())
}

impl<P: Program> Runner<P> {
    fn publish_menus(&mut self) {
        if !std::mem::take(&mut self.cx.menus_changed) {
            return;
        }
        let Some(menu) = &self.menu else {
            return;
        };
        let entries = assemble(self.cx.info, self.program.menus(&self.cx));
        if let Err(error) = menu.update(entries) {
            tracing::warn!("cannot update the menus: {error}");
        }
    }

    fn close(&mut self, window: WindowId) {
        if self.program.should_close(&mut self.cx, window) {
            self.cx.close(window);
        }
    }

    fn quit(&mut self) {
        if self.program.should_quit(&mut self.cx) {
            self.cx.quit();
        }
    }

    fn perform(&mut self, window: WindowId, action: &str) {
        let Some(standard) = Standard::parse(action) else {
            self.program.command(&mut self.cx, window, action);
            self.cx.menus_changed = true;
            return;
        };
        let result = match (standard, self.cx.window(window)) {
            (Standard::Hide | Standard::Minimize, Some((window, _))) => window.minimize(),
            (Standard::Zoom, Some((window, _))) => window.toggle_zoom(),
            (Standard::Quit, _) => {
                self.quit();
                Ok(())
            }
            (Standard::CloseWindow, _) => {
                self.close(window);
                Ok(())
            }
            (Standard::Settings, _) | (_, None) => Ok(()),
        };
        if let Err(error) = result {
            tracing::warn!("cannot {action}: {error}");
        }
    }

    fn deliver(&mut self, index: usize, window: WindowId, event: WindowEvent) -> bool {
        let handled = self.cx.views[index]
            .1
            .event(&mut self.cx.windows[index], event);
        if handled {
            self.cx.windows[index].request_redraw();
            self.program.changed(&mut self.cx, window);
        }
        handled
    }

    fn key(&mut self, index: usize, window: WindowId, input: Event) {
        if let Some(standard) = self.cx.info.keys.standard(&input) {
            return self.perform(window, standard.action());
        }
        if self.deliver(index, window, WindowEvent::Input(input.clone())) {
            return;
        }
        let menus = assemble(self.cx.info, self.program.menus(&self.cx));
        if let Some(action) =
            shortcut_action(&menus, &|shortcut| shortcut_pressed(&input, shortcut))
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
        }
        let opened = self
            .cx
            .views
            .last()
            .map(|(id, _)| *id)
            .filter(|id| Some(*id) != newest);
        let id = self.cx.info.id;
        let Some((window, _)) = opened
            .or_else(|| self.cx.focused())
            .and_then(|target| self.cx.window(target))
        else {
            return;
        };
        let token = match request.token {
            Some(token) => Ok(token),
            None => window.activation_token(id),
        };
        if let Err(error) = token.and_then(|token| window.activate(&token)) {
            tracing::warn!("cannot raise the window: {error}");
        }
    }

    fn focus(&mut self, index: usize, window: WindowId, focused: bool) {
        if focused {
            self.cx.focused = Some(window);
            self.cx.menus_changed = true;
        }
        self.cx.views[index].1.set_active(focused);
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
        self.cx.views[index].1.scene(&mut self.cx.windows[index])
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
                self.cx.views[index].1.set_appearance(appearance);
                self.cx.windows[index].request_redraw();
            }
            WindowEvent::Input(input @ Event::Key { .. }) => self.key(index, window, input),
            WindowEvent::MenuAction(action) => match Standard::parse(&action) {
                Some(_) => self.perform(window, &action),
                None => {
                    if !self.deliver(index, window, WindowEvent::MenuAction(action.clone())) {
                        self.perform(window, &action);
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
                    if let Some(window) = self.cx.focused() {
                        self.perform(window, &action);
                    }
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
            None => {}
        }
        self.publish_menus();
    }

    fn deadline(&self) -> Option<Instant> {
        self.program.deadline()
    }

    fn tick(&mut self, now: Instant) {
        if self
            .program
            .deadline()
            .is_some_and(|deadline| deadline <= now)
        {
            self.program.tick(&mut self.cx, now);
        }
        self.publish_menus();
    }

    fn animating(&self, index: usize) -> bool {
        self.cx.views[index].1.animating()
    }

    fn accessibility(&self, index: usize) -> Option<TreeUpdate> {
        self.cx.views[index].1.accessibility()
    }

    fn text_input_area(&self, index: usize) -> Option<Rect> {
        self.cx.views[index].1.text_input_area()
    }

    fn closed(&mut self) -> bool {
        self.cx.remove_closed();
        self.publish_menus();
        self.cx.quitting || self.cx.windows.is_empty()
    }
}
