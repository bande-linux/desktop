use std::cell::RefCell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, Node, Role};
use bande_toolkit::ui::{
    BUTTON_HEIGHT, Button, Context, Event, Key, Parts, Size, Widget, WidgetId, focus_ring_for,
};
use bande_toolkit::{Image, Item, Paragraph, Point, Rect, Shape, TextAlign, TextStyle};

use crate::L10N;
use crate::service::Choice;

const PADDING: f32 = 20.0;
const TITLE_GAP: f32 = 16.0;
const BUTTONS_GAP: f32 = 18.0;
const BUTTON_SPACING: f32 = 8.0;
const BUTTON_MIN_WIDTH: f32 = 76.0;
const TILE_WIDTH: f32 = 176.0;
const THUMBNAIL_HEIGHT: f32 = 110.0;
const LABEL_GAP: f32 = 6.0;
const LABEL_HEIGHT: f32 = 16.0;
const TILE_GAP: f32 = 12.0;
const TILE_RADIUS: f32 = 8.0;
const RING: f32 = 3.0;
const MOST_COLUMNS: usize = 4;
const LABEL_SIZE: f32 = 12.0;
const ELLIPSIS: char = '…';

pub type Outcome = Rc<RefCell<Option<Option<Vec<String>>>>>;

struct Tile {
    label: String,
    image: Option<Image>,
    paragraph: Option<Paragraph>,
    frame: Rect,
}

struct Grid {
    id: WidgetId,
    tiles: Vec<Tile>,
    selected: Rc<RefCell<Vec<usize>>>,
    current: usize,
    multiple: bool,
    columns: usize,
    share: Rc<dyn Fn()>,
    frame: Rect,
}

fn columns_for(count: usize) -> usize {
    match count {
        0..=1 => 1,
        2..=4 => 2,
        5..=9 => 3,
        _ => MOST_COLUMNS,
    }
}

