use std::os::fd::BorrowedFd;
use std::time::Instant;

use crate::L10N;
use crate::menu_bar::StatusView;
use crate::rail::{TaskbarView, bar_width, edge_band, owns_node, surface_size};
use crate::taskbar_client::anchor;
use accesskit::{Node, NodeId, Role as AccessRole, TreeInfo, TreeUpdate};
use bande_config::BarEdge;
use bande_toolkit::ui::Event;
use bande_toolkit::{
    App, Item, KeyboardFocus, LayerKind, LayerOptions, Point, Rect, Role, Scene, Shape, Window,
    WindowEvent,
};

use crate::menu_bar_client::MenuBarClient;
use crate::taskbar_client::Taskbar;

const BAR_NAMESPACE: &str = "bar";
const ROOT: NodeId = NodeId(1 << 41);
const HIDDEN_SHADOW: f32 = 8.0;
const UNREAD_SLIVER: (f32, f32) = (2.0, 24.0);

pub fn combined_role(edge: BarEdge, icon_size: f32) -> (Role, (u32, u32)) {
    (
        Role::Layer(LayerOptions {
            layer: LayerKind::Top,
            anchor: anchor(edge),
            exclusive_zone: bar_width(icon_size).round() as i32,
            keyboard: KeyboardFocus::OnDemand,
            namespace: BAR_NAMESPACE.to_owned(),
        }),
        surface_size(edge, icon_size),
    )
}

pub struct CombinedClient {
    taskbar: Taskbar,
    status: MenuBarClient<TaskbarView>,
    status_open: bool,
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
    pub fn new(taskbar: Taskbar, status: MenuBarClient<TaskbarView>) -> Self {
        Self {
            taskbar,
            status,
            status_open: false,
        }
    }

    fn route(&mut self, window: &mut Window, event: WindowEvent) {
        match &event {
            _ if pointer(&event) => {
                self.taskbar.event(window, event.clone());
                self.status.event(window, event);
            }
            WindowEvent::Input(_) | WindowEvent::Drag(_) | WindowEvent::FocusShell(_) => {
                self.taskbar.event(window, event);
            }
            WindowEvent::MenuAction(_) | WindowEvent::MenuClosed if self.taskbar.has_menu() => {
                self.taskbar.event(window, event);
            }
            WindowEvent::MenuAction(_) | WindowEvent::MenuClosed => {
                self.status.event(window, event);
            }
            WindowEvent::Accessibility(request) if owns_node(request.target_node) => {
                self.status.event(window, event);
            }
            WindowEvent::Accessibility(_) => self.taskbar.event(window, event),
            _ => {
                self.taskbar.event(window, event.clone());
                self.status.event(window, event);
            }
        }
    }
}

impl App for CombinedClient {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let view = self.status.view_mut();
        view.set_icon_size(self.taskbar.icon_size());
        self.taskbar
            .set_rail(view.apps_region((width as f32, height as f32)));
        let mut scene = self.status.scene(window);
        let apps = self.taskbar.scene(window);
        scene.width = width;
        scene.items.extend(apps.items);
        let revealed = self.taskbar.revealed(Instant::now());
        if revealed < 1.0 {
            let view = self.status.view_mut();
            let edge = view.edge();
            let hidden = ((view.width() + HIDDEN_SHADOW) * (1.0 - revealed)).round();
            let offset = match edge {
                BarEdge::Left => Point::new(-hidden, 0.0),
                BarEdge::Right => Point::new(hidden, 0.0),
                BarEdge::Top => Point::new(0.0, -hidden),
                BarEdge::Bottom => Point::new(0.0, hidden),
            };
            let mut items = vec![Item::Translate {
                offset,
                items: std::mem::take(&mut scene.items),
            }];
            if view.unread() && revealed <= 0.0 {
                let (width, height) = (width as f32, height as f32);
                let (thin, long) = UNREAD_SLIVER;
                let strip = edge_band(edge, (width, height), thin);
                let sliver = match edge {
                    BarEdge::Left | BarEdge::Right => Rect::new(strip.x, height - long, thin, long),
                    BarEdge::Top | BarEdge::Bottom => Rect::new(width - long, strip.y, long, thin),
                };
                items.push(Item::Fill {
                    shape: Shape::rect(sliver),
                    paint: window.appearance().palette().accent.into(),
                });
            }
            scene.items = items;
        }
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        self.route(window, event);
        let open = self.status.view_mut().open().is_some();
        if open != self.status_open {
            self.status_open = open;
            self.taskbar.set_menu_open(open);
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        let mut sources = self.taskbar.sources();
        sources.extend(self.status.sources());
        sources
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        let docked = self.taskbar.sources().len();
        if index < docked {
            self.taskbar.source_ready(window, index);
        } else {
            self.status.source_ready(window, index - docked);
        }
    }

    fn deadline(&self) -> Option<Instant> {
        [self.taskbar.deadline(), self.status.deadline()]
            .into_iter()
            .flatten()
            .min()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        if self
            .taskbar
            .deadline()
            .is_some_and(|deadline| deadline <= Instant::now())
        {
            self.taskbar.tick(window, now);
        }
        if self
            .status
            .deadline()
            .is_some_and(|deadline| deadline <= Instant::now())
        {
            self.status.tick(window, now);
        }
    }

    fn animating(&self) -> bool {
        self.taskbar.animating() || self.status.animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        let apps = self.taskbar.accessibility()?;
        let status = self.status.accessibility()?;
        let mut root = Node::new(AccessRole::Toolbar);
        root.set_label(L10N.tr("taskbar"));
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
