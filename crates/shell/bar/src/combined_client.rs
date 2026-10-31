use std::os::fd::BorrowedFd;
use std::time::Instant;

use crate::L10N;
use crate::menu_bar::StatusView;
use crate::rail::{EXPANDED_WIDTH, RailView, bar_width, owns_node, surface_size};
use accesskit::{Node, NodeId, Role as AccessRole, TreeInfo, TreeUpdate};
use bande_toolkit::ui::{Cursor, Event};
use bande_toolkit::{
    App, Edges, Item, KeyboardFocus, LayerKind, LayerOptions, Point, Rect, Role, Scene, Shape,
    Window, WindowEvent,
};

use crate::dock_client::Dock;
use crate::menu_bar_client::MenuBarClient;

const BAR_NAMESPACE: &str = "bar";
const MENU_BAR_NAMESPACE: &str = "menubar";
const ROOT: NodeId = NodeId(1 << 41);
const HIDDEN_SHADOW: f32 = 8.0;
const EDGE_GRAB: f32 = 4.0;
const UNREAD_SLIVER: (f32, f32) = (2.0, 24.0);

pub fn combined_role(icon_size: f32) -> (Role, (u32, u32)) {
    (
        Role::Layer(LayerOptions {
            layer: LayerKind::Top,
            anchor: Edges {
                left: true,
                top: true,
                bottom: true,
                right: false,
            },
            exclusive_zone: bar_width(icon_size).round() as i32,
            keyboard: KeyboardFocus::OnDemand,
            namespace: BAR_NAMESPACE.to_owned(),
        }),
        surface_size(icon_size),
    )
}

pub struct CombinedClient {
    dock: Dock,
    status: MenuBarClient<RailView>,
    status_open: bool,
    resizing: Option<f32>,
}

fn pointer(event: &WindowEvent) -> bool {
    matches!(
        event,
        WindowEvent::Input(
            Event::PointerDown(_)
                | Event::PointerUp(_)
                | Event::PointerMove(_)
                | Event::PointerEnter(_)
                | Event::PointerLeave
                | Event::Scroll(_)
        )
    )
}

impl CombinedClient {
    pub fn new(dock: Dock, status: MenuBarClient<RailView>) -> Self {
        Self {
            dock,
            status,
            status_open: false,
            resizing: None,
        }
    }

    fn on_inner_edge(&mut self, x: f32) -> bool {
        let width = self.status.view_mut().width();
        (width - EDGE_GRAB..width).contains(&x)
    }

    fn resize(&mut self, window: &mut Window, event: &WindowEvent) -> bool {
        match (event, self.resizing) {
            (WindowEvent::Input(Event::PointerDown(pointer)), None)
                if pointer.is_primary() && self.on_inner_edge(pointer.position.x) =>
            {
                self.resizing = Some(pointer.position.x);
            }
            (WindowEvent::Input(Event::PointerMove(pointer)), Some(_)) => {
                self.resizing = Some(pointer.position.x);
            }
            (WindowEvent::Input(Event::PointerUp(pointer)), Some(_)) => {
                self.resizing = None;
                let collapsed = bar_width(self.dock.icon_size());
                let names = pointer.position.x > (collapsed + EXPANDED_WIDTH) / 2.0;
                if names != self.dock.names() {
                    self.dock.set_names(names);
                    window.request_redraw();
                }
            }
            _ => return false,
        }
        window.set_cursor(Cursor::ColumnResize);
        true
    }

    fn route(&mut self, window: &mut Window, event: WindowEvent) {
        match &event {
            _ if pointer(&event) => {
                self.dock.event(window, event.clone());
                self.status.event(window, event);
            }
            WindowEvent::FocusShell(namespace) if namespace == MENU_BAR_NAMESPACE => {
                self.status.event(window, event);
            }
            WindowEvent::Input(_) | WindowEvent::Drag(_) | WindowEvent::FocusShell(_) => {
                self.dock.event(window, event);
            }
            WindowEvent::MenuAction(_) | WindowEvent::MenuClosed if self.dock.has_menu() => {
                self.dock.event(window, event);
            }
            WindowEvent::MenuAction(_) | WindowEvent::MenuClosed => {
                self.status.event(window, event);
            }
            WindowEvent::Accessibility(request) if owns_node(request.target_node) => {
                self.status.event(window, event);
            }
            WindowEvent::Accessibility(_) => self.dock.event(window, event),
            _ => {
                self.dock.event(window, event.clone());
                self.status.event(window, event);
            }
        }
    }
}

impl App for CombinedClient {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let view = self.status.view_mut();
        view.set_icon_size(self.dock.icon_size());
        view.set_expanded(self.dock.names());
        self.dock.set_rail(view.apps_region(height as f32));
        let mut scene = self.status.scene(window);
        let apps = self.dock.scene(window);
        scene.width = width;
        scene.items.extend(apps.items);
        let revealed = self.dock.revealed(Instant::now());
        if revealed < 1.0 {
            let view = self.status.view_mut();
            let hidden = -(view.width() + HIDDEN_SHADOW) * (1.0 - revealed);
            let mut items = vec![Item::Translate {
                offset: Point::new(hidden.round(), 0.0),
                items: std::mem::take(&mut scene.items),
            }];
            if view.unread() && revealed <= 0.0 {
                items.push(Item::Fill {
                    shape: Shape::rect(Rect::new(
                        0.0,
                        height as f32 - UNREAD_SLIVER.1,
                        UNREAD_SLIVER.0,
                        UNREAD_SLIVER.1,
                    )),
                    paint: window.appearance().palette().accent.into(),
                });
            }
            scene.items = items;
        }
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        if self.resize(window, &event) {
            return;
        }
        let hovered = match &event {
            WindowEvent::Input(Event::PointerMove(pointer)) => Some(pointer.position.x),
            _ => None,
        };
        self.route(window, event);
        if hovered.is_some_and(|x| self.on_inner_edge(x)) {
            window.set_cursor(Cursor::ColumnResize);
        }
        let open = self.status.view_mut().open().is_some();
        if open != self.status_open {
            self.status_open = open;
            self.dock.set_menu_open(open);
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        let mut sources = self.dock.sources();
        sources.extend(self.status.sources());
        sources
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        let docked = self.dock.sources().len();
        if index < docked {
            self.dock.source_ready(window, index);
        } else {
            self.status.source_ready(window, index - docked);
        }
    }

    fn deadline(&self) -> Option<Instant> {
        [self.dock.deadline(), self.status.deadline()]
            .into_iter()
            .flatten()
            .min()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        if self.dock.deadline().is_some_and(|deadline| deadline <= now) {
            self.dock.tick(window, now);
        }
        if self
            .status
            .deadline()
            .is_some_and(|deadline| deadline <= now)
        {
            self.status.tick(window, now);
        }
    }

    fn animating(&self) -> bool {
        self.dock.animating() || self.status.animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        let apps = self.dock.accessibility()?;
        let status = self.status.accessibility()?;
        let mut root = Node::new(AccessRole::Toolbar);
        root.set_label(L10N.tr("bar"));
        root.set_children(
            [&status, &apps]
                .into_iter()
                .filter_map(|tree| tree.tree.as_ref().map(|info| info.root))
                .collect::<Vec<_>>(),
        );
        let mut nodes = vec![(ROOT, root)];
        nodes.extend(status.nodes);
        nodes.extend(apps.nodes);
        Some(TreeUpdate {
            nodes,
            tree: Some(TreeInfo::new(ROOT)),
            tree_id: apps.tree_id,
            focus: apps.focus,
        })
    }
}
