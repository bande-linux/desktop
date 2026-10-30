use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use unbound_fs::{Action, Purpose, Task, same_volume};
use unbound_toolkit::ui::Pointer;
use unbound_toolkit::{Color, DragAction, Item, Point, Rect, Scene, Shape, TextAlign, TextStyle};
use unbound_xdg::path_from_file_uri;

use crate::browser::Location;
use crate::controls::Press;
use crate::sidebar::Row;
use crate::view::{FilesView, Hit, Request, contains, home};

const DRAG_THRESHOLD: f32 = 4.0;
const SPRING_DELAY: Duration = Duration::from_millis(900);
const ICON: f32 = 40.0;
const ICON_PIXELS: u32 = 64;
const STACK_OFFSET: f32 = 5.0;
const STACKED: usize = 3;
const BADGE: f32 = 20.0;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Target {
    Folder(PathBuf),
    Trash,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Spring {
    Go(Location),
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
    pub(crate) fn due(&self) -> Option<Instant> {
        self.spring.as_ref().map(|(_, since)| *since + SPRING_DELAY)
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

    pub(crate) fn start_drag(&mut self, _from: Point) -> bool {
        let paths = self.browser.selected_paths();
        if paths.is_empty() || self.browser.in_trash() {
            self.press = Some(Press::Files);
            return false;
        }
        self.press = Some(Press::Dragging);
        self.pending_rename = None;
        self.drag_sources = paths.clone();
        self.requests.push(Request::StartDrag(paths));
        true
    }

    pub fn drag_finished(&mut self) {
        self.press = None;
        self.drag_sources.clear();
        self.drop = None;
    }

    pub fn drag_icon(&mut self) -> Scene {
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
                paint: self.palette.accent_color.into(),
            });
            let label = self.text().paragraph(
                &count.to_string(),
                TextStyle::new(11.0, Color::hex(0xFFFFFF)).weight(TextStyle::SEMIBOLD),
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
        Scene {
            width: pixels,
            height: pixels,
            background: Color::hex(0).with_alpha(0),
            items: vec![Item::Opacity { alpha: 0.85, items }],
        }
    }

    fn table_frame(&self) -> Option<Rect> {
        self.browser.single().then(|| self.content_frame())
    }

    fn location_target(location: &Location) -> Target {
        match location {
            Location::Folder(folder) => Target::Folder(folder.clone()),
            Location::Trash => Target::Trash,
        }
    }

    fn drop_target(&self, at: Point) -> Option<(Target, Rect, Option<Spring>)> {
        if let Some(table) = self.table_frame().filter(|frame| contains(*frame, at)) {
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
                None => (Self::location_target(&column.location), table, None),
            });
        }
        let (rect, hit) = self
            .hits
            .iter()
            .rev()
            .find(|(rect, _)| contains(*rect, at))?;
        match hit {
            Hit::Place(index) => match self.sidebar.get(*index) {
                Some(Row::Place(place)) => Some((
                    Self::location_target(&place.location),
                    *rect,
                    Some(Spring::Go(place.location.clone())),
                )),
                _ => None,
            },
            Hit::Segment(path) if path.is_dir() => Some((
                Target::Folder(path.clone()),
                *rect,
                Some(Spring::Go(Location::Folder(path.clone()))),
            )),
            Hit::Row { column, row } => {
                let shown = self.browser.columns().get(*column)?;
                let entry = shown.entries.get(*row)?;
                if entry.is_folder() && entry.trashed.is_none() {
                    return Some((
                        Target::Folder(entry.path.clone()),
                        *rect,
                        Some(Spring::Open {
                            column: *column,
                            row: *row,
                        }),
                    ));
                }
                self.column_target(*column)
            }
            Hit::Column(column) => self.column_target(*column),
            _ => None,
        }
    }

    fn column_target(&self, column: usize) -> Option<(Target, Rect, Option<Spring>)> {
        let frame = self.hits.iter().find_map(|(rect, hit)| match hit {
            Hit::Column(index) if *index == column => Some(*rect),
            _ => None,
        })?;
        let location = &self.browser.columns().get(column)?.location;
        Some((Self::location_target(location), frame, None))
    }

    fn preferred(&self, target: &Target) -> Option<DragAction> {
        let Target::Folder(folder) = target else {
            return Some(DragAction::Move);
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
                .filter(|_| action.is_some())
                .map(|spring| (spring, since.unwrap_or_else(Instant::now))),
            target,
            highlight,
            action,
        });
        action
    }

    pub fn drag_entered(&mut self, at: Point, uris: bool) -> Option<DragAction> {
        if !uris {
            self.drop = None;
            return None;
        }
        self.drag_moved(at)
    }

    pub fn drag_left(&mut self) {
        self.drop = None;
    }

    pub(crate) fn spring(&mut self, now: Instant) {
        let Some(hover) = &mut self.drop else {
            return;
        };
        let Some((spring, since)) = hover.spring.take() else {
            return;
        };
        if since + SPRING_DELAY > now {
            hover.spring = Some((spring, since));
            return;
        }
        match spring {
            Spring::Go(location) => self.go(location),
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
        let Some((target, _, _)) = self.drop_target(at) else {
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
        let shape = Shape::rounded(hover.highlight.inset(1.0), 6.0);
        items.push(Item::Fill {
            shape,
            paint: self.palette.accent_color.with_alpha(40).into(),
        });
        items.push(Item::Stroke {
            shape,
            width: 2.0,
            paint: self.palette.accent_color.into(),
        });
    }
}
