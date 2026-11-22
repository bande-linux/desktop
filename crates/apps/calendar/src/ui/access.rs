use accesskit::{Action, ActionRequest, Node, NodeId, Role, Toggled, TreeUpdate};
use bande_l10n::{tr, trn};
use bande_toolkit::Rect;
use bande_toolkit::ui::access_rect;
use jiff::ToSpan;
use jiff::civil::Date;

use super::paint::{summary, when};
use super::sidebar::account_title;
use super::{CalendarView, Hit, Mode, Pane, Part as Edge};

const KIND_SHIFT: u32 = 28;
const PAYLOAD_LIMIT: u64 = 1 << KIND_SHIFT;
const YEAR_OFFSET: i64 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Sidebar,
    Account(usize),
    Calendar(usize),
    MiniMonth,
    MiniShift(bool),
    MiniDay(Date),
    Content,
    Day(Date),
    Month(Date),
    Event(usize),
    More(Date),
    Results,
    Result(usize),
}

fn date_payload(day: Date) -> u64 {
    let payload = (i64::from(day.year()) + YEAR_OFFSET) * 10_000
        + i64::from(day.month()) * 100
        + i64::from(day.day());
    payload.unsigned_abs()
}

fn payload_date(payload: u64) -> Option<Date> {
    let payload = i64::try_from(payload).ok()?;
    let year = i16::try_from(payload / 10_000 - YEAR_OFFSET).ok()?;
    Date::new(year, (payload / 100 % 100) as i8, (payload % 100) as i8).ok()
}

impl Part {
    fn index(self) -> Option<u64> {
        let (kind, payload) = match self {
            Self::Sidebar => (0, 0),
            Self::Account(index) => (1, index as u64),
            Self::Calendar(index) => (2, index as u64),
            Self::MiniMonth => (3, 0),
            Self::MiniShift(next) => (4, u64::from(next)),
            Self::MiniDay(day) => (5, date_payload(day)),
            Self::Content => (6, 0),
            Self::Day(day) => (7, date_payload(day)),
            Self::Month(day) => (8, date_payload(day)),
            Self::Event(index) => (9, index as u64),
            Self::More(day) => (10, date_payload(day)),
            Self::Results => (11, 0),
            Self::Result(index) => (12, index as u64),
        };
        (payload < PAYLOAD_LIMIT).then_some((kind << KIND_SHIFT) | payload)
    }

    fn from_index(index: u64) -> Option<Self> {
        let payload = index & (PAYLOAD_LIMIT - 1);
        let position = usize::try_from(payload).ok();
        Some(match index >> KIND_SHIFT {
            0 => Self::Sidebar,
            1 => Self::Account(position?),
            2 => Self::Calendar(position?),
            3 => Self::MiniMonth,
            4 => Self::MiniShift(payload == 1),
            5 => Self::MiniDay(payload_date(payload)?),
            6 => Self::Content,
            7 => Self::Day(payload_date(payload)?),
            8 => Self::Month(payload_date(payload)?),
            9 => Self::Event(position?),
            10 => Self::More(payload_date(payload)?),
            11 => Self::Results,
            12 => Self::Result(position?),
            _ => return None,
        })
    }

    fn group(self) -> Option<Self> {
        match self {
            Self::Account(_) | Self::Calendar(_) => Some(Self::Sidebar),
            Self::MiniShift(_) | Self::MiniDay(_) => Some(Self::MiniMonth),
            Self::Day(_) | Self::Month(_) | Self::Event(_) | Self::More(_) => Some(Self::Content),
            Self::Result(_) => Some(Self::Results),
            Self::Sidebar | Self::MiniMonth | Self::Content | Self::Results => None,
        }
    }
}

const GROUPS: [Part; 4] = [Part::Sidebar, Part::MiniMonth, Part::Content, Part::Results];

impl CalendarView {
    fn node_of(&self, part: Part) -> Option<NodeId> {
        part.index().map(|index| self.access.part(index))
    }

    fn part_of(&self, node: NodeId) -> Option<Part> {
        let owner = NodeId::from(self.access).0;
        (node.0 >> 32 == owner)
            .then(|| (node.0 & ((1 << 32) - 1)).checked_sub(1))
            .flatten()
            .and_then(Part::from_index)
    }

