use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use bande_toolkit::{Appearance, Role, Window, WindowError};
use zbus::blocking::Connection;

use crate::{Info, Kind, Program, View};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(u32);

pub(crate) enum Content<V> {
    App(V),
    Settings(Box<dyn View>),
}

impl<V: View> Content<V> {
    pub(crate) fn view(&mut self) -> &mut dyn View {
        match self {
            Self::App(view) => view,
            Self::Settings(view) => view.as_mut(),
        }
    }

    pub(crate) fn view_ref(&self) -> &dyn View {
        match self {
            Self::App(view) => view,
            Self::Settings(view) => view.as_ref(),
        }
    }
}

pub struct Cx<P: Program> {
    pub(crate) info: &'static Info,
    display: Option<UnixStream>,
    pub(crate) windows: Vec<Window>,
    pub(crate) views: Vec<(WindowId, Content<P::View>)>,
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
        self.open_content(kind, title, size, |appearance| {
            Content::App(view(appearance))
        })
    }

    pub(crate) fn open_content(
        &mut self,
        kind: Kind,
        title: &str,
        size: (u32, u32),
        content: impl FnOnce(Appearance) -> Content<P::View>,
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
        let content = content(window.appearance());
        self.windows.push(window);
        self.views.push((id, content));
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

    pub fn open_settings(&mut self) {
        let Some(settings) = &self.info.settings else {
            return;
        };
        let shown = self
            .views
            .iter()
            .zip(&self.windows)
            .find(|((_, content), window)| {
                matches!(content, Content::Settings(_)) && !window.is_closed()
            })
            .map(|((id, _), _)| *id);
        if let Some(window) = shown {
            return self.raise(window, None);
        }
        let view = settings.view;
        if let Err(error) = self.open_content(
            Kind::Main,
            &(settings.title)(),
            settings.size,
            |appearance| Content::Settings(view(appearance)),
        ) {
            tracing::warn!("cannot open the settings: {error}");
        }
    }

    pub(crate) fn raise(&mut self, window: WindowId, token: Option<String>) {
        let Some(index) = self.index(window) else {
            return;
        };
        let id = self.info.id;
        let window = &mut self.windows[index];
        let token = match token {
            Some(token) => Ok(token),
            None => window.activation_token(id),
        };
        if let Err(error) = token.and_then(|token| window.activate(&token)) {
            tracing::warn!("cannot raise the window: {error}");
        }
    }

    pub fn quit(&mut self) {
        self.quitting = true;
    }

    pub fn focused(&self) -> Option<WindowId> {
        self.focused
            .filter(|window| self.index(*window).is_some())
            .or_else(|| self.shown().last().map(|(id, _)| *id))
    }

    pub fn window(&mut self, window: WindowId) -> Option<(&mut Window, &mut P::View)> {
        let index = self.index(window)?;
        match &mut self.views[index].1 {
            Content::App(view) => Some((&mut self.windows[index], view)),
            Content::Settings(_) => None,
        }
    }

    pub fn view(&self, window: WindowId) -> Option<&P::View> {
        match &self.views[self.index(window)?].1 {
            Content::App(view) => Some(view),
            Content::Settings(_) => None,
        }
    }

    pub fn windows(&self) -> impl Iterator<Item = WindowId> + '_ {
        self.shown()
            .filter(|(_, content)| matches!(content, Content::App(_)))
            .map(|(id, _)| *id)
    }

    pub fn refresh_menus(&mut self) {
        self.menus_changed = true;
    }

    pub fn session(&self) -> Option<&Connection> {
        self.session.as_ref()
    }

    pub fn state_file(&self, name: &str) -> Option<PathBuf> {
        bande_xdg::state_home().map(|state| state.join("bande").join(name))
    }

    fn shown(&self) -> impl DoubleEndedIterator<Item = &(WindowId, Content<P::View>)> + '_ {
        self.windows
            .iter()
            .zip(&self.views)
            .filter(|(window, _)| !window.is_closed())
            .map(|(_, view)| view)
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
