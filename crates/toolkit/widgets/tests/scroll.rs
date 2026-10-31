#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use unbound_graphics::{Palette, Point, Scheme, TextSystem};
use unbound_widgets::{
    Axis, Context, Event, Key, Modifiers, Pointer, Scroll, ScrollPhase, ScrollSource, ScrollState,
    ScrollView, Size, Stack, TEXT_FIELD_HEIGHT, TextField, Ui, Widget, WidgetId,
};

const VIEWPORT: Size = Size::new(200.0, 100.0);

struct Scrolling {
    ui: Ui,
    now: Rc<Cell<Instant>>,
    epoch: Instant,
    root: WidgetId,
    state: Rc<RefCell<ScrollState>>,
    view: WidgetId,
    fields: Vec<WidgetId>,
}

fn scroll_view(count: usize, state: &Rc<RefCell<ScrollState>>) -> (ScrollView, Vec<WidgetId>) {
    let fields = (0..count)
        .map(|index| TextField::new(&format!("Field {index}"), "", |_| {}))
        .collect::<Vec<_>>();
    let ids = fields.iter().map(Widget::id).collect();
    let stack = Stack::new(
        Axis::Vertical,
        0.0,
        0.0,
        fields
            .into_iter()
            .map(|field| Box::new(field) as Box<dyn Widget>)
            .collect(),
    );
    (ScrollView::shared(Box::new(stack), Rc::clone(state)), ids)
}

impl Scrolling {
    fn new(count: usize) -> Self {
        let state = Rc::default();
        let (view, fields) = scroll_view(count, &state);
        let view_id = view.id();
        let root = Stack::new(Axis::Vertical, 0.0, 0.0, vec![Box::new(view)]);
        let root_id = root.id();
        let mut ui = Ui::new(
            Box::new(root),
            Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light)),
        );
        let now = Rc::new(Cell::new(Instant::now()));
        let shared = now.clone();
        ui.set_clock(move || shared.get());
        let epoch = now.get();
        let mut scrolling = Self {
            ui,
            now,
            epoch,
            root: root_id,
            state,
            view: view_id,
            fields,
        };
        scrolling.frame();
        scrolling
    }

    fn rebuild(&mut self) {
        let (view, fields) = scroll_view(self.fields.len(), &self.state);
        self.view = view.id();
        self.fields = fields;
        let root = self.ui.widget(self.root).expect("root");
        root.children_mut()[0] = Box::new(view);
    }

    fn frame(&mut self) {
        self.ui.scene(VIEWPORT.width as u32, VIEWPORT.height as u32);
    }

    fn advance(&mut self, milliseconds: u64) {
        let step = 16;
        let mut left = milliseconds;
        while left > 0 {
            let tick = left.min(step);
            self.now.set(self.now.get() + Duration::from_millis(tick));
            self.frame();
            left -= tick;
        }
    }

    fn offset(&mut self) -> f32 {
        self.frame();
        let first = self.ui.widget(self.fields[0]).expect("first field").frame();
        let view = self.ui.widget(self.view).expect("view").frame();
        view.y - first.y
    }

    fn scroll(&mut self, delta: f32, source: ScrollSource, phase: ScrollPhase) -> bool {
        self.ui.handle(&Event::Scroll(Scroll {
            position: Point::new(50.0, 50.0),
            delta: Point::new(0.0, delta),
            source,
            phase,
            modifiers: Modifiers::default(),
            time: self.now.get() - self.epoch,
        }))
    }

    fn limit(&self) -> f32 {
        TEXT_FIELD_HEIGHT * self.fields.len() as f32 - VIEWPORT.height
    }
}

#[test]
fn the_wheel_glides_to_its_target_and_stops_at_the_end_of_the_content() {
    let mut view = Scrolling::new(10);
    assert!(view.scroll(40.0, ScrollSource::Wheel, ScrollPhase::Changed));
    view.advance(16);
    let midway = view.offset();
    assert!(
        midway > 0.0 && midway < 40.0,
        "the wheel animates: {midway}"
    );
    view.advance(600);
    assert_eq!(view.offset(), 40.0);
    view.scroll(10_000.0, ScrollSource::Wheel, ScrollPhase::Changed);
    view.advance(800);
    assert_eq!(view.offset(), view.limit());
}

#[test]
fn content_that_fits_lets_scrolling_pass_to_an_outer_view() {
    let mut view = Scrolling::new(2);
    assert!(!view.scroll(40.0, ScrollSource::Wheel, ScrollPhase::Changed));
    assert_eq!(view.offset(), 0.0);
}

#[test]
fn a_finger_flick_keeps_gliding_after_the_lift_and_slows_to_a_stop() {
    let mut view = Scrolling::new(100);
    view.scroll(10.0, ScrollSource::Finger, ScrollPhase::Began);
    for _ in 0..5 {
        view.advance(10);
        view.scroll(20.0, ScrollSource::Finger, ScrollPhase::Changed);
    }
    view.scroll(0.0, ScrollSource::Finger, ScrollPhase::Ended);
    let lifted = view.offset();
    assert_eq!(lifted, 110.0, "the content follows the finger exactly");
    view.advance(100);
    let gliding = view.offset();
    assert!(gliding > lifted + 50.0, "momentum carries on: {gliding}");
    view.advance(2_000);
    let rest = view.offset();
    view.advance(500);
    assert_eq!(view.offset(), rest, "it comes to rest");
    assert!(rest < view.limit());
}

