use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bande_fs::{Action, Purpose, Task, same_volume};
use bande_toolkit::ui::Pointer;
use bande_toolkit::{
    Color, DragAction, DragIcon, Item, Point, Radius, Rect, Scene, Shape, TextAlign, TextStyle,
};
use bande_xdg::path_from_file_uri;

use crate::browser::Location;
use crate::controls::Press;
use crate::sidebar::{PlaceKind, Row, favourites};
use crate::toolbar::TOOLBAR_HEIGHT;
use crate::view::{FilesView, Hit, Request, SIDEBAR_WIDTH, home};
use crate::views::ViewMode;

const DRAG_THRESHOLD: f32 = 4.0;
const GAP_REACH: f32 = 6.0;
const EMPTY_FAVOURITES_Y: f32 = 40.0;
const ICON: f32 = 40.0;
const ICON_PIXELS: u32 = 64;
const STACK_OFFSET: f32 = 5.0;
const STACKED: usize = 3;
const BADGE: f32 = 20.0;
const TABLE_ICON_CENTRE: f32 = 6.0 + 8.0;
const COLUMN_ICON_CENTRE: f32 = 6.0 + 6.0 + 8.0;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Target {
    Folder(PathBuf),
    Trash,
    Favourite(Option<Location>),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Spring {
    Go(Location),
    Tab(usize),
    Open { column: usize, row: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Hover {
    target: Target,
    highlight: Rect,
    spring: Option<(Spring, Instant)>,
    action: Option<DragAction>,
}

impl Hover {
    pub(crate) fn due(&self, delay: Duration) -> Option<Instant> {
        self.spring.as_ref().map(|(_, since)| *since + delay)
    }
}

fn distance(a: Point, b: Point) -> f32 {
    (a.x - b.x).hypot(a.y - b.y)
}

fn inside(folder: &Path, sources: &[PathBuf]) -> bool {
    sources.iter().any(|source| folder.starts_with(source))
}

impl FilesView {
    pub(crate) fn maybe_drag(&mut self, from: Point, pointer: &Pointer) -> bool {
        if distance(from, pointer.position) < DRAG_THRESHOLD {
            return false;
        }
        self.start_drag(from)
    }

    pub(crate) fn start_drag(&mut self, from: Point) -> bool {
        let paths = self.browser.selected_paths();
        if paths.is_empty() || self.browser.in_trash() {
            self.press = Some(Press::Files);
            return false;
        }
        self.press = Some(Press::Dragging);
        self.pending_rename = None;
        let grab = self
            .grabbed_icon(from)
            .map_or(Point::new(0.0, 0.0), |centre| {
                Point::new(from.x - centre.x, from.y - centre.y)
            });
        self.requests.push(Request::StartDrag { paths, grab });
        true
    }

    fn grabbed_icon(&mut self, at: Point) -> Option<Point> {
        if self.table_frame().is_some_and(|frame| frame.contains(at)) {
            let row = self.table_row_at(at)?;
            let name = self.shown_table()?.cell_bounds(row, 0)?;
            return Some(Point::new(
                name.x + TABLE_ICON_CENTRE,
                name.y + name.height / 2.0,
            ));
        }
        let row = self.hits.iter().find_map(|(rect, hit)| match hit {
            Hit::Row { row, .. } if rect.contains(at) => Some(*row),
            _ => None,
        });
        if self.mode == ViewMode::Icons {
            return row.map(|row| self.grid_icon_centre(row));
        }
        if self.mode == ViewMode::Gallery {
            return self.hits.iter().find_map(|(rect, hit)| match hit {
                Hit::Row { .. } if rect.contains(at) => Some(Point::new(
                    rect.x + rect.width / 2.0,
                    rect.y + rect.height / 2.0,
                )),
                _ => None,
            });
        }
        self.hits.iter().find_map(|(rect, hit)| match hit {
            Hit::Row { .. } if rect.contains(at) => Some(Point::new(
                rect.x + COLUMN_ICON_CENTRE,
                rect.y + rect.height / 2.0,
            )),
            _ => None,
        })
    }

    pub(crate) fn reorder_moved(&mut self, index: usize, from: Point, pointer: &Pointer) -> bool {
        let location = match self.sidebar.get(index) {
            Some(Row::Place(place))
                if index <= favourites(&self.sidebar).len() && place.kind != PlaceKind::Volume =>
            {
                place.location.clone()
            }
            _ => return false,
        };
        if distance(from, pointer.position) < DRAG_THRESHOLD && self.drop.is_none() {
            return false;
        }
        self.drop = self
            .sidebar_gap(pointer.position)
            .map(|(target, highlight)| Hover {
                target,
                highlight,
                spring: None,
                action: Some(DragAction::Copy),
            });
        self.reordering = Some(location);
        true
    }

    pub(crate) fn reorder_finished(&mut self) -> bool {
        let Some(location) = self.reordering.take() else {
            return false;
        };
        if let Some(Hover {
            target: Target::Favourite(before),
            ..
        }) = self.drop.take()
        {
            self.place_in_sidebar(vec![location], before);
        }
        true
    }

    pub fn drag_finished(&mut self) {
        self.press = None;
        self.drag_sources.clear();
        self.drop = None;
    }

    pub fn drag_icon(&mut self, grab: Point) -> DragIcon {
        let entries = self
            .browser
            .selection()
            .into_iter()
            .take(STACKED)
            .cloned()
            .collect::<Vec<_>>();
        let count = self.browser.selection().len();
        let stack = STACK_OFFSET * entries.len().saturating_sub(1) as f32;
        let size = ICON + stack + BADGE / 2.0;
        let mut items = Vec::new();
        for (index, entry) in entries.iter().enumerate().rev() {
            if let Some(image) = self.entry_icon(entry, ICON_PIXELS) {
                let offset = STACK_OFFSET * index as f32;
                items.push(Item::Image {
                    rect: Rect::new(offset, BADGE / 2.0 + stack - offset, ICON, ICON),
                    image,
                });
            }
        }
        if count > 1 {
            let badge = Rect::new(size - BADGE, 0.0, BADGE, BADGE);
            items.push(Item::Fill {
                shape: Shape::rounded(badge, BADGE / 2.0),
                paint: self.palette.danger.into(),
            });
            let on_danger = self.palette.on_danger;
            let label = self.text().paragraph(
                &count.to_string(),
                TextStyle::new(11.0, on_danger).weight(TextStyle::BOLD),
                Some(BADGE),
                TextAlign::Center,
            );
            let top = badge.y + ((BADGE - label.height()) / 2.0).round();
            items.push(Item::Text {
                origin: Point::new(badge.x, top),
                paragraph: label,
            });
        }
        let pixels = size.ceil() as u32;
        let reach = ICON / 2.0;
        DragIcon {
            scene: Scene {
                width: pixels,
                height: pixels,
                background: Color::TRANSPARENT,
                items: vec![Item::Opacity { alpha: 0.85, items }],
            },
            hotspot: Point::new(
                reach + grab.x.clamp(-reach, reach),
                BADGE / 2.0 + stack + reach + grab.y.clamp(-reach, reach),
            ),
        }
    }

    fn table_frame(&self) -> Option<Rect> {
        (self.mode == ViewMode::List).then(|| self.content_frame())
    }

    fn location_target(location: &Location) -> Option<Target> {
        match location {
            Location::Folder(folder) => Some(Target::Folder(folder.clone())),
            Location::Trash => Some(Target::Trash),
            Location::Recents | Location::Search(_) => None,
        }
    }

    fn sidebar_gap(&self, at: Point) -> Option<(Target, Rect)> {
        if at.x >= SIDEBAR_WIDTH {
            return None;
        }
        let favourites = self
            .sidebar
            .iter()
            .enumerate()
            .skip(1)
            .take_while(|(_, row)| matches!(row, Row::Place(_)))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let rows = self
            .hits
            .iter()
            .filter_map(|(rect, hit)| match hit {
                Hit::Place(index) if favourites.contains(index) => Some((*rect, *index)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let location_at = |index: usize| match self.sidebar.get(index) {
            Some(Row::Place(place)) => Some(place.location.clone()),
            _ => None,
        };
        let line = |rect: Rect, y: f32| Rect::new(rect.x, y - 1.0, rect.width, 2.0);
        if rows.is_empty() {
            let first = TOOLBAR_HEIGHT + EMPTY_FAVOURITES_Y;
            let area = Rect::new(8.0, first, SIDEBAR_WIDTH - 16.0, 2.0);
            return (at.y >= first - 2.0 * GAP_REACH && at.y <= first + 2.0 * GAP_REACH)
                .then_some((Target::Favourite(None), area));
        }
        for (position, (rect, index)) in rows.iter().enumerate() {
            if (at.y - rect.y).abs() <= GAP_REACH {
                return Some((Target::Favourite(location_at(*index)), line(*rect, rect.y)));
            }
            let bottom = rect.y + rect.height;
            if position + 1 == rows.len()
                && at.y >= bottom - GAP_REACH
                && at.y <= bottom + 2.0 * GAP_REACH
            {
                return Some((Target::Favourite(None), line(*rect, bottom)));
            }
        }
        None
    }

    fn drop_target(&self, at: Point) -> Option<(Target, Rect, Option<Spring>)> {
        let folders =
            !self.drag_sources.is_empty() && self.drag_sources.iter().all(|source| source.is_dir());
        if folders && let Some((target, line)) = self.sidebar_gap(at) {
            return Some((target, line, None));
        }
        if let Some(table) = self.table_frame().filter(|frame| frame.contains(at)) {
            let column = &self.browser.columns()[0];
            let folder = self
                .table_row_at(at)
                .and_then(|row| Some((row, column.entries.get(row)?)))
                .filter(|(_, entry)| entry.is_folder() && entry.trashed.is_none());
            return Some(match folder {
                Some((row, entry)) => (
                    Target::Folder(entry.path.clone()),
                    self.table_row_bounds(row).unwrap_or(table),
                    Some(Spring::Go(Location::Folder(entry.path.clone()))),
                ),
                None => (Self::location_target(&column.location)?, table, None),
            });
        }
        let (rect, hit) = self.hits.iter().rev().find(|(rect, _)| rect.contains(at))?;
        match hit {
            Hit::Place(index) => match self.sidebar.get(*index) {
                Some(Row::Place(place)) => Some((
                    Self::location_target(&place.location)?,
                    *rect,
                    Some(Spring::Go(place.location.clone())),
                )),
                _ => None,
            },
            Hit::Tab(index) | Hit::CloseTab(index) => {
                let location = self.tab_location(*index)?;
                Some((
                    Self::location_target(&location)?,
                    *rect,
                    Some(Spring::Tab(*index)),
                ))
            }
            Hit::Segment(path) if path.is_dir() => Some((
                Target::Folder(path.clone()),
                *rect,
                Some(Spring::Go(Location::Folder(path.clone()))),
            )),
            Hit::Row { column, row } => {
                let shown = self.browser.columns().get(*column)?;
                let entry = shown.entries.get(*row)?;
                if entry.is_folder() && entry.trashed.is_none() {
                    let spring = match self.mode {
                        ViewMode::Columns => Spring::Open {
                            column: *column,
                            row: *row,
                        },
                        _ => Spring::Go(Location::Folder(entry.path.clone())),
                    };
                    return Some((Target::Folder(entry.path.clone()), *rect, Some(spring)));
                }
                self.column_target(*column)
            }
            Hit::Column(column) => self.column_target(*column),
            _ => None,
        }
    }

    fn column_target(&self, column: usize) -> Option<(Target, Rect, Option<Spring>)> {
        let frame = self.hits.iter().rev().find_map(|(rect, hit)| match hit {
            Hit::Column(index) if *index == column => Some(*rect),
            _ => None,
        })?;
        let location = &self.browser.columns().get(column)?.location;
        Some((Self::location_target(location)?, frame, None))
    }

    fn preferred(&self, target: &Target) -> Option<DragAction> {
        let folder = match target {
            Target::Folder(folder) => folder,
            Target::Trash => return Some(DragAction::Move),
            Target::Favourite(_) => return Some(DragAction::Copy),
        };
        let sources = &self.drag_sources;
        if inside(folder, sources) {
            return None;
        }
        let reference = sources.first().cloned().unwrap_or_else(home);
        let moves_nowhere = !sources.is_empty()
            && sources
                .iter()
                .all(|source| source.parent() == Some(folder.as_path()));
        match (same_volume(&reference, folder), moves_nowhere) {
            (true, true) => None,
            (true, false) => Some(DragAction::Move),
            (false, _) => Some(DragAction::Copy),
        }
    }

    pub fn drag_moved(&mut self, at: Point) -> Option<DragAction> {
        let Some((target, highlight, spring)) = self.drop_target(at) else {
            self.drop = None;
            return None;
        };
        let action = self.preferred(&target);
        let since = match &self.drop {
            Some(hover) if hover.target == target => hover.spring.as_ref().map(|(_, since)| *since),
            _ => None,
        };
        self.drop = Some(Hover {
            spring: spring
                .filter(|_| action.is_some() && self.spring_delay.is_some())
                .map(|spring| (spring, since.unwrap_or_else(Instant::now))),
            target,
            highlight,
            action,
        });
        action
    }

    pub fn drag_entered(&mut self, at: Point, uris: &[String]) -> Option<DragAction> {
        self.drag_sources = uris
            .iter()
            .filter_map(|uri| path_from_file_uri(uri))
            .collect();
        if self.drag_sources.is_empty() {
            self.drop = None;
            return None;
        }
        self.drag_moved(at)
    }

    pub fn drag_left(&mut self) {
        self.drop = None;
        self.drag_sources.clear();
    }

    pub(crate) fn spring(&mut self, now: Instant) {
        let Some(hover) = &mut self.drop else {
            return;
        };
        let Some((spring, since)) = hover.spring.take() else {
            return;
        };
        if self.spring_delay.is_none_or(|delay| since + delay > now) {
            hover.spring = Some((spring, since));
            return;
        }
        match spring {
            Spring::Go(location) => self.go(location),
            Spring::Tab(index) => self.select_tab(index),
            Spring::Open { column, row } => {
                self.browser.select(column, row);
                self.selection_changed();
            }
        }
    }

    pub fn dropped(&mut self, at: Point, uris: &[String], action: Option<DragAction>) -> bool {
        self.drop = None;
        let sources = uris
            .iter()
            .filter_map(|uri| path_from_file_uri(uri))
            .collect::<Vec<_>>();
        self.drag_sources = sources.clone();
        let target = self.drop_target(at);
        self.drag_sources.clear();
        let Some((target, _, _)) = target else {
            return false;
        };
        if sources.is_empty() {
            return false;
        }
        let destination = match target {
            Target::Trash => {
                self.start(Task::Trash { sources }, Purpose::Do(Action::Trash));
                return true;
            }
            Target::Favourite(before) => {
                self.place_in_sidebar(sources.into_iter().map(Location::Folder).collect(), before);
                return true;
            }
            Target::Folder(folder) => folder,
        };
        if inside(&destination, &sources) {
            return false;
        }
        let copy = action == Some(DragAction::Copy);
        let here = sources
            .iter()
            .all(|source| source.parent() == Some(destination.as_path()));
        match (copy, here) {
            (true, true) => self.start(Task::Duplicate { sources }, Purpose::Do(Action::Duplicate)),
            (false, true) => return false,
            (true, false) => self.start(
                Task::Copy {
                    sources,
                    destination,
                },
                Purpose::Do(Action::Copy),
            ),
            (false, false) => self.start(
                Task::Move {
                    sources,
                    destination,
                },
                Purpose::Do(Action::Move),
            ),
        }
        true
    }

    pub(crate) fn paint_drop(&self, items: &mut Vec<Item>) {
        let Some(hover) = self.drop.as_ref().filter(|hover| hover.action.is_some()) else {
            return;
        };
        if matches!(hover.target, Target::Favourite(_)) {
            items.push(Item::Fill {
                shape: Shape::rounded(hover.highlight, 1.0),
                paint: self.palette.accent.into(),
            });
            return;
        }
        let shape = Shape::rounded(hover.highlight.inset(1.0), Radius::S);
        items.push(Item::Fill {
            shape,
            paint: self.palette.accent_soft.into(),
        });
        items.push(Item::Stroke {
            shape,
            width: 2.0,
            paint: self.palette.accent.into(),
        });
    }
}