fn fitted(cx: &mut Context, text: &str, style: TextStyle, width: f32) -> Paragraph {
    let whole = cx.text.paragraph(text, style, None, TextAlign::Center);
    if whole.width() <= width {
        return whole;
    }
    let characters: Vec<char> = text.chars().collect();
    let (mut low, mut high) = (0, characters.len());
    while low < high {
        let middle = (low + high).div_ceil(2);
        let candidate: String = characters[..middle].iter().chain([&ELLIPSIS]).collect();
        if cx
            .text
            .paragraph(&candidate, style, None, TextAlign::Center)
            .width()
            <= width
        {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let shortened: String = characters[..low].iter().chain([&ELLIPSIS]).collect();
    cx.text
        .paragraph(&shortened, style, None, TextAlign::Center)
}

fn aspect_fit(image: &Image, area: Rect) -> Rect {
    let scale = (area.width / image.width() as f32).min(area.height / image.height() as f32);
    let (width, height) = (image.width() as f32 * scale, image.height() as f32 * scale);
    Rect::new(
        area.x + (area.width - width) / 2.0,
        area.y + (area.height - height) / 2.0,
        width,
        height,
    )
}

impl Grid {
    fn rows(&self) -> usize {
        self.tiles.len().div_ceil(self.columns.max(1))
    }

    fn select(&mut self, index: usize, toggle: bool) {
        self.current = index;
        let mut selected = self.selected.borrow_mut();
        if self.multiple && toggle {
            match selected.iter().position(|chosen| *chosen == index) {
                Some(at) => {
                    selected.remove(at);
                }
                None => selected.push(index),
            }
        } else {
            *selected = vec![index];
        }
    }

    fn tile_at(&self, point: Point) -> Option<usize> {
        self.tiles.iter().position(|tile| {
            point.x >= tile.frame.x
                && point.y >= tile.frame.y
                && point.x < tile.frame.x + tile.frame.width
                && point.y < tile.frame.y + tile.frame.height
        })
    }

    fn step(&mut self, key: Key) -> bool {
        let count = self.tiles.len();
        if count == 0 {
            return false;
        }
        let next = match key {
            Key::Left => self.current.checked_sub(1),
            Key::Right => Some(self.current + 1).filter(|next| *next < count),
            Key::Up => self.current.checked_sub(self.columns),
            Key::Down => Some(self.current + self.columns).filter(|next| *next < count),
            Key::Home => Some(0),
            Key::End => Some(count - 1),
            _ => None,
        };
        match next {
            Some(next) if self.multiple => {
                self.current = next;
                true
            }
            Some(next) => {
                self.select(next, false);
                true
            }
            None => matches!(key, Key::Left | Key::Right | Key::Up | Key::Down),
        }
    }
}

impl Widget for Grid {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        let style = TextStyle::new(LABEL_SIZE, cx.palette.ink);
        for tile in &mut self.tiles {
            tile.paragraph = Some(fitted(cx, &tile.label, style, TILE_WIDTH));
        }
        let columns = self.columns as f32;
        let rows = self.rows() as f32;
        Size::new(
            columns * TILE_WIDTH + (columns - 1.0).max(0.0) * TILE_GAP,
            rows * (THUMBNAIL_HEIGHT + LABEL_GAP + LABEL_HEIGHT) + (rows - 1.0).max(0.0) * TILE_GAP,
        )
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let columns = self.columns.max(1);
        for (index, tile) in self.tiles.iter_mut().enumerate() {
            let (column, row) = ((index % columns) as f32, (index / columns) as f32);
            tile.frame = Rect::new(
                frame.x + column * (TILE_WIDTH + TILE_GAP),
                frame.y + row * (THUMBNAIL_HEIGHT + LABEL_GAP + LABEL_HEIGHT + TILE_GAP),
                TILE_WIDTH,
                THUMBNAIL_HEIGHT + LABEL_GAP + LABEL_HEIGHT,
            );
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let selected = self.selected.borrow();
        let focused = cx.is_focused(self.id);
        for (index, tile) in self.tiles.iter().enumerate() {
            let well = Rect::new(tile.frame.x, tile.frame.y, TILE_WIDTH, THUMBNAIL_HEIGHT);
            items.push(Item::Fill {
                shape: Shape::rounded(well, TILE_RADIUS),
                paint: cx.palette.sheet.into(),
            });
            if let Some(image) = &tile.image {
                let fitted = aspect_fit(image, well.inset(8.0));
                items.push(Item::Clip {
                    shape: Shape::rounded(fitted, 3.0),
                    items: vec![Item::Image {
                        rect: fitted,
                        image: image.clone(),
                    }],
                });
            }
            if selected.contains(&index) {
                items.push(Item::Stroke {
                    shape: Shape::rounded(well.inset(-RING / 2.0), TILE_RADIUS + RING / 2.0),
                    width: RING,
                    paint: cx.palette.accent.into(),
                });
            }
            if focused && index == self.current && cx.keyboard_navigation {
                focus_ring_for(
                    &cx.palette,
                    Shape::rounded(well.inset(-RING), TILE_RADIUS + RING),
                    items,
                );
            }
            if let Some(paragraph) = &tile.paragraph {
                items.push(Item::Text {
                    origin: Point::new(
                        tile.frame.x + (TILE_WIDTH - paragraph.width()) / 2.0,
                        tile.frame.y + THUMBNAIL_HEIGHT + LABEL_GAP,
                    ),
                    paragraph: paragraph.clone(),
                });
            }
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::ListBox);
        node.set_label(L10N.tr("share-sources"));
        if self.multiple {
            node.set_multiselectable();
        }
        node
    }

    fn parts(&self, cx: &Context) -> Parts {
        let selected = self.selected.borrow();
        let nodes = self
            .tiles
            .iter()
            .enumerate()
            .map(|(index, tile)| {
                let mut node = Node::new(Role::ListBoxOption);
                node.set_label(tile.label.clone());
                node.set_selected(selected.contains(&index));
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
                node.set_bounds(accesskit::Rect {
                    x0: f64::from(tile.frame.x),
                    y0: f64::from(tile.frame.y),
                    x1: f64::from(tile.frame.x + tile.frame.width),
                    y1: f64::from(tile.frame.y + tile.frame.height),
                });
                (self.id.part(index as u64), node)
            })
            .collect::<Vec<_>>();
        Parts {
            children: nodes.iter().map(|(id, _)| *id).collect(),
            nodes,
            focus: cx
                .is_focused(self.id)
                .then(|| self.id.part(self.current as u64)),
        }
    }

    fn perform_part(&mut self, _cx: &mut Context, part: u64, request: &ActionRequest) -> bool {
        let index = part as usize;
        if index >= self.tiles.len() {
            return false;
        }
        match request.action {
            Action::Click => self.select(index, true),
            Action::Focus => self.current = index,
            _ => return false,
        }
        true
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => {
                let Some(index) = self.tile_at(pointer.position) else {
                    return false;
                };
                let toggle = pointer.modifiers.logo || pointer.modifiers.shift;
                self.select(index, toggle);
                if pointer.count >= 2 {
                    (self.share)();
                }
                true
            }
            Event::Key {
                key: Key::Space, ..
            } => {
                self.select(self.current, true);
                true
            }
            Event::Key { key, .. } => self.step(*key),
            _ => false,
        }
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        true
    }
}

pub struct Picker {
    id: WidgetId,
    title: String,
    children: Vec<Box<dyn Widget>>,
    outcome: Outcome,
    title_text: Option<Paragraph>,
    button_sizes: Vec<Size>,
    frame: Rect,
}

impl Picker {
    pub fn new(app_name: Option<&str>, choices: Vec<Choice>, multiple: bool) -> (Self, Outcome) {
        let outcome: Outcome = Rc::new(RefCell::new(None));
        let ids: Vec<String> = choices.iter().map(|choice| choice.id.clone()).collect();
        let selected = Rc::new(RefCell::new(match choices.is_empty() {
            true => Vec::new(),
            false => vec![0],
        }));
        let share: Rc<dyn Fn()> = {
            let (outcome, selected) = (Rc::clone(&outcome), Rc::clone(&selected));
            Rc::new(move || {
                let chosen: Vec<String> = selected
                    .borrow()
                    .iter()
                    .filter_map(|index| ids.get(*index).cloned())
                    .collect();
                if !chosen.is_empty() && outcome.borrow().is_none() {
                    *outcome.borrow_mut() = Some(Some(chosen));
                }
            })
        };
        let tiles = choices
            .into_iter()
            .map(|choice| {
                let label = match (choice.title.is_empty(), choice.app_id.is_empty()) {
                    (false, _) => choice.title,
                    (true, false) => choice.app_id,
                    (true, true) => L10N.tr("untitled-window"),
                };
                Tile {
                    label,
                    image: choice.thumbnail.and_then(|thumbnail| {
                        Image::from_rgba(thumbnail.width, thumbnail.height, thumbnail.rgba)
                    }),
                    paragraph: None,
                    frame: Rect::default(),
                }
            })
            .collect::<Vec<_>>();
        let grid = Grid {
            id: WidgetId::next(),
            columns: columns_for(tiles.len()),
            tiles,
            selected,
            current: 0,
            multiple,
            share: Rc::clone(&share),
            frame: Rect::default(),
        };
        let cancelled = Rc::clone(&outcome);
        let cancel = Button::new(&L10N.tr("cancel"), move || {
            if cancelled.borrow().is_none() {
                *cancelled.borrow_mut() = Some(None);
            }
        })
        .cancel();
        let share_button = Button::new(&L10N.tr("share"), move || share()).primary();
        let title = match app_name {
            Some(name) => L10N.tr_args("share-with", [("app", name.to_owned().into())]),
            None => L10N.tr("share-with-an-app"),
        };
        (
            Self {
                id: WidgetId::next(),
                title,
                children: vec![Box::new(grid), Box::new(cancel), Box::new(share_button)],
                outcome: Rc::clone(&outcome),
                title_text: None,
                button_sizes: Vec::new(),
                frame: Rect::default(),
            },
            outcome,
        )
    }
}

impl Widget for Picker {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let grid = self
            .children
            .first_mut()
            .map(|grid| grid.measure(cx, available))
            .unwrap_or_default();
        let width = (grid.width + 2.0 * PADDING).max(420.0).min(available.width);
        let title = cx.text.paragraph(
            &self.title,
            TextStyle::new(15.0, cx.palette.ink).weight(TextStyle::SEMIBOLD),
            Some(width - 2.0 * PADDING),
            TextAlign::Start,
        );
        self.button_sizes = self
            .children
            .iter_mut()
            .skip(1)
            .map(|button| {
                let size = button.measure(cx, available);
                Size::new(size.width.max(BUTTON_MIN_WIDTH), BUTTON_HEIGHT)
            })
            .collect();
        let height = PADDING
            + title.height()
            + TITLE_GAP
            + grid.height
            + BUTTONS_GAP
            + BUTTON_HEIGHT
            + PADDING;
        self.title_text = Some(title);
        Size::new(width.ceil(), height.ceil())
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let title_height = self.title_text.as_ref().map_or(0.0, Paragraph::height);
        let Some((grid, buttons)) = self.children.split_first_mut() else {
            return;
        };
        let grid_size = grid.measure(cx, Size::new(frame.width, frame.height));
        grid.place(
            cx,
            Rect::new(
                frame.x + ((frame.width - grid_size.width) / 2.0).round(),
                frame.y + PADDING + title_height + TITLE_GAP,
                grid_size.width,
                grid_size.height,
            ),
        );
        let mut x = frame.x + frame.width - PADDING;
        let y = frame.y + frame.height - PADDING - BUTTON_HEIGHT;
        for (button, size) in buttons.iter_mut().zip(&self.button_sizes).rev() {
            x -= size.width;
            button.place(cx, Rect::new(x, y, size.width, size.height));
            x -= BUTTON_SPACING;
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        if let Some(title) = &self.title_text {
            items.push(Item::Text {
                origin: Point::new(self.frame.x + PADDING, self.frame.y + PADDING),
                paragraph: title.clone(),
            });
        }
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Dialog);
        node.set_label(self.title.clone());
        node.set_modal();
        node
    }

    fn dismissed(&self) -> bool {
        self.outcome.borrow().is_some()
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bande_toolkit::ui::{FreeLayout, Modifiers, Ui};
    use bande_toolkit::{Appearance, TextSystem};

    fn choice(id: &str, title: &str) -> Choice {
        Choice {
            id: id.to_owned(),
            title: title.to_owned(),
            app_id: String::new(),
            thumbnail: None,
        }
    }

    fn present(multiple: bool) -> (Ui, Outcome) {
        let appearance = Appearance::default();
        let mut ui = Ui::new(
            Box::new(FreeLayout::new()),
            Context::new(TextSystem::new(), appearance.palette()),
        );
        let (picker, outcome) = Picker::new(
            Some("Meet"),
            vec![
                choice("monitor:screen", "Entire Screen"),
                choice("window:a", "Mail"),
                choice("window:b", "Notes"),
            ],
            multiple,
        );
        ui.present_dialog(picker);
        ui.layout(Size::new(1280.0, 800.0));
        (ui, outcome)
    }

    fn key(ui: &mut Ui, key: Key) {
        ui.handle(&Event::Key {
            key,
            modifiers: Modifiers::default(),
        });
    }

    #[test]
    fn return_shares_the_first_source_by_default() {
        let (mut ui, outcome) = present(false);

        key(&mut ui, Key::Enter);

        assert_eq!(
            *outcome.borrow(),
            Some(Some(vec!["monitor:screen".to_owned()]))
        );
    }

    #[test]
    fn arrow_keys_move_the_choice_between_sources() {
        let (mut ui, outcome) = present(false);

        key(&mut ui, Key::Right);
        key(&mut ui, Key::Right);
        key(&mut ui, Key::Left);
        key(&mut ui, Key::Enter);

        assert_eq!(*outcome.borrow(), Some(Some(vec!["window:a".to_owned()])));
    }

    #[test]
    fn escape_cancels_sharing() {
        let (mut ui, outcome) = present(false);

        key(&mut ui, Key::Escape);

        assert_eq!(*outcome.borrow(), Some(None));
    }

    #[test]
    fn space_adds_a_source_when_several_may_be_shared() {
        let (mut ui, outcome) = present(true);

        key(&mut ui, Key::Down);
        key(&mut ui, Key::Space);
        key(&mut ui, Key::Up);
        key(&mut ui, Key::Space);
        key(&mut ui, Key::Right);
        key(&mut ui, Key::Space);
        key(&mut ui, Key::Enter);

        assert_eq!(
            *outcome.borrow(),
            Some(Some(vec!["window:b".to_owned(), "window:a".to_owned()]))
        );
    }
}
