use std::time::{Duration, Instant};

use accesskit::{Node, NodeId};
use bande_graphics::Point;

use super::menu_node;
use super::model::Menu;
use super::panel::MenuPanel;
use crate::{Key, Modifiers};

const SUBMENU_DELAY: Duration = Duration::from_millis(200);
const SAFE_DELAY: Duration = Duration::from_millis(300);
const SAFE_MARGIN: f32 = 6.0;
const TYPE_SELECT_PAUSE: Duration = Duration::from_secs(1);
const FLASH_STEP: Duration = Duration::from_millis(50);
const FLASH: Duration = Duration::from_millis(100);
const SCROLL_SPEED: f32 = 300.0;
const SCROLL_STEP: Duration = Duration::from_millis(16);

#[derive(Debug, Clone, PartialEq)]
pub enum MenuOutcome {
    Nothing,
    Redraw,
    OpenSubmenu { level: usize, index: usize },
    CloseFrom(usize),
    Activate(String),
    Dismiss,
    Search(Menu),
}

pub(super) fn entered(mut panel: MenuPanel, from_keyboard: bool) -> MenuPanel {
    if from_keyboard {
        panel.highlighted = panel.step(None, true);
    }
    panel
}

pub struct MenuTracker {
    panels: Vec<MenuPanel>,
    typed: String,
    typed_at: Option<Instant>,
    submenu_due: Option<(usize, usize, Instant)>,
    armed: bool,
    flash: Option<(usize, Instant)>,
    searched: Option<(Menu, String)>,
    revision: u32,
    pointer: Option<(usize, Point)>,
    settle: Option<(usize, Point, Instant)>,
    autoscroll: Option<(usize, f32, Instant)>,
}

pub(super) fn inside_triangle(point: Point, corners: [Point; 3]) -> bool {
    let side = |a: Point, b: Point| (b.x - a.x) * (point.y - a.y) - (b.y - a.y) * (point.x - a.x);
    let [a, b, c] = corners;
    let sides = [side(a, b), side(b, c), side(c, a)];
    sides.iter().all(|value| *value >= 0.0) || sides.iter().all(|value| *value <= 0.0)
}

impl MenuTracker {
    pub fn new(root: MenuPanel, from_keyboard: bool) -> Self {
        let searched = root
            .menu()
            .search
            .is_some()
            .then(|| (root.menu().clone(), String::new()));
        Self {
            panels: vec![entered(root, from_keyboard)],
            typed: String::new(),
            typed_at: None,
            submenu_due: None,
            armed: false,
            flash: None,
            searched,
            revision: 0,
            pointer: None,
            settle: None,
            autoscroll: None,
        }
    }

    pub fn revision(&self) -> u32 {
        self.revision
    }

    pub fn searched(&mut self, panel: MenuPanel) {
        let width = self.panels[0].size().width;
        let mut panel = panel.at_least_as_wide_as(width);
        panel.highlighted = panel.step(None, true);
        self.panels = vec![panel];
        self.submenu_due = None;
        self.settle = None;
        self.revision = self.revision.wrapping_add(1);
    }

    fn search(&mut self, edit: impl FnOnce(&mut String)) -> MenuOutcome {
        let Some((source, query)) = &mut self.searched else {
            return MenuOutcome::Nothing;
        };
        let before = query.clone();
        edit(query);
        if *query == before {
            return MenuOutcome::Nothing;
        }
        let menu = if query.is_empty() {
            source.clone()
        } else {
            source.found(query)
        };
        MenuOutcome::Search(menu)
    }

    fn searching(&self) -> bool {
        self.searched
            .as_ref()
            .is_some_and(|(_, query)| !query.is_empty())
    }

    pub fn panels(&self) -> &[MenuPanel] {
        &self.panels
    }

    pub fn truncate(&mut self, levels: usize) {
        self.panels.truncate(levels.max(1));
        self.autoscroll = self
            .autoscroll
            .filter(|(level, _, _)| *level < self.panels.len());
    }

    pub fn submenu(&self, level: usize, index: usize) -> Option<Menu> {
        self.panels
            .get(level)?
            .menu()
            .items
            .get(index)?
            .submenu
            .clone()
    }