    fn part_of_hit(&self, hit: &Hit) -> Option<Part> {
        Some(match hit {
            Hit::Calendar(index) => Part::Calendar(*index),
            Hit::Account(id) => Part::Account(
                self.library
                    .accounts
                    .iter()
                    .position(|account| account.id == *id)?,
            ),
            Hit::MiniDay(day) => Part::MiniDay(*day),
            Hit::MiniShift(steps) => Part::MiniShift(*steps > 0),
            Hit::Event(index, Edge::Body) => Part::Event(*index),
            Hit::Event(_, Edge::End) => return None,
            Hit::AllDay(day) | Hit::MonthDay(day) | Hit::YearDay(day) => Part::Day(*day),
            Hit::YearMonth(day) => Part::Month(*day),
            Hit::More(day) => Part::More(*day),
            Hit::Result(index) => Part::Result(*index),
        })
    }

    fn day_label(&self, day: Date) -> String {
        let date = day
            .to_zoned(self.zone.clone())
            .map_or_else(|_| day.to_string(), |at| self.formats.long_date(&at));
        let count = self
            .shown
            .iter()
            .filter(|found| found.occurrence.first_day <= day && day <= found.occurrence.last_day)
            .count();
        match count {
            0 => date,
            count => trn!(
                "{date}, {count} event",
                "{date}, {count} events",
                count = count,
                date = date
            ),
        }
    }

    fn part_node(&self, part: Part, bounds: Option<Rect>) -> Option<Node> {
        let mut node = match part {
            Part::Sidebar => {
                let mut node = Node::new(Role::Group);
                node.set_label(tr!("Calendars"));
                node
            }
            Part::Account(index) => {
                let mut node = Node::new(Role::Heading);
                node.set_label(account_title(self.library.accounts.get(index)?));
                node.set_level(2);
                node
            }
            Part::Calendar(index) => {
                let calendar = self.library.calendars.get(index)?;
                let mut node = Node::new(Role::CheckBox);
                node.set_label(calendar.info.name.as_str());
                node.set_toggled(match calendar.info.visible {
                    true => Toggled::True,
                    false => Toggled::False,
                });
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
                node
            }
            Part::MiniMonth => {
                let mut node = Node::new(Role::Group);
                node.set_label(self.formats.month_title(self.mini_month));
                node
            }
            Part::MiniShift(next) => {
                let mut node = Node::new(Role::Button);
                node.set_label(match next {
                    true => tr!("Next Month"),
                    false => tr!("Previous Month"),
                });
                node.add_action(Action::Click);
                node
            }
            Part::MiniDay(day) => {
                let mut node = Node::new(Role::Button);
                node.set_label(
                    day.to_zoned(self.zone.clone())
                        .map_or_else(|_| day.to_string(), |at| self.formats.long_date(&at)),
                );
                node.add_action(Action::Click);
                node
            }
            Part::Content => {
                let mut node = Node::new(Role::Group);
                node.set_label(self.title());
                node.add_action(Action::Focus);
                node
            }
            Part::Day(day) => {
                let mut node = Node::new(Role::Button);
                node.set_label(self.day_label(day));
                node.add_action(Action::Focus);
                node.add_action(Action::Click);
                node
            }
            Part::Month(first) => {
                let mut node = Node::new(Role::Button);
                node.set_label(self.formats.month_title(first));
                node.add_action(Action::Click);
                node
            }
            Part::Event(index) => {
                let found = self.shown.get(index)?;
                let mut node = Node::new(Role::Button);
                node.set_label(summary(&self.library, found));
                node.set_description(self.event_description(found));
                node.set_selected(self.is_selected(found));
                node.add_action(Action::Focus);
                node.add_action(Action::Click);
                node
            }
            Part::More(_) => {
                let mut node = Node::new(Role::Button);
                node.set_label(tr!("Show All Events"));
                node.add_action(Action::Click);
                node
            }
            Part::Results => {
                let mut node = Node::new(Role::ListBox);
                node.set_label(tr!("Search Results"));
                node
            }
            Part::Result(index) => {
                let found = self.results.get(index)?;
                let mut node = Node::new(Role::ListBoxOption);
                node.set_label(summary(&self.library, found));
                node.set_description(self.event_description(found));
                node.set_selected(self.is_selected(found));
                node.add_action(Action::Focus);
                node.add_action(Action::Click);
                node
            }
        };
        if let Some(bounds) = bounds {
            node.set_bounds(access_rect(bounds));
        }
        Some(node)
    }

