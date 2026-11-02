use std::collections::BTreeMap;

use accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeUpdate};
use bande_toolkit::Rect;
use bande_toolkit::ui::{access_rect, group_heading};

use crate::L10N;
use crate::tabs::location_title;
use crate::view::{FilesView, Hit};
use crate::views::ViewMode;

const KIND_SHIFT: u32 = 30;
const COLUMN_SHIFT: u32 = 22;
const ROW_LIMIT: usize = 1 << COLUMN_SHIFT;
const COLUMN_LIMIT: usize = (1 << (KIND_SHIFT - COLUMN_SHIFT)) - 1;
const PART_MASK: u64 = (1 << 32) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Part {
    List(usize),
    Heading(usize),
    Toggle(usize),
    Item(usize, usize),
}

impl Part {
    fn index(self) -> Option<u64> {
        let (kind, column, row) = match self {
            Self::List(column) => (0, column, 0),
            Self::Heading(section) => (1, 0, section),
            Self::Toggle(section) => (2, 0, section),
            Self::Item(column, row) => (3, column, row),
        };
        (column < COLUMN_LIMIT && row < ROW_LIMIT)
            .then_some((kind << KIND_SHIFT) | ((column as u64) << COLUMN_SHIFT) | row as u64)
    }

    fn from_index(index: u64) -> Option<Self> {
        let column = ((index >> COLUMN_SHIFT) as usize) & COLUMN_LIMIT;
        let row = (index as usize) & (ROW_LIMIT - 1);
        match index >> KIND_SHIFT {
            0 => Some(Self::List(column)),
            1 => Some(Self::Heading(row)),
            2 => Some(Self::Toggle(row)),
            3 => Some(Self::Item(column, row)),
            _ => None,
        }
    }
}

impl FilesView {
    pub(crate) fn learn_kinds(&mut self) {
        if self.mode == ViewMode::List {
            return;
        }
        let unknown = self
            .hits
            .iter()
            .filter_map(|(_, hit)| match hit {
                Hit::Row { column, row } => self.browser.columns().get(*column)?.entries.get(*row),
                _ => None,
            })
            .filter(|entry| self.known_kind(entry).is_none())
            .cloned()
            .collect::<Vec<_>>();
        if unknown.is_empty() {
            return;
        }
        let mut formats = std::mem::take(&mut self.table_formats);
        for entry in &unknown {
            self.entry_kind(entry, &mut formats);
        }
        self.table_formats = formats;
    }

    fn node_of(&self, part: Part) -> Option<NodeId> {
        part.index().map(|index| self.access.part(index))
    }

    fn part_of(&self, node: NodeId) -> Option<Part> {
        let owner = NodeId::from(self.access).0;
        (node.0 >> 32 == owner)
            .then(|| (node.0 & PART_MASK).checked_sub(1))
            .flatten()
            .and_then(Part::from_index)
    }

