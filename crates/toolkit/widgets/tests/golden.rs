#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use accesskit::{Action, ActionRequest, TreeId};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;
use unbound_graphics::{CpuRenderer, Palette, Scheme, TextSystem, pixels};

use unbound_widgets::{
    Axis, Button, CheckState, Checkbox, ComboBox, Context, Event, Pointer, PopUpButton,
    ProgressBar, RadioGroup, SearchField, SegmentedControl, Size, Spinner, Stack, Stepper,
    TOOLTIP_DELAY, TextField, TextView, Ui, Widget, WidgetId,
};

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
    let empty = SearchField::new("Search", "", |_| {});
    let searching = SearchField::new("Search", "invoice", |_| {});
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
            Box::new(searching),
            Box::new(selected),
            Box::new(composing),
            Box::new(notes),
        ],
    );
    let mut ui = Ui::new(
        Box::new(stack),
        Context::new(TextSystem::new(), Palette::for_scheme(scheme)),
    );
    ui.layout(Size::new(320.0, 254.0));
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
    shoot("text-editing", (320, 254), text_editing);
}

fn row(children: Vec<Box<dyn Widget>>) -> Box<dyn Widget> {
    Box::new(Stack::new(Axis::Horizontal, 16.0, 0.0, children))
}

fn controls(scheme: Scheme) -> Ui {
    let checks = row(vec![
        Box::new(Checkbox::new("Off", CheckState::Off, |_| {})),
        Box::new(Checkbox::new("On", CheckState::On, |_| {})),
        Box::new(Checkbox::new("Mixed", CheckState::Mixed, |_| {})),
    ]);
    let choices = row(vec![
        Box::new(RadioGroup::new(
            "Size",
            &["Small", "Large"],
            Some(1),
            |_| {},
        )),
        Box::new(SegmentedControl::new(
            "View",
            &["Icons", "List", "Columns"],
            Some(1),
            |_| {},
        )),
    ]);
    let pickers = row(vec![
        Box::new(PopUpButton::new(
            "Sort by",
            &["Name", "Date Modified"],
            1,
            |_| {},
        )),
        Box::new(Stepper::new("Copies", 2.0, (1.0, 9.0), 1.0, |_| {})),
        Box::new(Spinner::new("Loading")),
    ]);
    let combo = Box::new(ComboBox::new("Size", "12 pt", &["10 pt", "12 pt"], |_| {}));
    let help = Button::new("Stop", || {}).tooltip("Quit the selected process");
    let help_id = help.id();
    let stack = Stack::new(
        Axis::Vertical,
        14.0,
        16.0,
        vec![
            checks,
            choices,
            pickers,
            combo,
            Box::new(ProgressBar::new("Copying", Some(0.4))),
            Box::new(ProgressBar::new("Preparing", None)),
            row(vec![Box::new(help)]),
        ],
    );
    let mut ui = Ui::new(
        Box::new(stack),
        Context::new(TextSystem::new(), Palette::for_scheme(scheme)),
    );
    let clock = Rc::new(Cell::new(Instant::now()));
    let time = clock.clone();
    ui.set_clock(move || time.get());
    ui.layout(Size::new(360.0, 296.0));
    let frame = ui.widget(help_id).expect("button").frame();
    ui.handle(&Event::PointerMove(Pointer::at(
        unbound_graphics::Point::new(frame.x + 10.0, frame.y + 8.0),
    )));
    clock.set(clock.get() + TOOLTIP_DELAY * 2);
    ui
}

#[test]
fn controls_match_their_goldens() {
    shoot("controls", (360, 296), controls);
}
