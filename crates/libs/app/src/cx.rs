use std::os::unix::net::UnixStream;

use bande_toolkit::{Appearance, Role, Window, WindowError};
use zbus::blocking::Connection;

use crate::{Info, Kind, Program};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(u32);

pub struct Cx<P: Program> {
    pub(crate) info: &'static Info,
    display: Option<UnixStream>,
    pub(crate) windows: Vec<Window>,
    pub(crate) views: Vec<(WindowId, P::View)>,
    next: u32,
    pub(crate) focused: Option<WindowId>,
    pub(crate) quitting: bool,
    pub(crate) menus_changed: bool,
    session: Option<Connection>,
}

impl<P: Program> Cx<P> {
    pub(crate) fn new(
        info: &'static Info,
        display: Option<UnixStream>,
        session: Option<Connection>,
    ) -> Self {
        Self {
            info,
            display,
            windows: Vec::new(),
            views: Vec::new(),
            next: 0,
            focused: None,
            quitting: false,
            menus_changed: false,
            session,
        }
    }

    pub fn open(
        &mut self,
        kind: Kind,
        title: &str,
        size: (u32, u32),
        view: impl FnOnce(Appearance) -> P::View,
    ) -> Result<WindowId, WindowError> {
        let role = match kind {
            Kind::Main => Role::toplevel(title, self.info.id),
            Kind::Dialog => Role::dialog(title, self.info.id),
            Kind::Utility => Role::utility(title, self.info.id),
        };
        let window = match (self.windows.first(), self.display.take()) {
            (Some(beside), _) => beside.open_beside(role, size)?,
            (None, Some(display)) => Window::open_on(display, role, size)?,
            (None, None) => Window::open(role, size)?,
        };
        let id = WindowId(self.next);
        self.next += 1;
        let view = view(window.appearance());
        self.windows.push(window);
        self.views.push((id, view));
        self.focused.get_or_insert(id);
        self.menus_changed = true;
        Ok(id)
    }

    pub fn close(&mut self, window: WindowId) {
        if let Some(index) = self.index(window) {
            self.windows[index].close();
            self.menus_changed = true;
        }
    }

    pub fn quit(&mut self) {
        self.quitting = true;
    }

    pub fn focused(&self) -> Option<WindowId> {
        self.focused
            .filter(|window| self.index(*window).is_some())
            .or_else(|| self.windows().last())
    }

    pub fn window(&mut self, window: WindowId) -> Option<(&mut Window, &mut P::View)> {
        let index = self.index(window)?;
        Some((&mut self.windows[index], &mut self.views[index].1))
    }

    pub fn windows(&self) -> impl Iterator<Item = WindowId> + '_ {
        self.windows
            .iter()
            .zip(&self.views)
            .filter(|(window, _)| !window.is_closed())
            .map(|(_, (id, _))| *id)
    }

    pub fn refresh_menus(&mut self) {
        self.menus_changed = true;
    }

    pub fn session(&self) -> Option<&Connection> {
        self.session.as_ref()
    }

    pub(crate) fn index(&self, window: WindowId) -> Option<usize> {
        self.views
            .iter()
            .position(|(id, _)| *id == window)
            .filter(|index| !self.windows[*index].is_closed())
    }

    pub(crate) fn remove_closed(&mut self) {
        let mut index = 0;
        while index < self.windows.len() {
            if self.windows[index].is_closed() {
                self.windows.remove(index);
                self.views.remove(index);
                self.menus_changed = true;
            } else {
                index += 1;
            }
        }
    }
}
