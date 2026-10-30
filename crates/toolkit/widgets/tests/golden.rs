#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use accesskit::{Action, ActionRequest, TreeId};
use unbound_graphics::{CpuRenderer, Palette, Scheme, TextSystem, pixels};
use unbound_widgets::{Axis, Context, Event, Stack, TextField, TextView, Ui, Widget, WidgetId};

const MIN_SSIM: f64 = 0.99;
const SCALES: [(f64, &str); 2] = [(1.0, "1x"), (1.5, "1.5x")];
const SCHEMES: [(Scheme, &str); 2] = [(Scheme::Light, "light"), (Scheme::Dark, "dark")];

fn check_golden(name: &str, size: (u32, u32), rgba: Vec<u8>) {
    let actual = image::RgbaImage::from_raw(size.0, size.1, rgba).expect("frame");
    let actual = image::DynamicImage::ImageRgba8(actual).to_rgb8();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.png"));
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

fn focus(ui: &mut Ui, id: WidgetId) {
    ui.perform(&ActionRequest {
        action: Action::Focus,
        target_tree: TreeId::ROOT,
        target_node: id.into(),
        data: None,
    });
}

fn shoot(name: &str, size: (u32, u32), build: fn(Scheme) -> Ui) {
    for (scheme, scheme_name) in SCHEMES {
        for (scale, scale_name) in SCALES {
            let mut ui = build(scheme);
            let scene = ui.scene(size.0, size.1);
            let (width, height) = (pixels(size.0, scale), pixels(size.1, scale));
            let mut rgba = vec![0; (width * height * 4) as usize];
            CpuRenderer::new()
                .render_into(&scene, scale, &mut rgba)
                .expect("render");
            check_golden(
                &format!("{name}-{scheme_name}-{scale_name}"),
                (width, height),
                rgba,
            );
        }
    }
}

fn text_editing(scheme: Scheme) -> Ui {
    let empty = TextField::new("Search", "", |_| {});
    let mut selected = TextField::new("Name", "Quarterly report", |_| {});
    selected.select(10..16);
    let composing = TextField::new("Name", "Tokyo ", |_| {});
    let mut notes = TextView::new(
        "Notes",
        "Bring the signed contract.\nAsk about the new office layout, the parking spaces and the date we can move in.",
        |_| {},
    )
    .visible_lines(4);
    notes.select(5..15);
    let (selected_id, composing_id) = (selected.id(), composing.id());
    let stack = Stack::new(
        Axis::Vertical,
        12.0,
        16.0,
        vec![
            Box::new(empty),
            Box::new(selected),
            Box::new(composing),
            Box::new(notes),
        ],
    );
    let mut ui = Ui::new(
        Box::new(stack),
        Context::new(TextSystem::new(), Palette::for_scheme(scheme)),
    );
    ui.layout(unbound_widgets::Size::new(320.0, 220.0));
    focus(&mut ui, composing_id);
    ui.handle(&Event::Preedit {
        text: "とうきょう".to_owned(),
        cursor: Some((6, 9)),
    });
    focus(&mut ui, selected_id);
    ui
}

#[test]
fn text_fields_and_views_match_their_goldens() {
    shoot("text-editing", (320, 220), text_editing);
}
