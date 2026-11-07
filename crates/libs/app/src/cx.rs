use std::path::PathBuf;

use bande_toolkit::{AppConnection, Appearance, Role, Window, WindowError};
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
    pub(crate) link: AppConnection,
    pub(crate) windows: Vec<Window>,
    pub(crate) views: Vec<(WindowId, Content<P::View>)>,
    next: u32,
    pub(crate) focused: Option<WindowId>,
    panels: Vec<WindowId>,
    pub(crate) quitting: bool,
    pub(crate) menus_changed: bool,
    session: Option<Connection>,
    background: bool,
}

impl<P: Program> Cx<P> {
    pub(crate) fn new(
        info: &'static Info,
        link: AppConnection,
        session: Option<Connection>,
        background: bool,
    ) -> Self {
        Self {
            info,
            link,
            windows: Vec::new(),
            views: Vec::new(),
            next: 0,
            focused: None,
            panels: Vec::new(),
            quitting: false,
            menus_changed: false,
            session,
            background,
        }
    }

    pub fn launched_in_background(&self) -> bool {
        self.background
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
            Kind::Main | Kind::Secondary(_) => Role::toplevel(title, self.info.id),
            Kind::Dialog(_) => Role::dialog(title, self.info.id),
            Kind::Utility(_) => Role::utility(title, self.info.id),
            Kind::Panel(_) => Role::panel(title, self.info.id),
        }
        .tagged(kind.tag());
        self.link.keep_running();
        let mut window = match self.windows.first() {
            Some(beside) => beside.open_beside(role, size)?,
            None => self.link.open(role, size)?,
        };
        let panel = matches!(kind, Kind::Panel(_));
        window.set_menu_button(!panel);
        window.export();
        let id = WindowId(self.next);
        self.next += 1;
        let content = content(window.appearance());
        self.windows.push(window);
        self.views.push((id, content));
        if panel {
            self.panels.push(id);
        } else {
            self.focused.get_or_insert(id);
        }
        self.menus_changed = true;
        Ok(id)
    }

    pub fn close(&mut self, window: WindowId) {
        if let Some(index) = self.index(window) {
            self.windows[index].close();
            self.menus_changed = true;
        }
    }

    pub fn print(&self, title: &str, document: crate::Document) {
        crate::print(title, self.info.id, document);
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
            Kind::Utility("settings"),
            &(settings.title)(),
            settings.size,
            |appearance| Content::Settings(view(appearance)),
        ) {
            tracing::warn!("cannot open the settings: {error}");
        }
    }

    pub fn raise(&mut self, window: WindowId, token: Option<String>) {
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

    pub(crate) fn sizing(&self) -> (bool, Option<bool>) {
        self.focused().map_or((false, None), |window| {
            let zoomable = self
                .index(window)
                .is_some_and(|index| self.windows[index].is_resizable());
            (zoomable, self.full_screen(window))
        })
    }

    pub(crate) fn full_screen(&self, window: WindowId) -> Option<bool> {
        let index = self
            .index(window)
            .filter(|_| !self.panels.contains(&window))?;
        let window = &self.windows[index];
        let full_screen = window.toplevel_state().fullscreen;
        (full_screen || window.is_resizable()).then_some(full_screen)
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

    pub(crate) fn remove_closed(&mut self) -> Vec<(WindowId, P::View)> {
        let mut removed = Vec::new();
        let mut index = 0;
        while index < self.windows.len() {
            if self.windows[index].is_closed() {
                self.windows.remove(index);
                let (id, content) = self.views.remove(index);
                self.panels.retain(|panel| *panel != id);
                if let Content::App(view) = content {
                    removed.push((id, view));
                }
                self.menus_changed = true;
            } else {
                index += 1;
            }
        }
        removed
    }
}