    fn highlight(&mut self, level: usize, index: Option<usize>, now: Instant) -> MenuOutcome {
        let Some(panel) = self.panels.get_mut(level) else {
            return MenuOutcome::Nothing;
        };
        if panel.highlighted == index {
            return MenuOutcome::Nothing;
        }
        panel.highlighted = index;
        if let Some(index) = index {
            panel.reveal(index);
        }
        self.submenu_due = index
            .filter(|index| panel.menu().items[*index].submenu.is_some())
            .map(|index| (level, index, now + SUBMENU_DELAY));
        if self.panels.len() > level + 1 {
            return MenuOutcome::CloseFrom(level + 1);
        }
        MenuOutcome::Redraw
    }

    pub fn pointer_move(&mut self, level: usize, point: Point, now: Instant) -> MenuOutcome {
        if self.flash.is_some() {
            return MenuOutcome::Nothing;
        }
        let Some(panel) = self.panels.get(level) else {
            return MenuOutcome::Nothing;
        };
        if let Some(direction) = panel.scroll_zone_at(point) {
            if self
                .autoscroll
                .is_none_or(|(known, toward, _)| known != level || toward != direction)
            {
                self.autoscroll = Some((level, direction, now));
            }
            self.pointer = Some((level, point));
            self.settle = None;
            return self.highlight(level, None, now);
        }
        if self.autoscroll.is_some_and(|(known, _, _)| known == level) {
            self.autoscroll = None;
        }
        let index = panel.item_at(point);
        if index.is_some() {
            self.armed = true;
        }
        let from = self
            .pointer
            .filter(|(known, _)| *known == level)
            .map(|(_, from)| from);
        self.pointer = Some((level, point));
        let keeps_submenu = index.is_none() && self.panels.len() > level + 1;
        if keeps_submenu {
            return MenuOutcome::Nothing;
        }
        if let (Some(from), Some(open)) = (from, self.heading_to_submenu(level))
            && index != Some(open)
            && self.toward_submenu(level, open, from, point)
        {
            self.settle = Some((level, point, now + SAFE_DELAY));
            return MenuOutcome::Nothing;
        }
        self.settle = None;
        self.highlight(level, index, now)
    }

    fn heading_to_submenu(&self, level: usize) -> Option<usize> {
        let open = self.panels.get(level)?.highlighted?;
        let opening = self
            .submenu_due
            .is_some_and(|(due, index, _)| due == level && index == open);
        (self.panels.len() > level + 1 || opening).then_some(open)
    }

    fn toward_submenu(&self, level: usize, open: usize, from: Point, point: Point) -> bool {
        let panel = &self.panels[level];
        let row = panel.row_rect(open);
        let height = self
            .panels
            .get(level + 1)
            .map_or(row.height, |submenu| submenu.size().height);
        let (edge, behind, moving_away) = match panel.submenus_open_left {
            true => (-SAFE_MARGIN, from.x + 1.0, point.x < from.x),
            false => (
                panel.size().width + SAFE_MARGIN,
                from.x - 1.0,
                point.x > from.x,
            ),
        };
        moving_away
            && inside_triangle(
                point,
                [
                    Point::new(behind, from.y),
                    Point::new(edge, row.y - SAFE_MARGIN),
                    Point::new(edge, row.y + height + SAFE_MARGIN),
                ],
            )
    }

