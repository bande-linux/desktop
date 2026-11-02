mod content;
mod loader;
mod paint;
mod pdf;

use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Instant;

use accesskit::{Live, Node, Role};
use bande_fs::Entry;
use bande_l10n::Localizer;
use bande_toolkit::ui::{Animated, Cursor, Event, Key, Modifiers, Spring};
use bande_toolkit::{Image, Point, Rect};
use bande_xdg::{DesktopEntry, state_home};

use crate::content::Shown;
use crate::loader::Loader;

const ENGLISH: &str = include_str!("../locales/en-US/bande-quick-look.ftl");

static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

const SIZE_FILE: &str = "bande/quick-look-size";
const DEFAULT_SIZE: (f32, f32) = (720.0, 540.0);
const MIN_SIZE: (f32, f32) = (360.0, 260.0);
const ZOOM_STEP: f32 = 1.25;
const MAX_ZOOM: f32 = 8.0;

#[derive(Debug, Clone)]
pub struct Subject {
    pub entry: Entry,
    pub mime: String,
    pub kind: String,
    pub app: Option<DesktopEntry>,
    pub icon: Option<Image>,
    pub origin: Option<Rect>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Open {
        app: Box<DesktopEntry>,
        path: PathBuf,
    },
    Copy(PathBuf),
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Edge {
    fn cursor(self) -> Cursor {
        match self {
            Self::Left => Cursor::ResizeLeft,
            Self::Right => Cursor::ResizeRight,
            Self::Top => Cursor::ResizeTop,
            Self::Bottom => Cursor::ResizeBottom,
            Self::TopLeft => Cursor::ResizeTopLeft,
            Self::TopRight => Cursor::ResizeTopRight,
            Self::BottomLeft => Cursor::ResizeBottomLeft,
            Self::BottomRight => Cursor::ResizeBottomRight,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Hit {
    Close,
    FullScreen,
    Previous,
    Next,
    IndexSheet,
    Rotate,
    Copy,
    Open,
    Tile(usize),
    Thumbnail(usize),
    Resize(Edge),
    Panel,
}

struct Resizing {
    edge: Edge,
    from: Point,
    size: (f32, f32),
}

pub struct QuickLook {
    subjects: Vec<Subject>,
    at: usize,
    full_screen: bool,
    index_sheet: bool,
    loader: Loader,
    shown: Shown,
    rotation: u8,
    zoom: f32,
    pan: Point,
    scroll: Point,
    size: (f32, f32),
    presence: Animated,
    closing: bool,
    reduce_motion: bool,
    hits: Vec<(Rect, Hit)>,
    hover: Option<Hit>,
    pressed: Option<Hit>,
    resizing: Option<Resizing>,
    frame: Rect,
    area: Rect,
    symbols: paint::Symbols,
}

fn size_file() -> Option<PathBuf> {
    state_home().map(|state| state.join(SIZE_FILE))
}

fn remembered_size() -> (f32, f32) {
    size_file()
        .and_then(|file| std::fs::read_to_string(file).ok())
        .and_then(|text| {
            let (width, height) = text.trim().split_once(' ')?;
            Some((width.parse().ok()?, height.parse().ok()?))
        })
        .filter(|(width, height): &(f32, f32)| width.is_finite() && height.is_finite())
        .unwrap_or(DEFAULT_SIZE)
}

fn remember_size(size: (f32, f32)) {
    let Some(file) = size_file() else {
        return;
    };
    let text = format!("{} {}\n", size.0.round(), size.1.round());
    if let Err(error) = bande_xdg::replace_file(&file, &text) {
        tracing::warn!("cannot remember the Quick Look size: {error}");
    }
}

impl QuickLook {
    pub fn open(
        subjects: Vec<Subject>,
        at: usize,
        full_screen: bool,
        reduce_motion: bool,
        now: Instant,
    ) -> std::io::Result<Self> {
        let mut presence = Animated::new(0.0);
        presence.animate_at(now, reduce_motion, 1.0, Spring::STANDARD);
        let mut quick_look = Self {
            at: at.min(subjects.len().saturating_sub(1)),
            subjects,
            full_screen,
            index_sheet: false,
            loader: Loader::new()?,
            shown: Shown::Loading,
            rotation: 0,
            zoom: 1.0,
            pan: Point::default(),
            scroll: Point::default(),
            size: remembered_size(),
            presence,
            closing: false,
            reduce_motion,
            hits: Vec::new(),
            hover: None,
            pressed: None,
            resizing: None,
            frame: Rect::default(),
            area: Rect::default(),
            symbols: paint::Symbols::default(),
        };
        quick_look.load();
        Ok(quick_look)
    }

    fn load(&mut self) {
        self.shown = Shown::Loading;
        self.rotation = 0;
        self.zoom = 1.0;
        self.pan = Point::default();
        self.scroll = Point::default();
        if let Some(subject) = self.subjects.get(self.at) {
            self.loader
                .load(subject.entry.path.clone(), subject.mime.clone());
        }
    }

    pub fn subject(&self) -> Option<&Subject> {
        self.subjects.get(self.at)
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.subject().map(|subject| &subject.entry.path)
    }

    pub fn is_full_screen(&self) -> bool {
        self.full_screen
    }

    pub fn is_closing(&self) -> bool {
        self.closing
    }

    pub fn follows_selection(&self) -> bool {
        self.subjects.len() < 2 && !self.closing
    }

    pub fn position(&self) -> Option<(usize, usize)> {
        (self.subjects.len() > 1).then_some((self.at + 1, self.subjects.len()))
    }

    pub fn follow(&mut self, subject: Subject) {
        if self.path() == Some(&subject.entry.path) {
            if let Some(shown) = self.subjects.get_mut(self.at) {
                shown.origin = subject.origin;
            }
            return;
        }
        self.subjects = vec![subject];
        self.at = 0;
        self.index_sheet = false;
        self.load();
    }

    pub fn show(&mut self, subjects: Vec<Subject>, at: usize) {
        self.subjects = subjects;
        self.at = at.min(self.subjects.len().saturating_sub(1));
        self.index_sheet = false;
        self.closing = false;
        self.load();
    }

    pub fn step(&mut self, forward: bool) -> bool {
        let next = match forward {
            true => (self.at + 1).min(self.subjects.len().saturating_sub(1)),
            false => self.at.saturating_sub(1),
        };
        if self.subjects.len() < 2 {
            return false;
        }
        if next != self.at {
            self.at = next;
            self.load();
        }
        true
    }

    fn go_to(&mut self, index: usize) {
        self.index_sheet = false;
        if index != self.at && index < self.subjects.len() {
            self.at = index;
            self.load();
        }
    }

    pub fn toggle_full_screen(&mut self) {
        self.full_screen = !self.full_screen;
    }

    pub fn close(&mut self, now: Instant) {
        if self.closing {
            return;
        }
        self.closing = true;
        self.resizing = None;
        self.presence
            .animate_at(now, self.reduce_motion, 0.0, Spring::QUICK);
    }

    pub fn is_closed(&self, now: Instant) -> bool {
        self.closing && !self.presence.value_at(now).1
    }

    pub fn animating(&self, now: Instant) -> bool {
        self.presence.value_at(now).1
    }

    pub fn sources(&self) -> Vec<BorrowedFd<'_>> {
        let mut sources = vec![self.loader.fd()];
        if let Shown::Pdf(pdf) = &self.shown {
            sources.push(pdf.fd());
        }
        sources
    }

    pub fn source_ready(&mut self, index: usize) -> bool {
        match index {
            0 => {
                let mut changed = false;
                for content in self.loader.take() {
                    self.shown = Shown::from_content(content);
                    changed = true;
                }
                changed
            }
            _ => match &mut self.shown {
                Shown::Pdf(pdf) => pdf.take(),
                _ => false,
            },
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.shown.deadline()
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        self.shown.tick(now)
    }

    pub fn frame(&self) -> Rect {
        self.frame
    }

    pub fn contains(&self, point: Point) -> bool {
        !self.closing && self.hits.iter().any(|(rect, _)| rect.contains(point))
    }

    pub fn input_region(&self) -> Vec<Rect> {
        self.hits
            .iter()
            .filter(|(_, hit)| matches!(hit, Hit::Panel | Hit::Resize(_)))
            .map(|(rect, _)| *rect)
            .collect()
    }

    fn hit(&self, point: Point) -> Option<Hit> {
        self.hits
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(point))
            .map(|(_, hit)| *hit)
    }

    pub fn cursor(&self, point: Point) -> Option<Cursor> {
        match self.resizing.as_ref().map(|resizing| resizing.edge) {
            Some(edge) => Some(edge.cursor()),
            None => match self.hit(point)? {
                Hit::Resize(edge) => Some(edge.cursor()),
                _ => Some(Cursor::Default),
            },
        }
    }

    fn zoom_by(&mut self, bigger: bool) -> bool {
        if !self.shown.is_picture() {
            return false;
        }
        self.zoom = match bigger {
            true => (self.zoom * ZOOM_STEP).min(MAX_ZOOM),
            false => (self.zoom / ZOOM_STEP).max(1.0),
        };
        if self.zoom == 1.0 {
            self.pan = Point::default();
        }
        true
    }

    fn rotate(&mut self) -> bool {
        if !self.shown.is_picture() {
            return false;
        }
        self.rotation = (self.rotation + 3) % 4;
        self.pan = Point::default();
        true
    }

    fn scroll_by(&mut self, delta: Point) -> bool {
        if self.index_sheet {
            return false;
        }
        if self.shown.is_picture() {
            if self.zoom <= 1.0 {
                return false;
            }
            self.pan = Point::new(self.pan.x - delta.x, self.pan.y - delta.y);
            return true;
        }
        self.scroll = Point::new(self.scroll.x + delta.x, self.scroll.y + delta.y);
        true
    }

    pub fn key(&mut self, key: Key, modifiers: Modifiers, now: Instant) -> bool {
        let command = modifiers.control;
        let plain = !command && !modifiers.shift && !modifiers.alt && !modifiers.logo;
        let page = (self.area.height - 40.0).max(40.0);
        match key {
            Key::Escape => self.close(now),
            Key::Left | Key::Right if plain && self.subjects.len() > 1 => {
                return self.step(key == Key::Right);
            }
            Key::PageDown | Key::PageUp if plain => {
                let sign = if key == Key::PageDown { 1.0 } else { -1.0 };
                return self.scroll_by(Point::new(0.0, sign * page));
            }
            Key::Home if plain => {
                self.scroll = Point::default();
            }
            Key::End if plain => self.scroll = Point::new(0.0, f32::MAX),
            Key::Function(11) => self.toggle_full_screen(),
            Key::Enter if command && self.subjects.len() > 1 => {
                self.index_sheet = !self.index_sheet;
            }
            Key::Character('=' | '+') if command => return self.zoom_by(true),
            Key::Character('-') if command => return self.zoom_by(false),
            Key::Character('0') if command && self.shown.is_picture() => {
                self.zoom = 1.0;
                self.pan = Point::default();
            }
            Key::Character('l' | 'L') if command => return self.rotate(),
            _ => return false,
        }
        true
    }

    pub fn handle(&mut self, event: &Event, now: Instant) -> (bool, Option<Action>) {
        if self.closing {
            return (false, None);
        }
        match event {
            Event::PointerMove(pointer) | Event::PointerEnter(pointer) => {
                if let Some(resizing) = &self.resizing {
                    self.size = paint::resized(resizing, pointer.position, self.area_bounds());
                    return (true, None);
                }
                let hover = self.hit(pointer.position);
                let changed = hover != self.hover;
                self.hover = hover;
                (changed || hover.is_some(), None)
            }
            Event::PointerLeave => {
                let changed = self.hover.take().is_some();
                (changed, None)
            }
            Event::PointerDown(pointer) => {
                let Some(hit) = self.hit(pointer.position) else {
                    return (false, None);
                };
                if let Hit::Resize(edge) = hit {
                    self.resizing = Some(Resizing {
                        edge,
                        from: pointer.position,
                        size: (self.frame.width, self.frame.height),
                    });
                }
                self.pressed = Some(hit);
                (true, None)
            }
            Event::PointerUp(pointer) => {
                if self.resizing.take().is_some() {
                    remember_size(self.size);
                    return (true, None);
                }
                let pressed = self.pressed.take();
                let hit = self.hit(pointer.position);
                if pressed.is_none() || pressed != hit {
                    return (pressed.is_some(), None);
                }
                let action = match hit {
                    Some(Hit::Close) => Some(Action::Close),
                    Some(Hit::FullScreen) => {
                        self.toggle_full_screen();
                        None
                    }
                    Some(Hit::Previous) => {
                        self.step(false);
                        None
                    }
                    Some(Hit::Next) => {
                        self.step(true);
                        None
                    }
                    Some(Hit::IndexSheet) => {
                        self.index_sheet = !self.index_sheet;
                        None
                    }
                    Some(Hit::Rotate) => {
                        self.rotate();
                        None
                    }
                    Some(Hit::Copy) => self.path().cloned().map(Action::Copy),
                    Some(Hit::Open) => self.subject().and_then(|subject| {
                        Some(Action::Open {
                            app: Box::new(subject.app.clone()?),
                            path: subject.entry.path.clone(),
                        })
                    }),
                    Some(Hit::Tile(index)) => {
                        self.go_to(index);
                        None
                    }
                    Some(Hit::Thumbnail(page)) => {
                        if let Shown::Pdf(pdf) = &self.shown {
                            self.scroll.y = pdf.top_of(page);
                        }
                        None
                    }
                    _ => None,
                };
                if action == Some(Action::Close) {
                    self.close(now);
                }
                (true, action)
            }
            Event::Scroll(scroll) if self.contains(scroll.position) => {
                if scroll.modifiers.control && self.shown.is_picture() {
                    self.zoom_by(scroll.delta.y < 0.0);
                    return (true, None);
                }
                self.scroll_by(scroll.delta);
                (true, None)
            }
            _ => (false, None),
        }
    }

    fn area_bounds(&self) -> Rect {
        self.area
    }

    pub fn access(&self) -> Node {
        let mut node = Node::new(Role::Dialog);
        node.set_label(L10N.tr("quick-look"));
        node.set_live(Live::Polite);
        if let Some(subject) = self.subject() {
            let mut description = format!("{}, {}", subject.entry.name, subject.kind);
            if let Some((at, count)) = self.position() {
                description.push_str(", ");
                description.push_str(
                    &L10N.tr_args("position", [("at", at.into()), ("count", count.into())]),
                );
            }
            node.set_description(description);
        }
        node.set_bounds(accesskit::Rect {
            x0: f64::from(self.frame.x),
            y0: f64::from(self.frame.y),
            x1: f64::from(self.frame.x + self.frame.width),
            y1: f64::from(self.frame.y + self.frame.height),
        });
        node
    }
}
