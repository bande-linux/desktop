use accesskit::{Action, ActionRequest, Node, Role};
use bande_toolkit::ui::{Context, Event, Key, Size, Widget, WidgetId};
use bande_toolkit::{Item, Point, Rect, Shape};

use crate::widgets::{Text, Tone};

const HEIGHT: f32 = 200.0;
const PADDING: f32 = 24.0;
const RADIUS: f32 = 6.0;
const MENU_BAR: f32 = 5.0;
const ALIGN: i32 = 48;
const FOCUS_RING: f32 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Area {
    fn overlaps(&self, other: &Area) -> bool {
        self.x < other.x + other.width
            && other.x < self.x + self.width
            && self.y < other.y + other.height
            && other.y < self.y + self.height
    }

    fn at(self, (x, y): (i32, i32)) -> Self {
        Self { x, y, ..self }
    }
}

fn aligned(value: i32, starts: [i32; 3]) -> i32 {
    starts
        .into_iter()
        .filter(|start| (value - start).abs() < ALIGN)
        .min_by_key(|start| (value - start).abs())
        .unwrap_or(value)
}

pub fn snapped(areas: &[Area], index: usize, wanted: (i32, i32)) -> (i32, i32) {
    let moving = areas[index];
    let (w, h) = (moving.width, moving.height);
    let mut candidates = Vec::new();
    for (other, area) in areas.iter().enumerate() {
        if other == index {
            continue;
        }
        let y = aligned(
            wanted.1.clamp(area.y - h + 1, area.y + area.height - 1),
            [
                area.y,
                area.y + area.height - h,
                area.y + (area.height - h) / 2,
            ],
        );
        let x = aligned(
            wanted.0.clamp(area.x - w + 1, area.x + area.width - 1),
            [
                area.x,
                area.x + area.width - w,
                area.x + (area.width - w) / 2,
            ],
        );
        candidates.extend([
            (area.x + area.width, y),
            (area.x - w, y),
            (x, area.y + area.height),
            (x, area.y - h),
        ]);
    }
    candidates
        .into_iter()
        .filter(|&position| {
            let placed = moving.at(position);
            areas
                .iter()
                .enumerate()
                .all(|(other, area)| other == index || !placed.overlaps(area))
        })
        .min_by_key(|&(x, y)| {
            let (dx, dy) = (i64::from(x - wanted.0), i64::from(y - wanted.1));
            dx * dx + dy * dy
        })
        .unwrap_or((moving.x, moving.y))
}

pub struct Screen {
    pub label: String,
    pub area: Area,
    pub main: bool,
}

struct Drag {
    index: usize,
    grabbed: Point,
    at: Point,
}

pub struct Arrangement {
    id: WidgetId,
    label: String,
    screens: Vec<Screen>,
    labels: Vec<Text>,
    selected: usize,
    drag: Option<Drag>,
    on_move: Box<dyn FnMut(usize, (i32, i32))>,
    frame: Rect,
}

impl Arrangement {
    pub fn new(
        label: &str,
        screens: Vec<Screen>,
        on_move: impl FnMut(usize, (i32, i32)) + 'static,
    ) -> Self {
        let labels = screens
            .iter()
            .map(|screen| Text::new(&screen.label, Tone::Primary).caption())
            .collect();
        let selected = screens.iter().position(|screen| !screen.main).unwrap_or(0);
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            screens,
            labels,
            selected,
            drag: None,
            on_move: Box::new(on_move),
            frame: Rect::default(),
        }
    }

    fn areas(&self) -> Vec<Area> {
        self.screens.iter().map(|screen| screen.area).collect()
    }

    fn view(&self) -> (f32, Point, Point) {
        let areas = self.areas();
        let left = areas.iter().map(|area| area.x).min().unwrap_or(0);
        let top = areas.iter().map(|area| area.y).min().unwrap_or(0);
        let right = areas
            .iter()
            .map(|area| area.x + area.width)
            .max()
            .unwrap_or(1);
        let bottom = areas
            .iter()
            .map(|area| area.y + area.height)
            .max()
            .unwrap_or(1);
        let (width, height) = ((right - left).max(1) as f32, (bottom - top).max(1) as f32);
        let scale = ((self.frame.width - 2.0 * PADDING) / width)
            .min((self.frame.height - 2.0 * PADDING) / height)
            .max(0.0);
        let inset = Point::new(
            self.frame.x + (self.frame.width - width * scale) / 2.0,
            self.frame.y + (self.frame.height - height * scale) / 2.0,
        );
        (scale, Point::new(left as f32, top as f32), inset)
    }

    fn rect_of(&self, index: usize) -> Rect {
        let (scale, origin, inset) = self.view();
        let area = self.screens[index].area;
        let mut rect = Rect::new(
            inset.x + (area.x as f32 - origin.x) * scale,
            inset.y + (area.y as f32 - origin.y) * scale,
            area.width as f32 * scale,
            area.height as f32 * scale,
        );
        if let Some(drag) = self.drag.as_ref().filter(|drag| drag.index == index) {
            rect.x += drag.at.x - drag.grabbed.x;
            rect.y += drag.at.y - drag.grabbed.y;
        }
        rect
    }

    fn drop_at(&mut self, index: usize, wanted: (i32, i32)) {
        let position = snapped(&self.areas(), index, wanted);
        if position != (self.screens[index].area.x, self.screens[index].area.y) {
            self.screens[index].area = self.screens[index].area.at(position);
            (self.on_move)(index, position);
        }
    }

    fn release(&mut self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        let (scale, _, _) = self.view();
        if scale <= 0.0 {
            return;
        }
        let area = self.screens[drag.index].area;
        let wanted = (
            area.x + ((drag.at.x - drag.grabbed.x) / scale).round() as i32,
            area.y + ((drag.at.y - drag.grabbed.y) / scale).round() as i32,
        );
        self.drop_at(drag.index, wanted);
    }

    fn push(&mut self, (dx, dy): (i32, i32)) {
        let Some(main) = self.screens.iter().position(|screen| screen.main) else {
            return;
        };
        if self.selected == main {
            return;
        }
        let anchor = self.screens[main].area;
        let moving = self.screens[self.selected].area;
        let wanted = (
            anchor.x + (anchor.width - moving.width) / 2 + dx * (anchor.width + moving.width) / 2,
            anchor.y
                + (anchor.height - moving.height) / 2
                + dy * (anchor.height + moving.height) / 2,
        );
        self.drop_at(self.selected, wanted);
    }
}