    pub fn placed(&mut self, level: usize, height: f32, beside_left: bool) {
        if level > 0
            && let Some(parent) = self.panels.get_mut(level - 1)
        {
            parent.open_submenus_left(beside_left);
        }
        if let Some(panel) = self.panels.get_mut(level) {
            panel.fit_height(height);
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub fn scroll(&mut self, level: usize, delta: f32) -> MenuOutcome {
        let Some(panel) = self.panels.get_mut(level) else {
            return MenuOutcome::Nothing;
        };
        if !panel.scroll_by(delta) {
            return MenuOutcome::Nothing;
        }
        self.after_scroll(level)
    }

    fn after_scroll(&mut self, level: usize) -> MenuOutcome {
        let pointer = self
            .pointer
            .filter(|(known, _)| *known == level)
            .map(|(_, point)| point);
        let Some(panel) = self.panels.get_mut(level) else {
            return MenuOutcome::Nothing;
        };
        if let Some(point) = pointer {
            panel.highlighted = panel.item_at(point);
            self.submenu_due = None;
        }
        if self.panels.len() > level + 1 {
            return MenuOutcome::CloseFrom(level + 1);
        }
        MenuOutcome::Redraw
    }

    fn step_autoscroll(&mut self, now: Instant) -> MenuOutcome {
        let Some((level, direction, at)) = self.autoscroll else {
            return MenuOutcome::Nothing;
        };
        if now < at + SCROLL_STEP {
            return MenuOutcome::Nothing;
        }
        let distance = direction * SCROLL_SPEED * now.duration_since(at).as_secs_f32();
        let Some(panel) = self.panels.get_mut(level) else {
            return MenuOutcome::Nothing;
        };
        let moved = panel.scroll_by(distance);
        let more = match direction > 0.0 {
            true => panel.scrolls_down(),
            false => panel.scrolls_up(),
        };
        self.autoscroll = (moved && more).then_some((level, direction, now));
        match moved {
            true => self.after_scroll(level),
            false => MenuOutcome::Nothing,
        }
    }

    pub fn pointer_press(&mut self, level: usize, point: Point) -> MenuOutcome {
        let Some(panel) = self.panels.get(level) else {
            return MenuOutcome::Nothing;
        };
        self.armed = true;
        match panel.item_at(point) {
            Some(index) if panel.menu().items[index].submenu.is_some() => {
                self.open_submenu(level, index)
            }
            _ => MenuOutcome::Nothing,
        }
    }

    pub fn pointer_release(&mut self, level: usize, point: Point, now: Instant) -> MenuOutcome {
        if !self.armed || self.flash.is_some() {
            return MenuOutcome::Nothing;
        }
        let Some(index) = self
            .panels
            .get(level)
            .and_then(|panel| panel.item_at(point))
        else {
            return MenuOutcome::Nothing;
        };
        self.activate(level, index, now)
    }

    fn open_submenu(&mut self, level: usize, index: usize) -> MenuOutcome {
        self.submenu_due = None;
        if self.panels.len() > level + 1 {
            return MenuOutcome::Nothing;
        }
        MenuOutcome::OpenSubmenu { level, index }
    }

    pub fn activate(&mut self, level: usize, index: usize, now: Instant) -> MenuOutcome {
        let Some(panel) = self.panels.get_mut(level) else {
            return MenuOutcome::Nothing;
        };
        let Some(item) = panel
            .menu()
            .items
            .get(index)
            .filter(|item| item.selectable())
        else {
            return MenuOutcome::Nothing;
        };
        if item.submenu.is_some() {
            return self.open_submenu(level, index);
        }
        let action = item.action.clone();
        panel.highlighted = Some(index);
        self.flash = Some((level, now));
        self.submenu_due = None;
        MenuOutcome::Activate(action)
    }

    pub fn key(&mut self, key: Key, modifiers: Modifiers, now: Instant) -> MenuOutcome {
        if self.flash.is_some() {
            return MenuOutcome::Nothing;
        }
        let level = self.panels.len() - 1;
        let current = self.panels[level].highlighted;
        match key {
            Key::Up | Key::Down => {
                let next = self.panels[level].step(current, key == Key::Down);
                self.highlight(level, next, now)
            }
            Key::Home | Key::End => {
                let next = self.panels[level].step(None, key == Key::Home);
                self.highlight(level, next, now)
            }
            Key::Right => match current {
                Some(index) if self.panels[level].menu().items[index].submenu.is_some() => {
                    self.open_submenu(level, index)
                }
                _ => MenuOutcome::Nothing,
            },
            Key::Left if level > 0 => MenuOutcome::CloseFrom(level),
            Key::Escape if level > 0 => MenuOutcome::CloseFrom(level),
            Key::Escape if self.searching() => self.search(String::clear),
            Key::Escape => MenuOutcome::Dismiss,
            Key::Backspace if self.searching() => self.search(|query| {
                query.pop();
            }),
            Key::Enter | Key::Space => match current {
                Some(index) => self.activate(level, index, now),
                None => MenuOutcome::Nothing,
            },
            Key::Character(character)
                if !modifiers.logo && !modifiers.control && self.searched.is_some() =>
            {
                self.search(|query| query.push(character))
            }
            Key::Character(character) if !modifiers.logo && !modifiers.control => {
                self.type_select(level, character, now)
            }
            _ => MenuOutcome::Nothing,
        }
    }

    fn type_select(&mut self, level: usize, character: char, now: Instant) -> MenuOutcome {
        let fresh = self
            .typed_at
            .is_none_or(|at| now.duration_since(at) > TYPE_SELECT_PAUSE);
        if fresh {
            self.typed.clear();
        }
        self.typed.extend(character.to_lowercase());
        self.typed_at = Some(now);
        let typed = self.typed.clone();
        let found =
            self.panels[level].menu().items.iter().position(|item| {
                item.selectable() && item.title.to_lowercase().starts_with(&typed)
            });
        match found {
            Some(index) => self.highlight(level, Some(index), now),
            None => MenuOutcome::Nothing,
        }
    }

    pub fn show_alternates(&mut self, modifiers: Modifiers) -> MenuOutcome {
        let shown = match (modifiers.alt, modifiers.shift) {
            (true, _) => 1,
            (false, true) => 2,
            (false, false) => 0,
        };
        let mut changed = false;
        for panel in &mut self.panels {
            changed |= panel.alternates != shown;
            panel.alternates = shown;
        }
        match changed {
            true => MenuOutcome::Redraw,
            false => MenuOutcome::Nothing,
        }
    }

    pub fn opened_submenu(&mut self, panel: MenuPanel, from_keyboard: bool) {
        let mut panel = entered(panel, from_keyboard);
        panel.alternates = self.panels.first().map_or(0, |root| root.alternates);
        self.panels.push(panel);
    }

    pub fn deadline(&self) -> Option<Instant> {
        let submenu = self.submenu_due.map(|(_, _, at)| at);
        let flash = self.flash.map(|(_, at)| at + FLASH);
        let settle = self.settle.map(|(_, _, at)| at);
        let autoscroll = self.autoscroll.map(|(_, _, at)| at + SCROLL_STEP);
        submenu
            .into_iter()
            .chain(flash)
            .chain(settle)
            .chain(autoscroll)
            .min()
    }

    pub fn tick(&mut self, now: Instant) -> MenuOutcome {
        let scrolled = self.step_autoscroll(now);
        if scrolled != MenuOutcome::Nothing {
            return scrolled;
        }
        if let Some((level, point, at)) = self.settle
            && at <= now
        {
            self.settle = None;
            let index = self
                .panels
                .get(level)
                .and_then(|panel| panel.item_at(point));
            return self.highlight(level, index, now);
        }
        if let Some((level, index, at)) = self.submenu_due
            && at <= now
        {
            return self.open_submenu(level, index);
        }
        if self.is_finished(now) {
            return MenuOutcome::Dismiss;
        }
        MenuOutcome::Nothing
    }

    pub fn is_finished(&self, now: Instant) -> bool {
        self.flash.is_some_and(|(_, at)| now >= at + FLASH)
    }

    pub fn is_flashing(&self) -> bool {
        self.flash.is_some()
    }

    pub fn shows_highlight(&self, level: usize, now: Instant) -> bool {
        self.flash.is_none_or(|(flashed, at)| {
            level != flashed || now.saturating_duration_since(at) >= FLASH_STEP
        })
    }

    pub fn access(&self) -> (Vec<(NodeId, Node)>, Vec<NodeId>, Option<NodeId>) {
        let mut nodes = Vec::new();
        let mut roots = Vec::new();
        for (level, panel) in self.panels.iter().enumerate() {
            roots.push(menu_node(level, None));
            nodes.extend(panel.access(level));
        }
        let level = self.panels.len() - 1;
        let focus = self.panels[level]
            .highlighted
            .map(|index| menu_node(level, Some(index)))
            .or(Some(menu_node(level, None)));
        (nodes, roots, focus)
    }

    pub fn item_for_node(&self, node: NodeId) -> Option<(usize, usize)> {
        self.panels.iter().enumerate().find_map(|(level, panel)| {
            (0..panel.menu().items.len())
                .find(|index| menu_node(level, Some(*index)) == node)
                .map(|index| (level, index))
        })
    }
}
