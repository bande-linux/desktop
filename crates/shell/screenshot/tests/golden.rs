#![allow(clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use unbound_screenshot::area::AreaSelection;
use unbound_screenshot::palette::{Mode, Palette as CapturePalette};
use unbound_screenshot::{Failure, L10N};
use unbound_toolkit::ui::{Alert, Context, Event, FreeLayout, Pointer, Size, Ui, Widget};
use unbound_toolkit::{
    Color, CpuRenderer, Item, Palette, Point, Rect, Scene, Scheme, Shape, TextSystem,
};

const SIZE: (u32, u32) = (640, 400);

fn backdrop() -> Item {
    Item::Fill {
        shape: Shape::rect(Rect::new(0.0, 0.0, SIZE.0 as f32, SIZE.1 as f32)),
        paint: Color::hex(0x7C8698).into(),
    }
}

fn check_golden(name: &str, scene: &Scene) {
    let frame = CpuRenderer::new().render(scene).expect("frame");
    unbound_golden::assert_golden_rgba!(name, (frame.width, frame.height), frame.rgba);
}

#[test]
fn an_area_being_chosen_matches_its_golden() {
    let mut selection = AreaSelection::new(Vec::new(), false);
    selection.handle(&Event::PointerDown(Pointer::at(Point::new(120.0, 90.0))));
    selection.handle(&Event::PointerMove(Pointer::at(Point::new(420.0, 290.0))));
    let mut scene = selection.scene(
        SIZE.0,
        SIZE.1,
        &Palette::for_scheme(Scheme::Light),
        &mut TextSystem::new(),
    );
    scene.items.insert(0, backdrop());
    check_golden("area-light", &scene);
}

#[test]
fn the_capture_toolbar_matches_its_golden() {
    let palette = Palette::for_scheme(Scheme::Light);
    let mut cx = Context::new(TextSystem::new(), palette);
    let (mut toolbar, _) = CapturePalette::new(Mode::Area);
    let size = toolbar.measure(&mut cx, Size::new(SIZE.0 as f32, SIZE.1 as f32));
    let mut layout = FreeLayout::new();
    layout.insert(
        toolbar,
        Rect::new(
            ((SIZE.0 as f32 - size.width) / 2.0).round(),
            SIZE.1 as f32 - 40.0 - size.height,
            size.width,
            size.height,
        ),
    );
    let mut ui = Ui::new(Box::new(layout), cx);
    let mut scene = ui.scene(SIZE.0, SIZE.1);
    scene.items.insert(0, backdrop());
    check_golden("toolbar-light", &scene);
}

#[test]
fn a_recording_that_cannot_start_says_so_and_matches_its_golden() {
    let (title, detail) = Failure::ScreenUnavailable.message();
    let mut ui = Ui::new(
        Box::new(FreeLayout::new()),
        Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light)),
    );
    ui.present_dialog(Alert::new(&title, &detail, |_| {}).primary(&L10N.tr("ok"), "ok"));
    ui.layout(Size::new(SIZE.0 as f32, SIZE.1 as f32));
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut scene = ui.scene(SIZE.0, SIZE.1);
    while ui.is_animating() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(16));
        scene = ui.scene(SIZE.0, SIZE.1);
    }
    scene.items.insert(0, backdrop());

    assert_eq!(title, "Screen recording is not available");
    check_golden("cannot-record-light", &scene);
}
