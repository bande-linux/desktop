use std::collections::HashMap;

use accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeUpdate};
use bande_l10n::{tr, trn};
use bande_toolkit::ui::Pointer;
use bande_toolkit::ui::access_rect;
use bande_toolkit::{Point, Rect};

use crate::store::Summary;
use crate::ui::format::{full_date, list_date};
use crate::ui::reader::{Respond, Target};
use crate::ui::sidebar::Row;
use crate::ui::view::{Hit, MailView, Pane};
use jiff::Timestamp;

const KIND_SHIFT: u32 = 28;
const INDEX_LIMIT: usize = 1 << KIND_SHIFT;
const PART_MASK: u64 = (1 << 32) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Sidebar,
    Place(usize),
    List,
    Message(usize),
    Conversation(usize),
    Reader,
    Target(usize),
    Body(usize),
    Text(usize),
    Offline(usize),
}

impl Part {
    fn index(self) -> Option<u64> {
        let (kind, index) = match self {
            Self::Sidebar => (0, 0),
            Self::Place(index) => (1, index),
            Self::List => (2, 0),
            Self::Message(index) => (3, index),
            Self::Conversation(index) => (4, index),
            Self::Reader => (5, 0),
            Self::Target(index) => (6, index),
            Self::Body(index) => (7, index),
            Self::Text(index) => (8, index),
            Self::Offline(index) => (9, index),
        };
        (index < INDEX_LIMIT).then_some((kind << KIND_SHIFT) | index as u64)
    }

    fn from_index(index: u64) -> Option<Self> {
        let row = (index as usize) & (INDEX_LIMIT - 1);
        Some(match index >> KIND_SHIFT {
            0 => Self::Sidebar,
            1 => Self::Place(row),
            2 => Self::List,
            3 => Self::Message(row),
            4 => Self::Conversation(row),
            5 => Self::Reader,
            6 => Self::Target(row),
            7 => Self::Body(row),
            8 => Self::Text(row),
            9 => Self::Offline(row),
            _ => return None,
        })
    }
}

fn centre(rect: Rect) -> Point {
    Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
}

fn clickable(role: Role, label: &str) -> Node {
    let mut node = Node::new(role);
    node.set_label(label);
    node.add_action(Action::Click);
    node.add_action(Action::Focus);
    node
}

impl MailView {
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

    fn bounds_of(&self, wanted: impl Fn(&Hit) -> bool) -> Option<Rect> {
        self.hits
            .iter()
            .find(|(_, hit)| wanted(hit))
            .map(|(rect, _)| *rect)
    }

    fn push(&self, tree: &mut TreeUpdate, part: Part, mut node: Node, bounds: Option<Rect>) {
        let Some(id) = self.node_of(part) else {
            return;
        };
        if let Some(bounds) = bounds {
            node.set_bounds(access_rect(bounds));
        }
        tree.nodes.push((id, node));
    }

    pub(crate) fn describe_panes(&self, tree: &mut TreeUpdate) {
        let panes = [
            self.describe_sidebar(tree),
            self.describe_list(tree),
            self.describe_reader(tree),
        ];
        if let Some((_, root)) = tree.nodes.first_mut() {
            for pane in panes.into_iter().flatten() {
                root.push_child(pane);
            }
        }
        if self.ui.focus().is_none() && !self.ui.has_sheet() {
            let focused = match self.focus {
                Pane::Sidebar => self
                    .sidebar
                    .iter()
                    .position(|row| matches!(row, Row::Place { place, .. } if *place == self.place))
                    .map_or(Part::Sidebar, Part::Place),
                Pane::List => self
                    .selected
                    .and_then(|id| self.messages.iter().position(|summary| summary.id == id))
                    .map_or(Part::List, Part::Message),
                Pane::Reader => match &self.reader_target {
                    Some((_, target)) => self
                        .hits
                        .iter()
                        .position(|(_, hit)| *hit == Hit::Reader(target.clone()))
                        .map_or(Part::Reader, Part::Target),
                    None => self
                        .thread
                        .as_ref()
                        .and_then(|thread| {
                            thread
                                .entries
                                .iter()
                                .position(|entry| entry.summary.id == thread.focus)
                        })
                        .map_or(Part::Reader, Part::Body),
                },
            };
            if let Some(focus) = [focused, Part::List]
                .into_iter()
                .filter_map(|part| self.node_of(part))
                .find(|id| tree.nodes.iter().any(|(node, _)| node == id))
            {
                tree.focus = focus;
            }
        }
    }