impl Widget for Arrangement {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        for label in &mut self.labels {
            label.measure(cx);
        }
        Size::new(available.width, HEIGHT)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let mut order: Vec<usize> = (0..self.screens.len()).collect();
        order.sort_by_key(|index| self.drag.as_ref().is_some_and(|drag| drag.index == *index));
        for index in order {
            let rect = self.rect_of(index);
            let shape = Shape::rounded(rect, RADIUS);
            let selected = index == self.selected;
            let fill = if selected {
                cx.palette.accent.with_alpha(96)
            } else {
                cx.palette.ink2.with_alpha(56)
            };
            items.push(Item::Fill {
                shape,
                paint: fill.into(),
            });
            items.push(Item::Stroke {
                shape: Shape::rounded(rect.inset(0.5), RADIUS - 0.5),
                width: 1.0,
                paint: if selected {
                    cx.palette.accent.into()
                } else {
                    cx.palette.hairline.into()
                },
            });
            if self.screens[index].main {
                items.push(Item::Fill {
                    shape: Shape::rounded(
                        Rect::new(rect.x + 1.0, rect.y + 1.0, rect.width - 2.0, MENU_BAR),
                        2.0,
                    ),
                    paint: cx.palette.ink.with_alpha(160).into(),
                });
            }
            if selected && cx.is_focused(self.id) {
                items.push(Item::Stroke {
                    shape: Shape::rounded(rect.inset(-FOCUS_RING), RADIUS + FOCUS_RING),
                    width: FOCUS_RING,
                    paint: cx.palette.accent.with_alpha(128).into(),
                });
            }
            let label = &self.labels[index];
            let size = label.size();
            if size.width <= rect.width - 8.0 {
                label.paint(
                    cx,
                    Point::new(
                        rect.x + (rect.width - size.width) / 2.0,
                        rect.y + (rect.height - size.height) / 2.0,
                    ),
                    items,
                );
            }
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Group);
        node.set_label(self.label.clone());
        if let Some(screen) = self.screens.get(self.selected) {
            node.set_value(screen.label.clone());
        }
        node.add_action(Action::Increment);
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => {
                let Some(index) = (0..self.screens.len())
                    .rev()
                    .find(|index| self.rect_of(*index).contains(pointer.position))
                else {
                    return false;
                };
                self.selected = index;
                self.drag = Some(Drag {
                    index,
                    grabbed: pointer.position,
                    at: pointer.position,
                });
            }
            Event::PointerMove(pointer) => {
                let Some(drag) = self.drag.as_mut() else {
                    return false;
                };
                drag.at = pointer.position;
            }
            Event::PointerUp(_) => {
                if self.drag.is_none() {
                    return false;
                }
                self.release();
            }
            Event::Key {
                key: Key::Space, ..
            } => {
                self.selected = (self.selected + 1) % self.screens.len().max(1);
            }
            Event::Key { key: Key::Left, .. } => self.push((-1, 0)),
            Event::Key {
                key: Key::Right, ..
            } => self.push((1, 0)),
            Event::Key { key: Key::Up, .. } => self.push((0, -1)),
            Event::Key { key: Key::Down, .. } => self.push((0, 1)),
            _ => return false,
        }
        true
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        if request.action != Action::Increment {
            return false;
        }
        self.selected = (self.selected + 1) % self.screens.len().max(1);
        true
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        cx.keyboard_navigation && self.screens.len() > 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(x: i32, y: i32, width: i32, height: i32) -> Area {
        Area {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn a_display_dropped_beside_another_snaps_to_its_edge_and_aligns_with_its_top() {
        let areas = [area(0, 0, 1512, 982), area(2000, 30, 1920, 1080)];

        assert_eq!(snapped(&areas, 1, (1540, 20)), (1512, 0));
        assert_eq!(snapped(&areas, 1, (-1900, 400)), (-1920, 400));
        assert_eq!(snapped(&areas, 1, (100, 1000)), (100, 982));
    }

    #[test]
    fn a_display_is_never_dropped_onto_another() {
        let areas = [
            area(0, 0, 1000, 600),
            area(1000, 0, 1000, 600),
            area(3000, 0, 500, 400),
        ];

        let (x, y) = snapped(&areas, 2, (1200, 100));
        let placed = area(x, y, 500, 400);
        assert!(!placed.overlaps(&areas[0]) && !placed.overlaps(&areas[1]));
        assert!(x == 2000 || y == 600 || y == -400 || x == -500);
    }
}
