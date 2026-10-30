#![allow(clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use unbound_screenshot::area::AreaSelection;
use unbound_screenshot::palette::{Mode, Palette as CapturePalette};
use unbound_toolkit::ui::{Context, Event, FreeLayout, Pointer, Size, Ui, Widget};
use unbound_toolkit::{
    Color, CpuRenderer, Item, Palette, Point, Rect, Scene, Scheme, Shape, TextSystem,
};

const MIN_SSIM: f64 = 0.99;
const SIZE: (u32, u32) = (640, 400);

fn backdrop() -> Item {
    Item::Fill {
        shape: Shape::rect(Rect::new(0.0, 0.0, SIZE.0 as f32, SIZE.1 as f32)),
        paint: Color::hex(0x7C8698).into(),
    }
}

fn check_golden(name: &str, scene: &Scene) {
    for (scale, suffix) in [(1.0, ""), (1.5, "-1.5x")] {
        let (width, height) = (
            (f64::from(scene.width) * scale).round() as u32,
            (f64::from(scene.height) * scale).round() as u32,
        );
        let mut rgba = vec![0; (width * height * 4) as usize];
        CpuRenderer::new()
            .render_into(scene, scale, &mut rgba)
            .expect("frame");
        let actual = image::RgbaImage::from_raw(width, height, rgba).expect("frame");
        let actual = image::DynamicImage::ImageRgba8(actual).to_rgb8();
        let name = format!("{name}{suffix}");
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.png"));
        if std::env::var_os("UNBOUND_UPDATE_GOLDENS").is_some() {
            std::fs::create_dir_all(path.parent().expect("golden directory")).expect("directory");
            actual.save(&path).expect("write golden");
        }
        let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
        actual.save(&target).expect("save the frame");
        let expected = image::open(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
            .to_rgb8();
        let similarity = image_compare::rgb_hybrid_compare(&expected, &actual).expect("same size");
        assert!(
            similarity.score >= MIN_SSIM,
            "{name} differs from its golden: {:.4} (frame in {})",
            similarity.score,
            target.display()
        );
    }
}

const SCHEMES: [(Scheme, &str); 2] = [(Scheme::Light, "light"), (Scheme::Dark, "dark")];

#[test]
fn an_area_being_chosen_matches_its_goldens() {
    for (scheme, look) in SCHEMES {
        let mut selection = AreaSelection::new(Vec::new(), false);
        selection.handle(&Event::PointerDown(Pointer::at(Point::new(120.0, 90.0))));
        selection.handle(&Event::PointerMove(Pointer::at(Point::new(420.0, 290.0))));
        let mut scene = selection.scene(
            SIZE.0,
            SIZE.1,
            &Palette::for_scheme(scheme),
            &mut TextSystem::new(),
        );
        scene.items.insert(0, backdrop());
        check_golden(&format!("area-{look}"), &scene);
    }
}

#[test]
fn the_capture_toolbar_matches_its_goldens() {
    for (scheme, look) in SCHEMES {
        let palette = Palette::for_scheme(scheme);
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
        check_golden(&format!("toolbar-{look}"), &scene);
    }
}
