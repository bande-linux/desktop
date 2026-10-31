#![allow(dead_code)]

use std::time::Duration;

use accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId};
use unbound_settings::SettingsView;

pub const SIZE: (u32, u32) = (900, 720);
pub const WAIT: Duration = Duration::from_secs(5);

pub fn nodes(view: &mut SettingsView) -> Vec<(NodeId, Node)> {
    view.scene(SIZE.0, SIZE.1);
    view.accessibility().nodes
}

pub fn find(view: &mut SettingsView, role: Role, label: &str) -> Option<NodeId> {
    nodes(view)
        .into_iter()
        .find(|(_, node)| {
            node.role() == role && node.label().is_some_and(|text| text.starts_with(label))
        })
        .map(|(id, _)| id)
}

pub fn has_text(view: &mut SettingsView, text: &str) -> bool {
    nodes(view).into_iter().any(|(_, node)| {
        node.label().is_some_and(|label| label.contains(text))
            || node.value().is_some_and(|value| value.contains(text))
    })
}

pub fn click(view: &mut SettingsView, target: NodeId) {
    view.perform(&ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: target,
        data: None,
    });
}
