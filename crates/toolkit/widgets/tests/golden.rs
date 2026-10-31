#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use accesskit::{Action, ActionRequest, TreeId};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;
use unbound_graphics::{CpuRenderer, Image, Palette, Scheme, TextSize, TextSystem, pixels};

use unbound_widgets::{
    Axis, Button, CheckState, Checkbox, ComboBox, Context, Event, Pointer, PopUpButton,
    ProgressBar, RadioGroup, SearchField, SegmentedControl, Size, Slider, Spinner, Stack, Stepper,
    TOOLTIP_DELAY, TextField, TextView, Toggle, Ui, Widget, WidgetId,
};

const LIGHT: (Scheme, &str, f64, &str) = (Scheme::Light, "light", 1.0, "1x");
const DARK: (Scheme, &str, f64, &str) = (Scheme::Dark, "dark", 1.0, "1x");
const LIGHT_FRACTIONAL: (Scheme, &str, f64, &str) = (Scheme::Light, "light", 1.5, "1.5x");

fn check_golden(name: &str, size: (u32, u32), rgba: Vec<u8>) {
    unbound_golden::assert_golden!(name, &unbound_golden::from_rgba(size.0, size.1, rgba));
}

fn focus(ui: &mut Ui, id: WidgetId) {
    ui.perform(&ActionRequest {
        action: Action::Focus,
        target_tree: TreeId::ROOT,
        target_node: id.into(),
        data: None,
    });
}

fn shoot(
    name: &str,
    size: (u32, u32),
    build: fn(Scheme) -> Ui,
    variants: &[(Scheme, &str, f64, &str)],
) {
    for &(scheme, scheme_name, scale, scale_name) in variants {
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
    shoot("text-editing", (320, 254), text_editing, &[LIGHT]);
}

fn row(children: Vec<Box<dyn Widget>>) -> Box<dyn Widget> {
    Box::new(Stack::new(Axis::Horizontal, 16.0, 0.0, children))
}

const TRASH: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path fill="#201E1A" d="M6 1h4l1 1h3v1.5H2V2h3zM3 5h10l-.8 9.2A1 1 0 0 1 11.2 15H4.8a1 1 0 0 1-1-.8zm3 2v6h1.2V7zm2.8 0v6H10V7z"/></svg>"##;

fn controls(scheme: Scheme) -> Ui {
    controls_at(scheme, TextSize::DEFAULT, CONTROLS)
}

fn larger_controls(scheme: Scheme) -> Ui {
    controls_at(scheme, TextSize::from_percent(150), LARGER_CONTROLS)
}

fn controls_at(scheme: Scheme, text_size: TextSize, area: (u32, u32)) -> Ui {
    let trash = Image::from_svg(TRASH, 32, 32).expect("symbol");
    let delete = Button::symbol(trash, "Move to Trash", || {});
    let delete_id = delete.id();
    let buttons = row(vec![
        Box::new(Button::new("Save", || {}).primary()),
        Box::new(Button::new("Cancel", || {}).cancel()),
        Box::new(Button::new("Options", || {}).quiet()),
        Box::new(Button::new("Move to Trash", || {}).destructive()),
        Box::new(delete),
    ]);
    let empty = TextField::new("Name", "", |_| {});
    let typed = TextField::new("Name", "Field recordings", |_| {});
    let typed_id = typed.id();
    let choices = row(vec![
        Box::new(Toggle::new("Hidden files", true, |_| {})),
        Box::new(Toggle::new("Ask first", false, |_| {})),
        Box::new(Checkbox::new("On", CheckState::On, |_| {})),
        Box::new(Checkbox::new("Off", CheckState::Off, |_| {})),
        Box::new(Checkbox::new("Mixed", CheckState::Mixed, |_| {})),
        Box::new(RadioGroup::new("Sort", &["Name", "Date"], Some(0), |_| {})),
    ]);
    let pickers = row(vec![
        Box::new(SegmentedControl::new(
            "View",
            &["Icons", "List", "Columns"],
            Some(1),
            |_| {},
        )),
        Box::new(PopUpButton::new(
            "Sort by",
            &["Name", "Date Modified"],
            1,
            |_| {},
        )),
        Box::new(
            Stepper::new("Size", 12.0, (9.0, 24.0), 1.0, |_| {})
                .format(|value| format!("{value} pt")),
        ),
        Box::new(Spinner::new("Loading")),
    ]);
    let combo = Box::new(ComboBox::new("Size", "12 pt", &["10 pt", "12 pt"], |_| {}));
    let stack = Stack::new(
        Axis::Vertical,
        14.0,
        16.0,
        vec![
            buttons,
            Box::new(empty),
            Box::new(typed),
            choices,
            pickers,
            combo,
            Box::new(Slider::new("Volume", 0.62, (0.0, 1.0), 0.01, |_| {})),
            Box::new(Slider::new("Speed", 0.5, (0.0, 1.0), 0.01, |_| {}).ticks(5)),
            Box::new(ProgressBar::new("Copying", Some(0.4))),
            Box::new(ProgressBar::new("Preparing", None)),
        ],
    );
    let mut text = TextSystem::new();
    text.set_size(text_size);
    let mut ui = Ui::new(
        Box::new(stack),
        Context::new(text, Palette::for_scheme(scheme)),
    );
    let clock = Rc::new(Cell::new(Instant::now()));
    let time = clock.clone();
    ui.set_clock(move || time.get());
    ui.layout(Size::new(area.0 as f32, area.1 as f32));
    focus(&mut ui, typed_id);
    let frame = ui.widget(delete_id).expect("button").frame();
    ui.handle(&Event::PointerMove(Pointer::at(
        unbound_graphics::Point::new(frame.x + 10.0, frame.y + 8.0),
    )));
    clock.set(clock.get() + TOOLTIP_DELAY * 2);
    ui
}

const CONTROLS: (u32, u32) = (600, 470);
const LARGER_CONTROLS: (u32, u32) = (760, 600);

#[test]
fn controls_match_their_goldens() {
    shoot(
        "controls",
        CONTROLS,
        controls,
        &[LIGHT, DARK, LIGHT_FRACTIONAL],
    );
    shoot("controls-150", LARGER_CONTROLS, larger_controls, &[LIGHT]);
}
