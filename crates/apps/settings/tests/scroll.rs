#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use accesskit::{Node, Role, TreeUpdate};
use unbound_settings::{PaneId, SettingId, SettingsView, Update, Value, sidebar_width};
use unbound_toolkit::ui::{Event, Key, Modifiers, Scroll, ScrollPhase, ScrollSource};
use unbound_toolkit::{Appearance, Palette, Point, Scheme, TextSize, TextSystem};

const PAGE: Point = Point::new(600.0, 100.0);
const SIDEBAR: Point = Point::new(100.0, 100.0);

struct Window {
    view: SettingsView,
    size: (u32, u32),
}

impl Window {
    fn new(pane: PaneId, text_size: TextSize, size: (u32, u32)) -> Self {
        let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
        view.set_appearance(Appearance {
            reduce_motion: true,
            text_size,
            ..Appearance::default()
        });
        view.select_pane(pane);
        let mut window = Self { view, size };
        window.tree();
        window
    }

    fn tree(&mut self) -> TreeUpdate {
        self.view.scene(self.size.0, self.size.1);
        self.view.accessibility()
    }

    fn handle(&mut self, event: Event) {
        self.view.handle(&event);
        self.tree();
    }

    fn wheel(&mut self, at: Point, delta: f32) {
        self.handle(Event::Scroll(Scroll {
            position: at,
            delta: Point::new(0.0, delta),
            source: ScrollSource::Wheel,
            phase: ScrollPhase::Changed,
            modifiers: Modifiers::default(),
            time: Duration::ZERO,
        }));
    }

    fn tab(&mut self) {
        self.handle(Event::Key {
            key: Key::Tab,
            modifiers: Modifiers::default(),
        });
    }

    fn rebuild(&mut self) {
        self.view.apply(Update::Values(vec![(
            SettingId::KeyRepeatRate,
            Value::Number(30),
        )]));
        self.tree();
    }

    fn scroll_views(&mut self) -> Vec<Node> {
        self.tree()
            .nodes
            .into_iter()
            .map(|(_, node)| node)
            .filter(|node| node.role() == Role::ScrollView)
            .collect()
    }

    fn sidebar_offset(&mut self) -> f64 {
        let views = self.scroll_views();
        views.first().and_then(Node::scroll_y).expect("sidebar")
    }

    fn page_offset(&mut self) -> f64 {
        let views = self.scroll_views();
        views.last().and_then(Node::scroll_y).expect("page")
    }

    fn visible(&self, bounds: accesskit::Rect) -> bool {
        bounds.y0 >= 0.0 && bounds.y1 <= f64::from(self.size.1)
    }

    fn shows(&mut self, label: &str) -> bool {
        self.tree().nodes.into_iter().any(|(_, node)| {
            node.label() == Some(label) && node.bounds().is_some_and(|bounds| self.visible(bounds))
        })
    }

    fn focused(&mut self) -> Node {
        let update = self.tree();
        update
            .nodes
            .into_iter()
            .find(|(id, _)| *id == update.focus)
            .map(|(_, node)| node)
            .expect("a focused control")
    }

    fn tab_into_page(&mut self) -> Node {
        for _ in 0..40 {
            self.tab();
            let focused = self.focused();
            if focused
                .bounds()
                .is_some_and(|bounds| bounds.x0 > f64::from(sidebar_width(TextSize::DEFAULT)))
            {
                return focused;
            }
        }
        panic!("no control on the page");
    }
}

fn last_shortcut() -> String {
    unbound_keyboard::Area::ALL
        .into_iter()
        .flat_map(unbound_keyboard::shortcuts_in)
        .last()
        .expect("a shortcut")
        .action
        .label()
}

#[test]
fn the_wheel_scrolls_a_long_page_to_its_end() {
    let mut window = Window::new(PaneId::KeyboardShortcuts, TextSize::DEFAULT, (900, 720));
    let last = last_shortcut();
    assert!(!window.shows(&last));
    window.wheel(PAGE, 10_000.0);
    assert!(window.page_offset() > 0.0);
    assert!(window.shows(&last));
}

#[test]
fn the_page_keeps_its_place_when_settings_rebuild() {
    let mut window = Window::new(PaneId::KeyboardShortcuts, TextSize::DEFAULT, (900, 720));
    window.wheel(PAGE, 300.0);
    assert_eq!(window.page_offset(), 300.0);
    window.rebuild();
    assert_eq!(window.page_offset(), 300.0);
}

#[test]
fn tabbing_onto_a_control_below_the_fold_scrolls_it_into_view() {
    let mut window = Window::new(PaneId::Accessibility, TextSize::DEFAULT, (900, 170));
    let first = window.tab_into_page();
    assert_eq!(window.page_offset(), 0.0);
    window.tab();
    let second = window.focused();
    assert_ne!(second.label(), first.label());
    assert!(window.page_offset() > 0.0);
    assert!(second.bounds().is_some_and(|bounds| window.visible(bounds)));
}

#[test]
fn a_rebuild_does_not_pull_the_page_back_to_the_focused_control() {
    let mut window = Window::new(PaneId::Accessibility, TextSize::DEFAULT, (900, 120));
    window.tab_into_page();
    assert!(window.page_offset() > 0.0);
    window.wheel(PAGE, -10_000.0);
    assert_eq!(window.page_offset(), 0.0);
    window.rebuild();
    assert_eq!(window.page_offset(), 0.0);
}

#[test]
fn the_sidebar_scrolls_with_the_wheel_at_large_text() {
    let mut window = Window::new(PaneId::Appearance, TextSize::from_percent(150), (900, 320));
    window.wheel(SIDEBAR, 10_000.0);
    assert!(window.sidebar_offset() > 0.0);
}
