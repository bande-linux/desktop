use std::cell::RefCell;
use std::rc::Rc;

use crate::menu_search::{MenuSearch, NAMESPACE};
use bande_toolkit::ui::Menu;
use bande_toolkit::{
    App, Edges, KeyboardFocus, LayerKind, LayerOptions, Role, Scene, TextSystem, Window,
    WindowEvent,
};

#[derive(Default)]
pub struct Handoff {
    pub menu: Menu,
    pub chosen: Vec<String>,
}

pub type SharedHandoff = Rc<RefCell<Handoff>>;

pub fn search_role() -> Role {
    Role::Layer(LayerOptions {
        layer: LayerKind::Overlay,
        anchor: Edges::ALL,
        exclusive_zone: -1,
        keyboard: KeyboardFocus::Exclusive,
        namespace: NAMESPACE.to_owned(),
    })
}

pub struct SearchCard {
    view: MenuSearch,
    handoff: SharedHandoff,
}

impl SearchCard {
    pub fn new(window: &mut Window, text: TextSystem, handoff: SharedHandoff) -> Self {
        if let Err(error) = window.hide() {
            tracing::warn!("cannot hide the menu search: {error}");
        }
        Self {
            view: MenuSearch::new(window.appearance(), text),
            handoff,
        }
    }

    fn close(window: &mut Window) {
        if window.is_visible()
            && let Err(error) = window.hide()
        {
            tracing::warn!("cannot hide the menu search: {error}");
        }
    }

    fn toggle(&mut self, window: &mut Window) {
        if window.is_visible() {
            Self::close(window);
            return;
        }
        let menu = self.handoff.borrow().menu.clone();
        self.view.open(menu);
        if let Err(error) = window.show() {
            tracing::warn!("cannot show the menu search: {error}");
        }
        window.request_redraw();
    }

    fn settle(&mut self, window: &mut Window) {
        if let Some(action) = self.view.take_chosen() {
            self.handoff.borrow_mut().chosen.push(action);
            Self::close(window);
        }
        if self.view.take_closing() {
            Self::close(window);
        }
        window.request_redraw();
    }
}

impl App for SearchCard {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.view.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::FocusShell(namespace) if namespace == NAMESPACE => self.toggle(window),
            WindowEvent::Input(input) if window.is_visible() => {
                self.view.handle(&input);
                window.set_cursor(self.view.cursor());
                self.settle(window);
            }
            WindowEvent::Accessibility(request) => {
                self.view.perform(&request);
                self.settle(window);
            }
            WindowEvent::Focus(false) => Self::close(window),
            WindowEvent::AppearanceChanged(appearance) => {
                self.view.set_appearance(appearance);
                window.request_redraw();
            }
            _ => {}
        }
    }

    fn animating(&self) -> bool {
        self.view.is_animating()
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.view.accessibility())
    }
}
