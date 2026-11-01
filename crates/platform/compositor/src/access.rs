use accesskit::{
    ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, Node, NodeId, Role,
    TreeId, TreeInfo, TreeUpdate,
};
use accesskit_unix::Adapter;

const WINDOW: NodeId = NodeId(1);
const LIST: NodeId = NodeId(2);
const FIRST_ITEM: u64 = 3;

struct Passive;

impl ActivationHandler for Passive {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        None
    }
}

impl ActionHandler for Passive {
    fn do_action(&mut self, _request: ActionRequest) {}
}

impl DeactivationHandler for Passive {
    fn deactivate_accessibility(&mut self) {}
}

#[derive(Default)]
pub(crate) struct Announcer {
    adapter: Option<Adapter>,
    shown: bool,
}

fn overlay(title: &str, items: &[String], selected: usize) -> TreeUpdate {
    let mut window = Node::new(Role::Window);
    window.set_label(title);
    window.set_children(vec![LIST]);
    let mut list = Node::new(Role::ListBox);
    list.set_label(title);
    let ids: Vec<NodeId> = (0..items.len() as u64)
        .map(|index| NodeId(FIRST_ITEM + index))
        .collect();
    list.set_children(ids.clone());
    let mut nodes = vec![(WINDOW, window), (LIST, list)];
    for (index, (item, id)) in items.iter().zip(&ids).enumerate() {
        let mut node = Node::new(Role::ListBoxOption);
        node.set_label(item.as_str());
        node.set_selected(index == selected);
        nodes.push((*id, node));
    }
    TreeUpdate {
        nodes,
        tree: Some(TreeInfo {
            toolkit_name: Some("bande-compositor".to_owned()),
            toolkit_version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            ..TreeInfo::new(WINDOW)
        }),
        tree_id: TreeId::ROOT,
        focus: ids.get(selected).copied().unwrap_or(LIST),
    }
}

impl Announcer {
    pub(crate) fn announce(&mut self, title: &str, items: &[String], selected: usize) {
        let adapter = self
            .adapter
            .get_or_insert_with(|| Adapter::new(Passive, Passive, Passive));
        adapter.update_if_active(|| overlay(title, items, selected));
        if !self.shown {
            adapter.update_window_focus_state(true);
            self.shown = true;
        }
    }

    pub(crate) fn withdraw(&mut self) {
        if let Some(adapter) = self.adapter.as_mut().filter(|_| self.shown) {
            adapter.update_window_focus_state(false);
        }
        self.shown = false;
    }
}