#[test]
fn a_flick_is_measured_by_event_timestamps_even_when_events_arrive_together() {
    let mut view = Scrolling::new(100);
    let at = Duration::from_millis;
    view.ui.handle(&finger(10.0, ScrollPhase::Began, at(0)));
    for step in 1..=5 {
        view.ui
            .handle(&finger(20.0, ScrollPhase::Changed, at(step * 10)));
    }
    view.ui.handle(&finger(0.0, ScrollPhase::Ended, at(50)));
    let lifted = view.offset();
    view.advance(100);
    let gliding = view.offset() - lifted;
    assert!(
        gliding > 100.0 && gliding < 300.0,
        "a 2000 pt/s flick glides like one: {gliding}"
    );
}

#[test]
fn a_finger_that_rests_before_lifting_does_not_fling() {
    let mut view = Scrolling::new(100);
    let at = Duration::from_millis;
    view.ui.handle(&finger(10.0, ScrollPhase::Began, at(0)));
    for step in 1..=5 {
        view.ui
            .handle(&finger(20.0, ScrollPhase::Changed, at(step * 10)));
    }
    view.ui.handle(&finger(0.0, ScrollPhase::Ended, at(400)));
    let lifted = view.offset();
    view.advance(300);
    assert_eq!(view.offset(), lifted);
}

#[test]
fn the_scrollbar_lingers_without_redrawing_then_fades_out() {
    let mut view = Scrolling::new(10);
    view.scroll(40.0, ScrollSource::Wheel, ScrollPhase::Changed);
    view.advance(400);
    assert!(!view.ui.is_animating(), "a still scrollbar needs no frames");
    let wake = view
        .ui
        .deadline()
        .expect("the end of the linger is scheduled");
    assert!(wake > view.now.get());
    view.now.set(wake + Duration::from_millis(16));
    view.frame();
    assert!(view.ui.is_animating(), "the fade-out animates");
    view.advance(400);
    assert!(!view.ui.is_animating());
    assert_eq!(view.ui.deadline(), None);
}

fn finger(delta: f32, phase: ScrollPhase, time: Duration) -> Event {
    Event::Scroll(Scroll {
        position: Point::new(50.0, 50.0),
        delta: Point::new(0.0, delta),
        source: ScrollSource::Finger,
        phase,
        modifiers: Modifiers::default(),
        time,
    })
}

#[test]
fn pulling_past_the_top_resists_and_springs_back_on_release() {
    let mut view = Scrolling::new(10);
    view.scroll(-50.0, ScrollSource::Finger, ScrollPhase::Began);
    view.scroll(-50.0, ScrollSource::Finger, ScrollPhase::Changed);
    let pulled = view.offset();
    assert!(pulled < 0.0 && pulled > -100.0, "rubber band: {pulled}");
    view.advance(300);
    view.scroll(0.0, ScrollSource::Finger, ScrollPhase::Ended);
    view.advance(800);
    assert_eq!(view.offset(), 0.0);
}

#[test]
fn tabbing_to_a_field_below_the_fold_scrolls_it_into_view() {
    let mut view = Scrolling::new(10);
    for _ in 0..6 {
        view.ui.handle(&Event::Key {
            key: Key::Tab,
            modifiers: Modifiers::default(),
        });
    }
    view.advance(800);
    let field = view.ui.widget(view.fields[5]).expect("field").frame();
    assert!(field.y >= 0.0 && field.y + field.height <= VIEWPORT.height + 0.5);
    assert!(view.offset() > 0.0);
}

#[test]
fn dragging_the_scrollbar_knob_scrolls_in_proportion() {
    let mut view = Scrolling::new(10);
    let strip_x = VIEWPORT.width - 6.0;
    view.scroll(1.0, ScrollSource::Wheel, ScrollPhase::Changed);
    view.advance(400);
    let start = view.offset();
    view.ui
        .handle(&Event::PointerDown(Pointer::at(Point::new(strip_x, 8.0))));
    view.ui
        .handle(&Event::PointerMove(Pointer::at(Point::new(strip_x, 200.0))));
    view.ui
        .handle(&Event::PointerUp(Pointer::at(Point::new(strip_x, 200.0))));
    assert!(start < 2.0);
    assert_eq!(view.offset(), view.limit());
}

#[test]
fn a_rebuilt_view_over_the_same_state_keeps_the_offset() {
    let mut view = Scrolling::new(10);
    view.scroll(40.0, ScrollSource::Wheel, ScrollPhase::Changed);
    view.advance(600);
    view.rebuild();
    assert_eq!(view.offset(), 40.0);
}

#[test]
fn a_pull_past_the_end_springs_back_on_a_rebuilt_view() {
    let mut view = Scrolling::new(10);
    let limit = view.limit();
    view.scroll(limit + 50.0, ScrollSource::Finger, ScrollPhase::Began);
    view.scroll(50.0, ScrollSource::Finger, ScrollPhase::Changed);
    view.rebuild();
    let pulled = view.offset();
    assert!(
        pulled > limit && pulled < limit + 100.0,
        "rubber band: {pulled}"
    );
    view.advance(300);
    view.scroll(0.0, ScrollSource::Finger, ScrollPhase::Ended);
    view.rebuild();
    view.advance(800);
    assert_eq!(view.offset(), limit);
}