    pub(crate) fn describe_items(&self, tree: &mut TreeUpdate) {
        let table = NodeId::from(self.table);
        tree.nodes
            .retain(|(id, _)| *id != table && id.0 >> 32 != table.0);
        let mut parts = BTreeMap::<Part, Option<Rect>>::new();
        for (rect, hit) in &self.hits {
            let part = match hit {
                Hit::Column(column) => Part::List(*column),
                Hit::Row { column, row } => Part::Item(*column, *row),
                Hit::GroupToggle(section) => Part::Toggle(*section),
                _ => continue,
            };
            let bounds = parts.entry(part).or_default();
            *bounds = Some(match (*bounds, part, self.mode) {
                (Some(known), Part::Item(..), ViewMode::Icons) => union(known, *rect),
                _ => *rect,
            });
        }
        parts.extend(
            self.headings
                .iter()
                .map(|(rect, section)| (Part::Heading(*section), Some(*rect))),
        );
        let active = self.browser.active();
        if let Some(row) = self.browser.active_column().selected {
            parts.entry(Part::Item(active, row)).or_default();
        }
        let groups = self.group_cache.cached();
        let start = |part: &Part| match part {
            Part::Heading(section) | Part::Toggle(section) => {
                groups.get(*section).map_or(0, |(start, _)| *start)
            }
            Part::Item(_, row) => *row,
            Part::List(_) => 0,
        };
        let mut lists = BTreeMap::<usize, Vec<(Part, Node)>>::new();
        let mut shown = Vec::new();
        for (part, bounds) in &parts {
            let column = match part {
                Part::List(column) => {
                    shown.push((*column, *bounds));
                    continue;
                }
                Part::Item(column, _) => *column,
                Part::Heading(_) | Part::Toggle(_) => 0,
            };
            if let Some(node) = self.part_node(*part, *bounds) {
                lists.entry(column).or_default().push((*part, node));
            }
        }
        let mut lists_in_window = Vec::new();
        for (column, bounds) in shown {
            let (Some(list), Some(shown)) = (
                self.node_of(Part::List(column)),
                self.browser.columns().get(column),
            ) else {
                continue;
            };
            let mut node = Node::new(Role::ListBox);
            node.set_label(location_title(&shown.location));
            node.set_multiselectable();
            node.set_size_of_set(shown.entries.len());
            if let Some(bounds) = bounds {
                node.set_bounds(access_rect(bounds));
            }
            let mut members = lists.remove(&column).unwrap_or_default();
            members.sort_by_key(|(part, _)| (start(part), *part));
            let mut children = Vec::with_capacity(members.len());
            for (part, member) in members {
                if let Some(id) = self.node_of(part) {
                    children.push(id);
                    tree.nodes.push((id, member));
                }
            }
            node.set_children(children);
            tree.nodes.push((list, node));
            lists_in_window.push(list);
        }
        if let Some((_, root)) = tree.nodes.first_mut() {
            let children = root
                .children()
                .iter()
                .copied()
                .filter(|child| *child != table)
                .chain(lists_in_window)
                .collect::<Vec<_>>();
            root.set_children(children);
        }
        if self.ui.focus().is_none() && !self.ui.has_sheet() {
            let cursor = self
                .browser
                .active_column()
                .selected
                .map_or(Part::List(active), |row| Part::Item(active, row));
            if let Some(focus) = [cursor, Part::List(active)]
                .into_iter()
                .filter_map(|part| self.node_of(part))
                .find(|id| tree.nodes.iter().any(|(node, _)| node == id))
            {
                tree.focus = focus;
            }
        }
    }

    fn part_node(&self, part: Part, bounds: Option<Rect>) -> Option<Node> {
        let mut node = match part {
            Part::List(_) => return None,
            Part::Heading(section) => {
                let (_, title) = self.group_cache.cached().get(section)?;
                return Some(group_heading(title, bounds?));
            }
            Part::Toggle(section) => {
                let (_, title) = self.group_cache.cached().get(section)?;
                let mut node = Node::new(Role::Button);
                node.set_label(L10N.tr(match self.grid_scroll.shows_all(title) {
                    true => "group-show-less",
                    false => "group-show-all",
                }));
                node.add_action(Action::Click);
                node
            }
            Part::Item(column, row) => {
                let shown = self.browser.columns().get(column)?;
                let entry = shown.entries.get(row)?;
                let mut node = Node::new(Role::ListBoxOption);
                node.set_label(self.shown_name(entry));
                if let Some(kind) = self.known_kind(entry) {
                    node.set_description(&*kind);
                }
                node.set_selected(shown.is_marked(row));
                node.set_position_in_set(row);
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
                node
            }
        };
        if let Some(bounds) = bounds {
            node.set_bounds(access_rect(bounds));
        }
        Some(node)
    }

    pub(crate) fn perform_on_items(&mut self, request: &ActionRequest) -> Option<bool> {
        let part = self.part_of(request.target_node)?;
        Some(match (part, request.action) {
            (Part::Toggle(section), Action::Click) => {
                self.toggle_group(section);
                true
            }
            (Part::Item(column, row), Action::Click | Action::Focus) => {
                let shown = self.browser.columns().get(column)?;
                shown.entries.get(row)?;
                let chosen = column == self.browser.active() && shown.marked == [row];
                self.focus_content();
                if chosen && request.action == Action::Click {
                    self.open();
                } else if !chosen {
                    self.commit_rename();
                    self.browser.select(column, row);
                    self.selection_changed();
                }
                true
            }
            _ => false,
        })
    }
}

fn union(a: Rect, b: Rect) -> Rect {
    let (left, top) = (a.x.min(b.x), a.y.min(b.y));
    let right = (a.x + a.width).max(b.x + b.width);
    let bottom = (a.y + a.height).max(b.y + b.height);
    Rect::new(left, top, right - left, bottom - top)
}
