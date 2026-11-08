#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bande_graphics::{Item, Palette, Point, Scheme, TextSystem};
use bande_widgets::{
    Axis, Button, Context, Event, Key, Modifiers, Pointer, Size, Stack, Ui, Widget,
};

fn rings(ui: &mut Ui) -> usize {
    let focus = Palette::for_scheme(Scheme::Light).focus.into();
    ui.scene(300, 40)
        .items
        .iter()
        .filter(|item| matches!(item, Item::Stroke { paint, .. } if *paint == focus))
        .count()
}

fn click(ui: &mut Ui, at: Point) {
    ui.handle(&Event::PointerDown(Pointer::at(at)));
    ui.handle(&Event::PointerUp(Pointer::at(at)));
}

#[test]
fn focus_ring_shows_only_for_keyboard_focus() {
    let first = Button::new("First", || {});
    let second = Button::new("Second", || {});
    let (first_id, second_id) = (first.id(), second.id());
    let row = Stack::new(
        Axis::Horizontal,
        8.0,
        6.0,
        vec![Box::new(first), Box::new(second)],
    );
    let mut cx = Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light));
    cx.keyboard_navigation = true;
    let mut ui = Ui::new(Box::new(row), cx);
    ui.layout(Size::new(300.0, 40.0));
    let centre = |ui: &mut Ui, id| {
        let frame = ui.widget(id).unwrap().frame();
        Point::new(frame.x + frame.width / 2.0, frame.y + frame.height / 2.0)
    };

    let at = centre(&mut ui, first_id);
    click(&mut ui, at);
    assert_eq!(ui.focus(), Some(first_id));
    assert_eq!(ui.accessibility().focus, first_id.into());
    assert_eq!(rings(&mut ui), 0);

    ui.handle(&Event::Key {
        key: Key::Tab,
        modifiers: Modifiers::default(),
    });
    assert_eq!(ui.focus(), Some(second_id));
    assert_eq!(ui.accessibility().focus, second_id.into());
    assert_eq!(rings(&mut ui), 1);

    let at = centre(&mut ui, first_id);
    click(&mut ui, at);
    assert_eq!(ui.focus(), Some(first_id));
    assert_eq!(rings(&mut ui), 0);
}
