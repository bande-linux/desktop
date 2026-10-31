#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bande_graphics::{Appearance, Item, Palette, Scheme, TextSystem};
use bande_widgets::{
    Context, Event, FrameTime, Key, Modifiers, Toggle, Ui, Widget, set_frame_time,
};

struct Clock(Rc<Cell<Instant>>);

impl Clock {
    fn advance(&self, milliseconds: u64) {
        self.0
            .set(self.0.get() + Duration::from_millis(milliseconds));
    }
}

fn switch(reduce_motion: bool) -> (Ui, Clock) {
    let toggle = Toggle::new("Wi-Fi", false, |_| {});
    let id = toggle.id();
    let mut ui = Ui::new(
        Box::new(toggle),
        Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light)),
    );
    ui.set_appearance(Appearance {
        reduce_motion,
        ..Appearance::default()
    });
    let now = Rc::new(Cell::new(Instant::now()));
    let shared = now.clone();
    ui.set_clock(move || shared.get());
    ui.context().keyboard_navigation = true;
    ui.perform(&accesskit::ActionRequest {
        action: accesskit::Action::Focus,
        target_tree: accesskit::TreeId::ROOT,
        target_node: id.into(),
        data: None,
    });
    (ui, Clock(now))
}

fn knob_x(ui: &mut Ui) -> f32 {
    let scene = ui.scene(38, 22);
    let fills = scene
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Fill { shape, .. } => Some(shape.rect.x),
            _ => None,
        })
        .collect::<Vec<_>>();
    fills[1]
}

fn flip(ui: &mut Ui) {
    ui.handle(&Event::Key {
        key: Key::Space,
        modifiers: Modifiers::default(),
    });
}

#[test]
fn a_switch_knob_springs_across_and_settles() {
    let (mut ui, clock) = switch(false);
    let off = knob_x(&mut ui);
    assert!(!ui.is_animating());
    flip(&mut ui);
    assert_eq!(knob_x(&mut ui), off, "motion starts where the knob is");
    assert!(ui.is_animating());
    clock.advance(80);
    let moving = knob_x(&mut ui);
    assert!(moving > off + 1.0);
    clock.advance(1000);
    let on = knob_x(&mut ui);
    assert!(on > moving);
    assert!(
        !ui.is_animating(),
        "a settled spring stops asking for frames"
    );
}

#[test]
fn flipping_back_mid_way_reverses_from_where_the_knob_is() {
    let (mut ui, clock) = switch(false);
    let off = knob_x(&mut ui);
    flip(&mut ui);
    clock.advance(60);
    let turning = knob_x(&mut ui);
    flip(&mut ui);
    let reversed = knob_x(&mut ui);
    assert!((reversed - turning).abs() < 0.01, "no jump on retarget");
    clock.advance(16);
    assert!(
        knob_x(&mut ui) > turning - 0.5,
        "the knob keeps its velocity for a moment instead of snapping back"
    );
    clock.advance(1000);
    assert_eq!(knob_x(&mut ui), off);
}

#[test]
fn with_reduce_motion_the_knob_moves_at_once() {
    let (mut ui, _clock) = switch(true);
    let off = knob_x(&mut ui);
    flip(&mut ui);
    assert!(knob_x(&mut ui) > off + 10.0);
    assert!(!ui.is_animating());
}

#[test]
fn an_animation_started_by_input_moves_on_its_first_frame_by_one_refresh() {
    let refresh = Duration::from_micros(8333);
    let toggle = Toggle::new("Wi-Fi", false, |_| {});
    let id = toggle.id();
    let mut ui = Ui::new(
        Box::new(toggle),
        Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light)),
    );
    let shown = Instant::now();
    let frame = |at: u32| FrameTime {
        presentation: shown + refresh * at,
        interval: refresh,
    };
    set_frame_time(frame(1));
    ui.context().keyboard_navigation = true;
    ui.perform(&accesskit::ActionRequest {
        action: accesskit::Action::Focus,
        target_tree: accesskit::TreeId::ROOT,
        target_node: id.into(),
        data: None,
    });
    let off = knob_x(&mut ui);
    flip(&mut ui);
    let first = knob_x(&mut ui);
    assert!(
        first > off,
        "the first frame already shows one refresh of motion"
    );
    set_frame_time(frame(2));
    assert!(
        knob_x(&mut ui) > first,
        "each frame is sampled at its presentation"
    );
}
