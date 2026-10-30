mod card;

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::time::Instant;

use jiff::Zoned;
use unbound_datetime::DateFormats;
use unbound_l10n::Localizer;
use unbound_notify::{Notification, Presentation, Stack};
use unbound_spring::rubber_band;
use unbound_toolkit::ui::{Animated, Event, Key, Spring};
use unbound_toolkit::{
    Color, Item, Palette, Paragraph, Point, Rect, Scene, Shape, TextAlign, TextStyle, TextSystem,
};

use card::{CLOSE_REACH, contains};
pub use card::{Card, DEFAULT_ACTION, Part, Sender, age_label};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-notifications.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub const BANNER_WIDTH: f32 = 344.0;
pub const CENTER_WIDTH: f32 = 368.0;
const MARGIN: f32 = 12.0;
const GAP: f32 = 10.0;
const MAX_BANNERS: usize = 4;
const BANNER_BODY_LINES: usize = 3;
const CENTER_BODY_LINES: usize = 5;
const CENTER_PADDING: f32 = 12.0;
const CENTER_RADIUS: f32 = 20.0;
const HEADER_HEIGHT: f32 = 36.0;
const GROUP_HEIGHT: f32 = 28.0;
const TITLE_SIZE: f32 = 15.0;
const SMALL_SIZE: f32 = 12.0;
const DRAG_SLOP: f32 = 4.0;
const BLUR_BANDS: usize = 5;
const CARD_ALPHA: u8 = 214;
const PANEL_ALPHA: u8 = 190;
const DISMISS_FRACTION: f32 = 0.35;
const FLICK_SPEED: f32 = 600.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Activate {
        id: u32,
        action: String,
        group: String,
    },
    Dismiss(u32),
    HideBanner(u32),
    Clear(String),
    ClearAll,
    HoldBanners(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    ClearAll,
    ShowLess(String),
    ClearGroup(String),
    Card(usize, Part),
}

struct Banner {
    card: Card,
    x: Animated,
    y: Animated,
    leaving: bool,
}

struct Drag {
    group: String,
    start: Point,
    last: (f32, Instant),
    velocity: f32,
    moved: bool,
}

enum Row {
    Title {
        title: Paragraph,
        clear_all: Option<(Paragraph, Rect)>,
        status: Option<Paragraph>,
    },
    Group {
        group: String,
        name: Paragraph,
        show_less: (Paragraph, Rect),
        clear: Rect,
    },
    Card {
        card: Card,
        collapsed: bool,
    },
    Empty(Paragraph),
}

struct PlacedRow {
    y: f32,
    height: f32,
    row: Row,
}

pub type Senders = Box<dyn FnMut(&Notification, &str) -> Sender>;

pub struct Screen {
    palette: Palette,
    text: TextSystem,
    reduce_motion: bool,
    senders: Senders,
    presentation: Presentation,
    first_seen: HashMap<u32, Instant>,
    dates: DateFormats,
    banners: Vec<Banner>,
    open: bool,
    center_x: Animated,
    rows: Vec<PlacedRow>,
    expanded: HashSet<String>,
    scroll: f32,
    focus: Option<usize>,
    pointer: Option<Point>,
    pressed: Option<Target>,
    drag: Option<Drag>,
    holding: bool,
    translucent: bool,
    size: (f32, f32),
    requests: Vec<Request>,
}

fn hidden_offset(width: f32) -> f32 {
    width + MARGIN + CLOSE_REACH
}

impl Screen {
    pub fn new(palette: Palette, text: TextSystem, senders: Senders) -> Self {
        Self {
            palette,
            text,
            reduce_motion: false,
            senders,
            presentation: Presentation::default(),
            first_seen: HashMap::new(),
            dates: DateFormats::from_env(),
            banners: Vec::new(),
            open: false,
            center_x: Animated::new(hidden_offset(CENTER_WIDTH)),
            rows: Vec::new(),
            expanded: HashSet::new(),
            scroll: 0.0,
            focus: None,
            pointer: None,
            pressed: None,
            drag: None,
            holding: false,
            translucent: false,
            size: (0.0, 0.0),
            requests: Vec::new(),
        }
    }

