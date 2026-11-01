use std::cell::Cell;
use std::rc::Rc;

use accesskit::{Node, Role};
use bande_graphics::{Item, Palette, Point, Rect, Scheme, TextSystem};
use bande_widgets::{Context, Event, Label, Pointer, Size, Ui, Widget, WidgetId};

struct Pressable {
    id: WidgetId,
    frame: Rect,
    pressed: Rc<Cell<bool>>,
}

impl Widget for Pressable {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, _cx: &Context, _items: &mut Vec<Item>) {}

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::Button)
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(_) => self.pressed.set(true),
            Event::PointerUp(_) | Event::PointerLeave => self.pressed.set(false),
            _ => return false,
        }
        true
    }
}

#[test]
fn a_press_cut_short_by_a_sheet_is_released() {
    let pressed = Rc::new(Cell::new(false));
    let root = Pressable {
        id: WidgetId::next(),
        frame: Rect::default(),
        pressed: pressed.clone(),
    };
    let cx = Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light));
    let mut ui = Ui::new(Box::new(root), cx);
    ui.layout(Size::new(200.0, 100.0));

    ui.handle(&Event::PointerDown(Pointer::at(Point::new(100.0, 50.0))));
    assert!(pressed.get());

    ui.present_dialog(Label::new("Sheet"));
    assert!(
        !pressed.get(),
        "the press outlived the capture it lost to the sheet"
    );
    assert!(!ui.pointer_captured());
}