    fn describe_sidebar(&self, tree: &mut TreeUpdate) -> Option<NodeId> {
        let mut children = Vec::new();
        for (index, row) in self.sidebar.iter().enumerate() {
            let (node, bounds) = match row {
                Row::Heading(title) => {
                    let mut node = Node::new(Role::Heading);
                    node.set_label(title.as_str());
                    node.set_level(2);
                    let offline = self
                        .accounts
                        .iter()
                        .find(|(account, _)| account.email == *title)
                        .map(|(account, _)| account.id)
                        .filter(|account| self.offline(*account));
                    if let Some(account) = offline {
                        let mut button = clickable(Role::Button, &tr!("Offline"));
                        button.set_description(self.failure(account));
                        let bounds = self.bounds_of(|hit| *hit == Hit::Offline(account));
                        self.push(tree, Part::Offline(index), button, bounds);
                        node.set_children(
                            self.node_of(Part::Offline(index))
                                .into_iter()
                                .collect::<Vec<_>>(),
                        );
                    }
                    (node, None)
                }
                Row::Place {
                    place,
                    title,
                    unread,
                    ..
                } => {
                    let mut node = clickable(Role::ListBoxOption, title);
                    if *unread > 0 {
                        node.set_description(trn!("1 unread", "{count} unread", count = *unread));
                    }
                    node.set_selected(*place == self.place);
                    let bounds = self.bounds_of(|hit| *hit == Hit::Place(index));
                    (node, bounds)
                }
            };
            self.push(tree, Part::Place(index), node, bounds);
            children.extend(self.node_of(Part::Place(index)));
        }
        let mut node = Node::new(Role::ListBox);
        node.set_label(tr!("Mailboxes"));
        node.set_bounds(access_rect(self.sidebar_frame()));
        node.add_action(Action::Focus);
        node.set_children(children);
        self.push(tree, Part::Sidebar, node, None);
        self.node_of(Part::Sidebar)
    }

    fn message_node(&self, index: usize, summary: &Summary, now: Timestamp) -> Node {
        let subject = match summary.subject.as_str() {
            "" => tr!("No Subject"),
            subject => subject.to_owned(),
        };
        let sender = match summary.sender.shown() {
            "" => tr!("No Sender"),
            shown => shown.to_owned(),
        };
        let mut node = clickable(Role::ListBoxOption, &subject);
        let date = list_date(summary.date, now, &self.zone, &self.formats);
        let mut details = vec![tr!("From {sender}, {date}", sender = sender, date = date)];
        if !summary.flags.seen {
            details.push(tr!("Unread"));
        }
        if summary.flags.flagged {
            details.push(tr!("Flagged"));
        }
        if summary.attachments {
            details.push(tr!("Has Attachments"));
        }
        if let Some(count) = self.conversations.count(summary.id) {
            details.push(tr!("{count} messages", count = count));
        }
        details.push(summary.preview.clone());
        node.set_description(details.join(", "));
        node.set_selected(self.is_chosen(summary.id));
        node.set_position_in_set(index + 1);
        node.set_size_of_set(self.messages.len());
        node
    }

    fn describe_list(&self, tree: &mut TreeUpdate) -> Option<NodeId> {
        let now = self.now.unwrap_or_else(Timestamp::now);
        let mut children = Vec::new();
        let shown = self
            .hits
            .iter()
            .filter_map(|(rect, hit)| match hit {
                Hit::Message(id) => Some((*id, *rect)),
                _ => None,
            })
            .collect::<HashMap<_, _>>();
        for (index, summary) in self.messages.iter().enumerate() {
            let bounds = shown.get(&summary.id).copied();
            if bounds.is_none() && !self.is_chosen(summary.id) {
                continue;
            }
            self.push(
                tree,
                Part::Message(index),
                self.message_node(index, summary, now),
                bounds,
            );
            children.extend(self.node_of(Part::Message(index)));
            if let Some(capsule) = self.bounds_of(|hit| *hit == Hit::Conversation(summary.id)) {
                let label = match self.conversations.collapsed(summary.id) {
                    Some(_) => tr!("Expand Conversation"),
                    None => tr!("Collapse Conversation"),
                };
                self.push(
                    tree,
                    Part::Conversation(index),
                    clickable(Role::Button, &label),
                    Some(capsule),
                );
                children.extend(self.node_of(Part::Conversation(index)));
            }
        }
        let mut node = Node::new(Role::ListBox);
        node.set_label(self.title());
        node.set_multiselectable();
        node.set_size_of_set(self.messages.len());
        node.set_bounds(access_rect(self.list_frame()));
        node.add_action(Action::Focus);
        node.set_children(children);
        self.push(tree, Part::List, node, None);
        self.node_of(Part::List)
    }

