use std::os::fd::BorrowedFd;
use std::time::Instant;

use accesskit::{Node, NodeId, Role as AccessRole, TreeInfo, TreeUpdate};
use unbound_bar::L10N;
use unbound_bar::rail::{RailView, bar_width, owns_node, surface_size};
use unbound_toolkit::ui::Event;
use unbound_toolkit::{
    App, Edges, KeyboardFocus, LayerKind, LayerOptions, Role, Scene, Window, WindowEvent,
};

use crate::dock_client::Dock;
use crate::menu_bar_client::MenuBarClient;

const BAR_NAMESPACE: &str = "bar";
const MENU_BAR_NAMESPACE: &str = "menubar";
const ROOT: NodeId = NodeId(1 << 41);

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
        Self { dock, status }
    }
}

impl App for CombinedClient {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let view = self.status.view_mut();
        view.set_icon_size(self.dock.icon_size());
        self.dock.set_rail(view.apps_region(height as f32));
        let mut scene = self.status.scene(window);
        let apps = self.dock.scene(window);
        scene.width = width;
        scene.items.extend(apps.items);
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
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
