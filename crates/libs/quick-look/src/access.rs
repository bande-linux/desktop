use std::time::Instant;

use accesskit::{
    Action as AccessAction, ActionRequest, Live, Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate,
};
use bande_toolkit::Rect;

use crate::{Hit, L10N, QuickLook};

const ROOT: NodeId = NodeId(1);
const FIRST: u64 = 2;

fn bounds(rect: Rect) -> accesskit::Rect {
    accesskit::Rect {
        x0: f64::from(rect.x),
        y0: f64::from(rect.y),
        x1: f64::from(rect.x + rect.width),
        y1: f64::from(rect.y + rect.height),
    }
}

impl QuickLook {
    fn label(&self, hit: Hit) -> Option<String> {
        let id = match hit {
            Hit::Close => "close",
            Hit::FullScreen if self.full_screen => "exit-full-screen",
            Hit::FullScreen => "full-screen",
            Hit::Previous => "previous",
            Hit::Next => "next",
            Hit::IndexSheet => "index-sheet",
            Hit::Rotate => "rotate",
            Hit::Copy => "copy",
            Hit::Unlock => "unlock",
            Hit::Open => {
                let app = self.subject()?.app.as_ref()?.name.clone();
                return Some(L10N.tr_args("open-with", [("app", app.into())]));
            }
            Hit::Tile(index) => return Some(self.subjects.get(index)?.entry.name.clone()),
            Hit::Thumbnail(page) => {
                return Some(L10N.tr_args("page", [("number", (page + 1).into())]));
            }
            Hit::Resize(_) | Hit::Header | Hit::Panel => return None,
        };
        Some(L10N.tr(id))
    }

    fn controls(&self) -> Vec<(Rect, Hit, String)> {
        self.hits
            .iter()
            .filter_map(|(rect, hit)| Some((*rect, *hit, self.label(*hit)?)))
            .collect()
    }

    pub fn accessibility(&self) -> TreeUpdate {
        let mut root = Node::new(Role::Dialog);
        root.set_label(L10N.tr("quick-look"));
        root.set_live(Live::Polite);
        if let Some(subject) = self.subject() {
            let mut description = format!("{}, {}", subject.entry.name, subject.kind);
            if let Some((at, count)) = self.position() {
                description.push_str(", ");
                description.push_str(
                    &L10N.tr_args("position", [("at", at.into()), ("count", count.into())]),
                );
            }
            root.set_description(description);
        }
        root.set_bounds(bounds(self.frame));
        let mut nodes = Vec::new();
        for (index, (rect, hit, label)) in self.controls().into_iter().enumerate() {
            let role = match hit {
                Hit::Tile(_) | Hit::Thumbnail(_) => Role::ListBoxOption,
                _ => Role::Button,
            };
            let mut node = Node::new(role);
            node.set_label(label);
            node.set_bounds(bounds(rect));
            node.add_action(AccessAction::Click);
            nodes.push((NodeId(FIRST + index as u64), node));
        }
        root.set_children(nodes.iter().map(|(id, _)| *id).collect::<Vec<_>>());
        nodes.insert(0, (ROOT, root));
        TreeUpdate {
            nodes,
            tree: Some(TreeInfo {
                toolkit_name: Some("bande-toolkit".to_owned()),
                ..TreeInfo::new(ROOT)
            }),
            tree_id: TreeId::ROOT,
            focus: ROOT,
        }
    }

    pub fn perform(&mut self, request: &ActionRequest, now: Instant) -> bool {
        if request.action != AccessAction::Click {
            return false;
        }
        let Some(index) = request.target_node.0.checked_sub(FIRST) else {
            return false;
        };
        let Some((_, hit, _)) = self.controls().get(index as usize).cloned() else {
            return false;
        };
        self.activate(hit, now);
        true
    }
}