    fn target_label(&self, target: &Target) -> Option<(Role, String)> {
        let thread = self.thread.as_ref()?;
        Some(match target {
            Target::Card(_) => return None,
            Target::Expand(id) => {
                let entry = thread.entry(*id)?;
                let sender = match entry.summary.sender.shown() {
                    "" => tr!("No Sender"),
                    shown => shown.to_owned(),
                };
                (
                    Role::Button,
                    tr!("Show Message from {sender}", sender = sender),
                )
            }
            Target::Quotes(id) => (
                Role::Button,
                match thread.entry(*id)?.quotes {
                    true => tr!("Hide Quoted Text"),
                    false => tr!("Show Quoted Text"),
                },
            ),
            Target::Respond(_, Respond::Reply) => (Role::Button, tr!("Reply")),
            Target::Respond(_, Respond::ReplyAll) => (Role::Button, tr!("Reply All")),
            Target::Respond(_, Respond::Forward) => (Role::Button, tr!("Forward")),
            Target::Link(_, _, shown) => (Role::Link, shown.clone()),
            Target::Address(address) => (Role::Button, address.full()),
            Target::Attachment(id, index) => {
                let attachment = thread
                    .entry(*id)?
                    .reading
                    .as_ref()?
                    .content
                    .attachments
                    .get(*index)?;
                (Role::Button, attachment.name.clone())
            }
            Target::LoadRemote(_) => (Role::Button, tr!("Load Remote Content")),
        })
    }

    fn describe_reader(&self, tree: &mut TreeUpdate) -> Option<NodeId> {
        let frame = self.reader_frame();
        if frame.width <= 0.0 {
            return None;
        }
        let mut children = Vec::new();
        if let Some(thread) = &self.thread {
            for (index, entry) in thread.entries.iter().enumerate() {
                let Some(reading) = &entry.reading else {
                    continue;
                };
                let sender = match reading.content.envelope.sender().shown() {
                    "" => tr!("No Sender"),
                    shown => shown.to_owned(),
                };
                let date = reading
                    .content
                    .envelope
                    .date
                    .map(|date| full_date(date, &self.zone, &self.formats))
                    .unwrap_or_default();
                let mut article = clickable(
                    Role::Article,
                    &tr!("From {sender}, {date}", sender = sender, date = date),
                );
                let mut text = Node::new(Role::Label);
                text.set_value(reading.document.text());
                self.push(tree, Part::Text(index), text, None);
                article.set_children(
                    self.node_of(Part::Text(index))
                        .into_iter()
                        .collect::<Vec<_>>(),
                );
                let bounds =
                    self.bounds_of(|hit| *hit == Hit::Reader(Target::Card(entry.summary.id)));
                self.push(tree, Part::Body(index), article, bounds);
                children.extend(self.node_of(Part::Body(index)));
            }
        }
        for (index, (rect, hit)) in self.hits.iter().enumerate() {
            let Hit::Reader(target) = hit else {
                continue;
            };
            let Some((role, label)) = self.target_label(target) else {
                continue;
            };
            let mut node = clickable(role, &label);
            if let Target::Link(url, ..) = target {
                node.set_url(url.as_str());
            }
            self.push(tree, Part::Target(index), node, Some(*rect));
            children.extend(self.node_of(Part::Target(index)));
        }
        let mut node = Node::new(Role::Document);
        node.set_label(tr!("Message"));
        node.set_bounds(access_rect(frame));
        node.add_action(Action::Focus);
        node.set_children(children);
        self.push(tree, Part::Reader, node, None);
        self.node_of(Part::Reader)
    }

    pub(crate) fn perform_on_panes(&mut self, request: &ActionRequest) -> Option<bool> {
        let part = self.part_of(request.target_node)?;
        if !matches!(request.action, Action::Click | Action::Focus) {
            return Some(false);
        }
        self.ui.clear_focus();
        match part {
            Part::Sidebar => self.focus = Pane::Sidebar,
            Part::List => self.focus = Pane::List,
            Part::Reader => self.focus = Pane::Reader,
            Part::Place(index) => {
                let Some(Row::Place { place, .. }) = self.sidebar.get(index).cloned() else {
                    return Some(false);
                };
                self.focus = Pane::Sidebar;
                if place != self.place {
                    self.choose_place(place);
                }
            }
            Part::Offline(index) => {
                let Some(Row::Heading(title)) = self.sidebar.get(index) else {
                    return Some(false);
                };
                let account = self
                    .accounts
                    .iter()
                    .find(|(account, _)| account.email == *title)?
                    .0
                    .id;
                self.focus = Pane::Sidebar;
                if request.action == Action::Click {
                    self.show_failure(account);
                }
            }
            Part::Message(index) => {
                let id = self.messages.get(index)?.id;
                if !self.is_chosen(id) || !self.also.is_empty() {
                    self.select(id);
                }
                self.focus = Pane::List;
            }
            Part::Conversation(index) => {
                let id = self.messages.get(index)?.id;
                self.focus = Pane::List;
                if request.action == Action::Click {
                    self.toggle_conversation(id);
                }
            }
            Part::Body(index) | Part::Text(index) => {
                let id = self.thread.as_ref()?.entries.get(index)?.summary.id;
                self.focus = Pane::Reader;
                self.focus_entry(id);
            }
            Part::Target(index) => {
                let (rect, Hit::Reader(target)) = self.hits.get(index).cloned()? else {
                    return Some(false);
                };
                self.focus = Pane::Reader;
                if request.action == Action::Click {
                    self.click_reader(target, &Pointer::at(centre(rect)));
                }
            }
        }
        Some(true)
    }
}
