use accesskit::{
    Action, ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, Node, NodeId,
    Role, TreeId, TreeInfo, TreeUpdate,
};
use accesskit_unix::Adapter;
use smithay::reexports::calloop::channel::Sender;

use crate::state::State;

const WINDOW: NodeId = NodeId(1);
const FIRST_GROUP: u64 = 2;
const GROUP_STRIDE: u64 = 1 << 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Overlay {
    ForceQuit,
    WindowMenu,
    Overview,
    Assist,
    Frame,
}

pub(crate) type AccessRequests = Sender<(Overlay, ActionRequest)>;

struct Inactive;

impl ActivationHandler for Inactive {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        None
    }
}

impl DeactivationHandler for Inactive {
    fn deactivate_accessibility(&mut self) {}
}

pub(crate) struct Requests(Option<(Overlay, AccessRequests)>);

impl ActionHandler for Requests {
    fn do_action(&mut self, request: ActionRequest) {
        if let Some((overlay, requests)) = &self.0
            && requests.send((*overlay, request)).is_err()
        {
            tracing::debug!("the compositor no longer takes accessibility actions");
        }
    }
}

pub(crate) fn adapter(overlay: Overlay, requests: Option<&AccessRequests>) -> Adapter {
    Adapter::new(
        Inactive,
        Requests(requests.map(|requests| (overlay, requests.clone()))),
        Inactive,
    )
}

pub(crate) struct Entry {
    pub(crate) label: String,
    pub(crate) selected: bool,
    pub(crate) disabled: bool,
}

impl Entry {
    pub(crate) fn new(label: String) -> Self {
        Self {
            label,
            selected: false,
            disabled: false,
        }
    }

    pub(crate) fn selected(self, selected: bool) -> Self {
        Self { selected, ..self }
    }

    pub(crate) fn disabled(self, disabled: bool) -> Self {
        Self { disabled, ..self }
    }
}

pub(crate) struct Group {
    pub(crate) role: Role,
    pub(crate) item_role: Role,
    pub(crate) label: String,
    pub(crate) clickable: bool,
    pub(crate) entries: Vec<Entry>,
}

fn group_node(group: usize) -> NodeId {
    NodeId(FIRST_GROUP + group as u64)
}

fn entry_node(group: usize, entry: usize) -> NodeId {
    NodeId(GROUP_STRIDE * (group as u64 + 1) + entry as u64)
}

pub(crate) fn entry_of(node: NodeId) -> Option<(usize, usize)> {
    let group = usize::try_from(node.0 / GROUP_STRIDE)
        .ok()?
        .checked_sub(1)?;
    let entry = usize::try_from(node.0 % GROUP_STRIDE).ok()?;
    Some((group, entry))
}

fn tree_info() -> TreeInfo {
    TreeInfo {
        toolkit_name: Some("bande-compositor".to_owned()),
        toolkit_version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        ..TreeInfo::new(WINDOW)
    }
}

pub(crate) fn grouped(title: &str, groups: &[Group], focus: Option<(usize, usize)>) -> TreeUpdate {
    let mut window = Node::new(Role::Window);
    window.set_label(title);
    window.set_children((0..groups.len()).map(group_node).collect::<Vec<_>>());
    let mut nodes = vec![(WINDOW, window)];
    for (index, group) in groups.iter().enumerate() {
        let mut container = Node::new(group.role);
        container.set_label(group.label.as_str());
        container.set_children(
            (0..group.entries.len())
                .map(|entry| entry_node(index, entry))
                .collect::<Vec<_>>(),
        );
        nodes.push((group_node(index), container));
        for (position, entry) in group.entries.iter().enumerate() {
            let mut node = Node::new(group.item_role);
            node.set_label(entry.label.as_str());
            if matches!(group.item_role, Role::ListBoxOption | Role::Tab) {
                node.set_selected(entry.selected);
            }
            if entry.disabled {
                node.set_disabled();
            } else if group.clickable {
                node.add_action(Action::Click);
            }
            nodes.push((entry_node(index, position), node));
        }
    }
    let focus = focus
        .filter(|(group, entry)| {
            groups
                .get(*group)
                .is_some_and(|known| *entry < known.entries.len())
        })
        .map(|(group, entry)| entry_node(group, entry))
        .unwrap_or(WINDOW);
    TreeUpdate {
        nodes,
        tree: Some(tree_info()),
        tree_id: TreeId::ROOT,
        focus,
    }
}

pub(crate) fn window_with(
    title: &str,
    roots: Vec<NodeId>,
    nodes: Vec<(NodeId, Node)>,
    focus: Option<NodeId>,
) -> TreeUpdate {
    let mut window = Node::new(Role::Window);
    window.set_label(title);
    window.set_children(roots);
    let mut all = vec![(WINDOW, window)];
    all.extend(nodes);
    TreeUpdate {
        nodes: all,
        tree: Some(tree_info()),
        tree_id: TreeId::ROOT,
        focus: focus.unwrap_or(WINDOW),
    }
}

#[derive(Default)]
pub(crate) struct Announcer {
    adapter: Option<Adapter>,
    requests: Option<(Overlay, AccessRequests)>,
    shown: bool,
}

impl Announcer {
    pub(crate) fn connect(&mut self, overlay: Overlay, requests: AccessRequests) {
        self.requests = Some((overlay, requests));
    }

    pub(crate) fn publish(&mut self, takes_focus: bool, tree: impl FnOnce() -> TreeUpdate) {
        let requests = self.requests.clone();
        let adapter = self
            .adapter
            .get_or_insert_with(|| Adapter::new(Inactive, Requests(requests), Inactive));
        adapter.update_if_active(tree);
        if takes_focus && !self.shown {
            adapter.update_window_focus_state(true);
            self.shown = true;
        }
    }

    pub(crate) fn announce(&mut self, title: &str, items: &[String], selected: usize) {
        let group = Group {
            role: Role::ListBox,
            item_role: Role::ListBoxOption,
            label: title.to_owned(),
            clickable: false,
            entries: items
                .iter()
                .enumerate()
                .map(|(index, item)| Entry::new(item.clone()).selected(index == selected))
                .collect(),
        };
        self.publish(true, || grouped(title, &[group], Some((0, selected))));
    }

    pub(crate) fn withdraw(&mut self) {
        if let Some(adapter) = self.adapter.as_mut().filter(|_| self.shown) {
            adapter.update_window_focus_state(false);
        }
        self.shown = false;
    }
}

impl State {
    pub(crate) fn connect_access(&mut self, requests: &AccessRequests) {
        self.force_quit.take_actions(requests.clone());
        self.window_menus
            .announcer
            .connect(Overlay::WindowMenu, requests.clone());
        self.overview
            .announcer
            .connect(Overlay::Overview, requests.clone());
        self.assist
            .announcer
            .connect(Overlay::Assist, requests.clone());
        self.decorations
            .announcer
            .connect(Overlay::Frame, requests.clone());
    }

    pub(crate) fn access_action(&mut self, overlay: Overlay, request: ActionRequest) {
        match overlay {
            Overlay::ForceQuit => self.force_quit_action(request),
            Overlay::WindowMenu => self.window_menu_action_request(&request),
            Overlay::Overview => self.overview_action(&request),
            Overlay::Assist => self.assist_action(&request),
            Overlay::Frame => self.frame_action(&request),
        }
    }
}