    fn is_selected(&self, found: &crate::store::Found) -> bool {
        self.selected
            .as_ref()
            .is_some_and(|selected| self.selection_of(found) == *selected)
    }

    fn event_description(&self, found: &crate::store::Found) -> String {
        let when = when(&self.formats, found);
        match self.library.calendars.get(found.calendar) {
            Some(calendar) => format!("{when}, {}", calendar.info.name),
            None => when,
        }
    }

    pub(super) fn describe(&self, tree: &mut TreeUpdate) {
        let mut parts: Vec<(Part, Rect)> = Vec::new();
        for (rect, hit) in &self.hits {
            if let Some(part) = self.part_of_hit(hit)
                && !parts.iter().any(|(known, _)| *known == part)
            {
                parts.push((part, *rect));
            }
        }
        let mut groups = Vec::new();
        for group in GROUPS {
            let members: Vec<(Part, Rect)> = parts
                .iter()
                .filter(|(part, _)| part.group() == Some(group))
                .copied()
                .collect();
            if members.is_empty() {
                continue;
            }
            let (Some(id), Some(mut node)) = (self.node_of(group), self.part_node(group, None))
            else {
                continue;
            };
            let mut children = Vec::with_capacity(members.len());
            for (part, rect) in members {
                if let (Some(child), Some(member)) =
                    (self.node_of(part), self.part_node(part, Some(rect)))
                {
                    children.push(child);
                    tree.nodes.push((child, member));
                }
            }
            node.set_children(children);
            tree.nodes.push((id, node));
            groups.push(id);
        }
        if let Some((_, root)) = tree.nodes.first_mut() {
            for group in &groups {
                root.push_child(*group);
            }
        }
        if self.ui.focus().is_some() || self.ui.has_sheet() {
            return;
        }
        let selected = self
            .selected_found()
            .and_then(|found| self.shown.iter().position(|shown| *shown == found))
            .map(Part::Event);
        let wanted = match self.pane {
            Pane::Sidebar(index) => vec![Part::Calendar(index)],
            Pane::Content => selected
                .into_iter()
                .chain([Part::Day(self.focus), Part::Content])
                .collect(),
        };
        if let Some(focus) = wanted
            .into_iter()
            .filter_map(|part| self.node_of(part))
            .find(|id| tree.nodes.iter().any(|(node, _)| node == id))
        {
            tree.focus = focus;
        }
    }

    pub(super) fn perform_on_parts(&mut self, request: &ActionRequest) -> Option<bool> {
        let part = self.part_of(request.target_node)?;
        let click = request.action == Action::Click;
        Some(match (part, request.action) {
            (Part::Calendar(index), Action::Click | Action::Focus) => {
                self.ui.clear_focus();
                self.pane = Pane::Sidebar(index);
                if click {
                    self.toggle_calendar(index);
                }
                true
            }
            (Part::MiniShift(next), Action::Click) => {
                let steps: i64 = match next {
                    true => 1,
                    false => -1,
                };
                self.mini_month = self.mini_month.saturating_add(steps.months());
                true
            }
            (Part::MiniDay(day), Action::Click) => {
                self.go_to(day);
                true
            }
            (Part::Content, Action::Focus) => {
                self.ui.clear_focus();
                self.pane = Pane::Content;
                true
            }
            (Part::Day(day), Action::Click | Action::Focus) => {
                self.focus_day(day);
                true
            }
            (Part::Month(first), Action::Click) => {
                self.go_to(first);
                self.set_mode(Mode::Month);
                true
            }
            (Part::More(day), Action::Click) => {
                self.go_to(day);
                self.set_mode(Mode::Day);
                true
            }
            (Part::Event(index), Action::Click | Action::Focus) => {
                let Some(found) = self.shown.get(index).cloned() else {
                    return Some(false);
                };
                self.ui.clear_focus();
                self.pane = Pane::Content;
                self.selected = Some(self.selection_of(&found));
                if click {
                    self.open_inspector(found);
                }
                true
            }
            (Part::Result(index), Action::Click | Action::Focus) => {
                let Some(found) = self.results.get(index).cloned() else {
                    return Some(false);
                };
                self.go_to(found.occurrence.first_day);
                self.selected = Some(self.selection_of(&found));
                if click {
                    self.pending_open = self.selected.clone();
                }
                true
            }
            _ => false,
        })
    }
}