    pub fn set_appearance(&mut self, palette: Palette, reduce_motion: bool, now: Instant) {
        self.palette = palette;
        self.reduce_motion = reduce_motion;
        self.rebuild(now);
    }

    pub fn set_translucent(&mut self, translucent: bool) {
        self.translucent = translucent;
    }

    fn material(&self) -> Palette {
        let mut palette = self.palette;
        if self.translucent {
            palette.overlay = palette.overlay.with_alpha(CARD_ALPHA);
            palette.sheet = palette.sheet.with_alpha(PANEL_ALPHA);
        }
        palette
    }

    pub fn blur_rects(&self, now: Instant) -> Vec<Rect> {
        let mut rects = Vec::new();
        if self.banners_visible() {
            for banner in &self.banners {
                let origin = self.banner_origin(banner, now);
                let card = Rect::new(origin.x, origin.y, BANNER_WIDTH, banner.card.height);
                rects.extend(Shape::rounded(card, card::RADIUS).bands(BLUR_BANDS));
            }
        }
        if self.center_visible(now) {
            rects.extend(Shape::rounded(self.panel_rect(now), CENTER_RADIUS).bands(BLUR_BANDS));
        }
        rects
    }

    pub fn set_size(&mut self, width: u32, height: u32) {
        self.size = (width as f32, height as f32);
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn present(&mut self, presentation: Presentation, now: Instant) {
        for stack in presentation.center.iter().chain(&presentation.banners) {
            for notification in &stack.notifications {
                self.first_seen.entry(notification.id).or_insert(now);
            }
        }
        let known: HashSet<u32> = presentation
            .center
            .iter()
            .chain(&presentation.banners)
            .flat_map(|stack| stack.notifications.iter().map(|found| found.id))
            .collect();
        self.first_seen.retain(|id, _| known.contains(id));
        self.expanded.retain(|group| {
            presentation
                .center
                .iter()
                .any(|stack| &stack.group == group && stack.notifications.len() > 1)
        });
        self.presentation = presentation;
        self.rebuild(now);
    }

    pub fn rebuild(&mut self, now: Instant) {
        self.rebuild_banners(now);
        self.rebuild_center(now);
    }

    fn card(
        &mut self,
        stack: &Stack,
        count: usize,
        index: usize,
        now: Instant,
        wide: bool,
    ) -> Card {
        let notification = &stack.notifications[index];
        let sender = (self.senders)(notification, &stack.group);
        let age = self
            .first_seen
            .get(&notification.id)
            .map(|seen| now.saturating_duration_since(*seen))
            .unwrap_or_default();
        let wall = Zoned::now();
        let age = age_label(&wall.saturating_sub(age), &wall, &self.dates);
        let (width, lines) = if wide {
            (CENTER_WIDTH - 2.0 * CENTER_PADDING, CENTER_BODY_LINES)
        } else {
            (BANNER_WIDTH, BANNER_BODY_LINES)
        };
        Card::new(
            notification,
            &stack.group,
            count,
            sender,
            &age,
            width,
            lines,
            &self.palette,
            &mut self.text,
        )
    }

    fn rebuild_banners(&mut self, now: Instant) {
        let stacks: Vec<Stack> = self
            .presentation
            .banners
            .iter()
            .take(MAX_BANNERS)
            .cloned()
            .collect();
        let mut kept = Vec::new();
        let mut y = MARGIN;
        for stack in &stacks {
            let card = self.card(stack, stack.notifications.len(), 0, now, false);
            let height = card.height + card.depth();
            let banner = match self
                .banners
                .iter()
                .position(|banner| banner.card.group == stack.group && !banner.leaving)
            {
                Some(index) => {
                    let mut banner = self.banners.remove(index);
                    banner.card = card;
                    banner
                        .y
                        .animate_at(now, self.reduce_motion, y, Spring::STANDARD);
                    banner
                }
                None => {
                    let mut x = Animated::new(hidden_offset(BANNER_WIDTH));
                    x.animate_at(now, self.reduce_motion, 0.0, Spring::STANDARD);
                    Banner {
                        card,
                        x,
                        y: Animated::new(y),
                        leaving: false,
                    }
                }
            };
            kept.push(banner);
            y += height + GAP;
        }
        for mut banner in self.banners.drain(..) {
            if !banner.leaving {
                banner.leaving = true;
                banner.x.animate_at(
                    now,
                    self.reduce_motion,
                    hidden_offset(BANNER_WIDTH),
                    Spring::STANDARD,
                );
            }
            kept.push(banner);
        }
        self.banners = kept;
        if self.drag.as_ref().is_some_and(|drag| {
            !self
                .banners
                .iter()
                .any(|banner| banner.card.group == drag.group)
        }) {
            self.drag = None;
        }
    }

    fn rebuild_center(&mut self, now: Instant) {
        let palette = self.palette;
        let mut rows = Vec::new();
        let title = self.text.paragraph(
            &L10N.tr("notification-center"),
            TextStyle::new(TITLE_SIZE, palette.ink).weight(TextStyle::SEMIBOLD),
            None,
            TextAlign::Start,
        );
        let inner = CENTER_WIDTH - 2.0 * CENTER_PADDING;
        let stacks = self.presentation.center.clone();
        let clear_all = (!stacks.is_empty()).then(|| {
            let label = self.text.paragraph(
                &L10N.tr("clear-all"),
                TextStyle::new(SMALL_SIZE, palette.ink).weight(TextStyle::MEDIUM),
                None,
                TextAlign::Start,
            );
            let width = label.width() + 20.0;
            let rect = Rect::new(inner - width, (HEADER_HEIGHT - 24.0) / 2.0, width, 24.0);
            (label, rect)
        });
        let status = self.presentation.do_not_disturb.then(|| {
            self.text.paragraph(
                &L10N.tr("do-not-disturb-on"),
                TextStyle::new(SMALL_SIZE, palette.ink2),
                Some(inner),
                TextAlign::Start,
            )
        });
        let mut y = CENTER_PADDING;
        let title_height = HEADER_HEIGHT + status.as_ref().map_or(0.0, Paragraph::height);
        rows.push(PlacedRow {
            y,
            height: title_height,
            row: Row::Title {
                title,
                clear_all,
                status,
            },
        });
        y += title_height + GAP / 2.0;
        if stacks.is_empty() {
            let empty = self.text.paragraph(
                &L10N.tr("no-notifications"),
                TextStyle::new(13.0, palette.ink2),
                Some(inner),
                TextAlign::Center,
            );
            let height = empty.height() + 2.0 * HEADER_HEIGHT;
            rows.push(PlacedRow {
                y,
                height,
                row: Row::Empty(empty),
            });
            y += height;
        }
        for stack in &stacks {
            let expanded = self.expanded.contains(&stack.group);
            if expanded {
                let name = (self.senders)(&stack.notifications[0], &stack.group).name;
                let name = self.text.paragraph(
                    &name,
                    TextStyle::new(13.0, palette.ink).weight(TextStyle::SEMIBOLD),
                    Some(inner / 2.0),
                    TextAlign::Start,
                );
                let label = self.text.paragraph(
                    &L10N.tr("show-less"),
                    TextStyle::new(SMALL_SIZE, palette.ink).weight(TextStyle::MEDIUM),
                    None,
                    TextAlign::Start,
                );
                let clear = Rect::new(inner - 24.0, 2.0, 24.0, 24.0);
                let width = label.width() + 20.0;
                let show_less = Rect::new(clear.x - 6.0 - width, 2.0, width, 24.0);
                rows.push(PlacedRow {
                    y,
                    height: GROUP_HEIGHT,
                    row: Row::Group {
                        group: stack.group.clone(),
                        name,
                        show_less: (label, show_less),
                        clear,
                    },
                });
                y += GROUP_HEIGHT + GAP / 2.0;
                for index in 0..stack.notifications.len() {
                    let card = self.card(stack, 1, index, now, true);
                    let height = card.height;
                    rows.push(PlacedRow {
                        y,
                        height,
                        row: Row::Card {
                            card,
                            collapsed: false,
                        },
                    });
                    y += height + GAP;
                }
            } else {
                let count = stack.notifications.len();
                let card = self.card(stack, count, 0, now, true);
                let height = card.height + card.depth();
                rows.push(PlacedRow {
                    y,
                    height,
                    row: Row::Card {
                        card,
                        collapsed: count > 1,
                    },
                });
                y += height + GAP;
            }
        }
        self.rows = rows;
        let targets = self.targets().len();
        self.focus = self.focus.filter(|focus| *focus < targets);
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
    }

    fn content_height(&self) -> f32 {
        self.rows
            .last()
            .map_or(0.0, |row| row.y + row.height + CENTER_PADDING)
    }

    fn panel_height(&self) -> f32 {
        self.content_height()
            .min((self.size.1 - 2.0 * MARGIN).max(HEADER_HEIGHT))
    }

    fn max_scroll(&self) -> f32 {
        (self.content_height() - self.panel_height()).max(0.0)
    }

    fn panel_rect(&self, now: Instant) -> Rect {
        let (offset, _) = self.center_x.value_at(now);
        Rect::new(
            self.size.0 - MARGIN - CENTER_WIDTH + offset,
            MARGIN,
            CENTER_WIDTH,
            self.panel_height(),
        )
    }

    pub fn open(&mut self, now: Instant) {
        if self.open {
            return;
        }
        self.open = true;
        self.scroll = 0.0;
        self.focus = None;
        self.end_hold();
        self.rebuild(now);
        self.center_x
            .animate_at(now, self.reduce_motion, 0.0, Spring::STANDARD);
    }

    pub fn close(&mut self, now: Instant) {
        if !self.open {
            return;
        }
        self.open = false;
        self.pressed = None;
        self.center_x.animate_at(
            now,
            self.reduce_motion,
            hidden_offset(CENTER_WIDTH),
            Spring::STANDARD,
        );
    }

    pub fn toggle(&mut self, now: Instant) {
        if self.open {
            self.close(now);
        } else {
            self.open(now);
        }
    }

    pub fn animating(&self, now: Instant) -> bool {
        self.center_x.value_at(now).1
            || self
                .banners
                .iter()
                .any(|banner| banner.x.value_at(now).1 || banner.y.value_at(now).1)
    }

    pub fn settle(&mut self, now: Instant) {
        self.banners
            .retain(|banner| !(banner.leaving && !banner.x.value_at(now).1));
    }

    fn center_visible(&self, now: Instant) -> bool {
        self.open || self.center_x.value_at(now).1
    }

    fn banners_visible(&self) -> bool {
        !self.open
    }

    pub fn visible(&self, now: Instant) -> bool {
        self.center_visible(now) || (self.banners_visible() && !self.banners.is_empty())
    }

    pub fn wants_keyboard(&self) -> bool {
        self.open
    }

    pub fn input_region(&self) -> Option<Vec<Rect>> {
        if self.open {
            return None;
        }
        Some(
            self.banners
                .iter()
                .filter(|banner| !banner.leaving)
                .map(|banner| {
                    banner.card.bounds(Point::new(
                        self.size.0 - MARGIN - BANNER_WIDTH + banner.x.target(),
                        banner.y.target(),
                    ))
                })
                .collect(),
        )
    }

    fn banner_origin(&self, banner: &Banner, now: Instant) -> Point {
        Point::new(
            self.size.0 - MARGIN - BANNER_WIDTH + banner.x.value_at(now).0,
            banner.y.value_at(now).0,
        )
    }

    fn banner_at(&self, point: Point, now: Instant) -> Option<(usize, Part)> {
        if !self.banners_visible() {
            return None;
        }
        self.banners
            .iter()
            .enumerate()
            .filter(|(_, banner)| !banner.leaving)
            .find_map(|(index, banner)| {
                let origin = self.banner_origin(banner, now);
                let local = Point::new(point.x - origin.x, point.y - origin.y);
                banner.card.part_at(local, true).map(|part| (index, part))
            })
    }

    fn targets(&self) -> Vec<Target> {
        let mut targets = Vec::new();
        for (index, placed) in self.rows.iter().enumerate() {
            match &placed.row {
                Row::Title {
                    clear_all: Some(_), ..
                } => targets.push(Target::ClearAll),
                Row::Group { group, .. } => {
                    targets.push(Target::ShowLess(group.clone()));
                    targets.push(Target::ClearGroup(group.clone()));
                }
                Row::Card { .. } => targets.push(Target::Card(index, Part::Body)),
                Row::Title { .. } | Row::Empty(_) => {}
            }
        }
        targets
    }

    fn center_target_at(&self, point: Point, now: Instant) -> Option<Target> {
        let panel = self.panel_rect(now);
        if !contains(panel, point) {
            return None;
        }
        let local = Point::new(
            point.x - panel.x - CENTER_PADDING,
            point.y - panel.y + self.scroll,
        );
        for (index, placed) in self.rows.iter().enumerate() {
            let inside = Point::new(local.x, local.y - placed.y);
            match &placed.row {
                Row::Title {
                    clear_all: Some((_, rect)),
                    ..
                } if contains(*rect, inside) => return Some(Target::ClearAll),
                Row::Group {
                    group,
                    show_less: (_, rect),
                    clear,
                    ..
                } => {
                    if contains(*rect, inside) {
                        return Some(Target::ShowLess(group.clone()));
                    }
                    if contains(*clear, inside) {
                        return Some(Target::ClearGroup(group.clone()));
                    }
                }
                Row::Card { card, .. } => {
                    let hovered = self.pointer_row(now) == Some(index);
                    if let Some(part) = card.part_at(inside, hovered) {
                        return Some(Target::Card(index, part));
                    }
                }
                Row::Title { .. } | Row::Empty(_) => {}
            }
        }
        None
    }

    fn pointer_row(&self, now: Instant) -> Option<usize> {
        let point = self.pointer?;
        let panel = self.panel_rect(now);
        let local_y = point.y - panel.y + self.scroll;
        let local_x = point.x - panel.x - CENTER_PADDING;
        self.rows.iter().position(|placed| {
            matches!(placed.row, Row::Card { .. })
                && local_y >= placed.y - CLOSE_REACH
                && local_y < placed.y + placed.height
                && local_x >= -CLOSE_REACH
        })
    }

    fn run(&mut self, target: Target, now: Instant) {
        match target {
            Target::ClearAll => self.requests.push(Request::ClearAll),
            Target::ShowLess(group) => {
                self.expanded.remove(&group);
                self.rebuild_center(now);
            }
            Target::ClearGroup(group) => self.requests.push(Request::Clear(group)),
            Target::Card(index, part) => {
                let Some(PlacedRow {
                    row: Row::Card { card, collapsed },
                    ..
                }) = self.rows.get(index)
                else {
                    return;
                };
                let (id, group, collapsed) = (card.id, card.group.clone(), *collapsed);
                match part {
                    Part::Close if collapsed => self.requests.push(Request::Clear(group)),
                    Part::Close => self.requests.push(Request::Dismiss(id)),
                    Part::Body if collapsed => {
                        self.expanded.insert(group);
                        self.rebuild_center(now);
                    }
                    Part::Body => {
                        self.requests.push(Request::Activate {
                            id,
                            action: DEFAULT_ACTION.to_owned(),
                            group,
                        });
                        self.close(now);
                    }
                    Part::Action(action) => {
                        self.requests.push(Request::Activate { id, action, group });
                        self.close(now);
                    }
                }
            }
        }
    }

    fn end_hold(&mut self) {
        if self.holding {
            self.holding = false;
            self.requests.push(Request::HoldBanners(false));
        }
    }

    fn follow_hover(&mut self, now: Instant) {
        let over = self
            .pointer
            .is_some_and(|point| self.banner_at(point, now).is_some());
        let hold = over || self.drag.is_some();
        if hold != self.holding {
            self.holding = hold;
            self.requests.push(Request::HoldBanners(hold));
        }
    }

    pub fn handle(&mut self, event: &Event, now: Instant) -> bool {
        match event {
            Event::PointerEnter(pointer) | Event::PointerMove(pointer) => {
                self.pointer = Some(pointer.position);
                if let Some(drag) = &mut self.drag {
                    let dx = pointer.position.x - drag.start.x;
                    let elapsed = now.saturating_duration_since(drag.last.1).as_secs_f32();
                    if elapsed > 0.0 {
                        drag.velocity = (pointer.position.x - drag.last.0) / elapsed;
                    }
                    drag.last = (pointer.position.x, now);
                    drag.moved |= dx.abs() > DRAG_SLOP
                        || (pointer.position.y - drag.start.y).abs() > DRAG_SLOP;
                    if drag.moved {
                        let x = if dx >= 0.0 {
                            dx
                        } else {
                            -rubber_band(f64::from(-dx), 60.0) as f32
                        };
                        let group = drag.group.clone();
                        if let Some(banner) = self
                            .banners
                            .iter_mut()
                            .find(|banner| banner.card.group == group)
                        {
                            banner.x.jump_to(x);
                        }
                    }
                }
                if !self.open {
                    self.follow_hover(now);
                }
                true
            }
            Event::PointerLeave => {
                self.pointer = None;
                if !self.open && self.drag.is_none() {
                    self.follow_hover(now);
                }
                true
            }
            Event::PointerDown(pointer) if pointer.is_primary() => {
                self.pointer = Some(pointer.position);
                if self.open {
                    self.pressed = self.center_target_at(pointer.position, now);
                    if self.pressed.is_none() && !contains(self.panel_rect(now), pointer.position) {
                        self.close(now);
                    }
                    return true;
                }
                if let Some((index, part)) = self.banner_at(pointer.position, now) {
                    if part == Part::Body {
                        self.drag = Some(Drag {
                            group: self.banners[index].card.group.clone(),
                            start: pointer.position,
                            last: (pointer.position.x, now),
                            velocity: 0.0,
                            moved: false,
                        });
                    }
                    self.pressed = Some(Target::Card(index, part));
                }
                true
            }
            Event::PointerUp(pointer) if pointer.is_primary() => {
                let pressed = self.pressed.take();
                if self.open {
                    if pressed.is_some()
                        && pressed == self.center_target_at(pointer.position, now)
                        && let Some(target) = pressed
                    {
                        self.run(target, now);
                    }
                    return true;
                }
                if let Some(drag) = self.drag.take()
                    && drag.moved
                {
                    self.release(&drag, now);
                    self.follow_hover(now);
                    return true;
                }
                let released = self
                    .banner_at(pointer.position, now)
                    .map(|(index, part)| Target::Card(index, part));
                if let Some(Target::Card(index, part)) =
                    pressed.filter(|pressed| Some(pressed) == released.as_ref())
                {
                    self.banner_click(index, part);
                }
                true
            }
            Event::Scroll(scroll) if self.open => {
                let max = self.max_scroll();
                self.scroll = (self.scroll + scroll.delta.y).clamp(0.0, max);
                true
            }
            Event::Key { key, modifiers } if self.open => {
                self.key(*key, modifiers.shift, now);
                true
            }
            _ => false,
        }
    }

    fn banner_click(&mut self, index: usize, part: Part) {
        let Some(banner) = self.banners.get(index) else {
            return;
        };
        let (id, group) = (banner.card.id, banner.card.group.clone());
        self.requests.push(match part {
            Part::Close => Request::Dismiss(id),
            Part::Body => Request::Activate {
                id,
                action: DEFAULT_ACTION.to_owned(),
                group,
            },
            Part::Action(action) => Request::Activate { id, action, group },
        });
    }

    fn release(&mut self, drag: &Drag, now: Instant) {
        let Some(banner) = self
            .banners
            .iter_mut()
            .find(|banner| banner.card.group == drag.group)
        else {
            return;
        };
        let (x, _) = banner.x.value_at(now);
        let flung = drag.velocity > FLICK_SPEED
            || (x > BANNER_WIDTH * DISMISS_FRACTION && drag.velocity > -FLICK_SPEED);
        if flung {
            banner.leaving = true;
            banner.x.animate_at(
                now,
                self.reduce_motion,
                hidden_offset(BANNER_WIDTH),
                Spring::MOMENTUM,
            );
            self.requests.push(Request::HideBanner(banner.card.id));
        } else {
            banner
                .x
                .animate_at(now, self.reduce_motion, 0.0, Spring::MOMENTUM);
        }
    }

    fn key(&mut self, key: Key, shift: bool, now: Instant) {
        let targets = self.targets();
        let step = |focus: Option<usize>, forward: bool| -> Option<usize> {
            if targets.is_empty() {
                return None;
            }
            Some(match (focus, forward) {
                (None, true) => 0,
                (None, false) => targets.len() - 1,
                (Some(at), true) => (at + 1) % targets.len(),
                (Some(at), false) => (at + targets.len() - 1) % targets.len(),
            })
        };
        match key {
            Key::Escape => self.close(now),
            Key::Down => self.focus = step(self.focus, true),
            Key::Up => self.focus = step(self.focus, false),
            Key::Tab => self.focus = step(self.focus, !shift),
            Key::Enter | Key::Space => {
                if let Some(target) = self.focus.and_then(|at| targets.get(at)).cloned() {
                    self.run(target, now);
                }
            }
            Key::Backspace | Key::Delete => {
                if let Some(Target::Card(index, _)) =
                    self.focus.and_then(|at| targets.get(at)).cloned()
                {
                    self.run(Target::Card(index, Part::Close), now);
                }
            }
            _ => {}
        }
        self.reveal_focus();
    }

    fn reveal_focus(&mut self) {
        let row = match self.focus.and_then(|at| self.targets().get(at).cloned()) {
            Some(Target::Card(index, _)) => index,
            Some(Target::ShowLess(group) | Target::ClearGroup(group)) => {
                match self.rows.iter().position(
                    |placed| matches!(&placed.row, Row::Group { group: found, .. } if *found == group),
                ) {
                    Some(row) => row,
                    None => return,
                }
            }
            Some(Target::ClearAll) => 0,
            None => return,
        };
        let Some(placed) = self.rows.get(row) else {
            return;
        };
        let visible = self.panel_height();
        if placed.y < self.scroll {
            self.scroll = (placed.y - CENTER_PADDING).max(0.0);
        } else if placed.y + placed.height > self.scroll + visible {
            self.scroll =
                (placed.y + placed.height + CENTER_PADDING - visible).min(self.max_scroll());
        }
    }

    pub fn scene(&mut self, now: Instant) -> Scene {
        let (width, height) = self.size;
        let mut items = Vec::new();
        if self.banners_visible() {
            let hovered = self.pointer.and_then(|point| self.banner_at(point, now));
            for (index, banner) in self.banners.iter().enumerate() {
                let origin = self.banner_origin(banner, now);
                let part = hovered
                    .as_ref()
                    .filter(|(at, _)| *at == index)
                    .map(|(_, part)| part);
                let shift = banner.x.value_at(now).0.max(0.0);
                let fade = 1.0 - (shift / (BANNER_WIDTH + MARGIN)).clamp(0.0, 1.0) * 0.6;
                let mut card = Vec::new();
                banner
                    .card
                    .paint(origin, &self.material(), part, false, &mut card);
                items.push(Item::Opacity {
                    alpha: fade,
                    items: card,
                });
            }
        }
        if self.center_visible(now) {
            self.paint_center(now, &mut items);
        }
        Scene {
            width: width as u32,
            height: height as u32,
            background: Color::TRANSPARENT,
            items,
        }
    }

    fn paint_center(&self, now: Instant, items: &mut Vec<Item>) {
        let palette = self.material();
        let panel = self.panel_rect(now);
        let shape = Shape::rounded(panel, CENTER_RADIUS);
        items.push(Item::Shadow {
            shape,
            offset: Point::new(0.0, 8.0),
            blur_radius: 32.0,
            color: Color::hex(0).with_alpha(56),
        });
        items.push(Item::Fill {
            shape,
            paint: palette.sheet.into(),
        });
        items.push(Item::Stroke {
            shape,
            width: 0.5,
            paint: palette.hairline.into(),
        });
        let focused = self.focus.and_then(|at| self.targets().get(at).cloned());
        let hovered = self
            .pointer
            .and_then(|point| self.center_target_at(point, now));
        let hovered_row = self.pointer_row(now);
        let mut content = Vec::new();
        for (index, placed) in self.rows.iter().enumerate() {
            let origin = Point::new(0.0, placed.y);
            match &placed.row {
                Row::Title {
                    title,
                    clear_all,
                    status,
                } => {
                    content.push(Item::Text {
                        origin: Point::new(
                            4.0,
                            (origin.y + (HEADER_HEIGHT - title.height()) / 2.0).round(),
                        ),
                        paragraph: title.clone(),
                    });
                    if let Some((label, rect)) = clear_all {
                        let rect = Rect::new(rect.x, rect.y + origin.y, rect.width, rect.height);
                        pill(
                            &mut content,
                            &palette,
                            label,
                            rect,
                            hovered == Some(Target::ClearAll),
                            focused == Some(Target::ClearAll),
                        );
                    }
                    if let Some(status) = status {
                        content.push(Item::Text {
                            origin: Point::new(4.0, origin.y + HEADER_HEIGHT),
                            paragraph: status.clone(),
                        });
                    }
                }
                Row::Group {
                    group,
                    name,
                    show_less: (label, rect),
                    clear,
                } => {
                    content.push(Item::Text {
                        origin: Point::new(
                            4.0,
                            (origin.y + (GROUP_HEIGHT - name.height()) / 2.0).round(),
                        ),
                        paragraph: name.clone(),
                    });
                    let less = Rect::new(rect.x, rect.y + origin.y, rect.width, rect.height);
                    let less_target = Target::ShowLess(group.clone());
                    pill(
                        &mut content,
                        &palette,
                        label,
                        less,
                        hovered.as_ref() == Some(&less_target),
                        focused.as_ref() == Some(&less_target),
                    );
                    let clear_target = Target::ClearGroup(group.clone());
                    let clear = Rect::new(clear.x, clear.y + origin.y, clear.width, clear.height);
                    round_close(
                        &mut content,
                        &palette,
                        clear,
                        hovered.as_ref() == Some(&clear_target),
                        focused.as_ref() == Some(&clear_target),
                    );
                }
                Row::Card { card, .. } => {
                    let part = match &hovered {
                        Some(Target::Card(at, part)) if *at == index => Some(part.clone()),
                        _ if hovered_row == Some(index) => Some(Part::Body),
                        _ => None,
                    };
                    let is_focused = matches!(&focused, Some(Target::Card(at, _)) if *at == index);
                    card.paint(origin, &palette, part.as_ref(), is_focused, &mut content);
                }
                Row::Empty(paragraph) => content.push(Item::Text {
                    origin: Point::new(0.0, origin.y + HEADER_HEIGHT),
                    paragraph: paragraph.clone(),
                }),
            }
        }
        items.push(Item::Clip {
            shape,
            items: vec![Item::Translate {
                offset: Point::new(panel.x + CENTER_PADDING, panel.y - self.scroll),
                items: content,
            }],
        });
    }
}

fn pill(
    items: &mut Vec<Item>,
    palette: &Palette,
    label: &Paragraph,
    rect: Rect,
    hovered: bool,
    focused: bool,
) {
    items.push(Item::Fill {
        shape: Shape::rounded(rect, rect.height / 2.0),
        paint: palette.ink.with_alpha(if hovered { 34 } else { 16 }).into(),
    });
    if focused {
        items.push(Item::Stroke {
            shape: Shape::rounded(rect, rect.height / 2.0),
            width: 2.0,
            paint: palette.accent.into(),
        });
    }
    items.push(Item::Text {
        origin: Point::new(
            (rect.x + (rect.width - label.width()) / 2.0).round(),
            (rect.y + (rect.height - label.height()) / 2.0).round(),
        ),
        paragraph: label.clone(),
    });
}

fn round_close(items: &mut Vec<Item>, palette: &Palette, rect: Rect, hovered: bool, focused: bool) {
    let shape = Shape::rounded(rect, rect.height / 2.0);
    items.push(Item::Fill {
        shape,
        paint: palette.ink.with_alpha(if hovered { 34 } else { 16 }).into(),
    });
    if focused {
        items.push(Item::Stroke {
            shape,
            width: 2.0,
            paint: palette.accent.into(),
        });
    }
    let center = Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
    let arm = 4.0;
    for (dx, dy) in [(arm, arm), (arm, -arm)] {
        items.push(Item::Line {
            from: Point::new(center.x - dx, center.y - dy),
            to: Point::new(center.x + dx, center.y + dy),
            width: 1.5,
            paint: palette.ink.into(),
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [
            include_str!("lib.rs"),
            include_str!("card.rs"),
            include_str!("main.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
