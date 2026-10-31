#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::Cell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use unbound_bar::low_memory::{Effect, LOW_MEMORY_SIZE, LowMemoryView};
use unbound_status::memory::AppMemory;
use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{CpuRenderer, Image, Palette, Scheme, TextSystem};

const SETTLED: Duration = Duration::from_secs(3);

fn repository_icon(path: &str, points: u32) -> Image {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path);
    Image::load(&path, points).expect("icon")
}

fn app(id: &str, name: &str, megabytes: u64) -> AppMemory {
    AppMemory {
        app_id: id.to_owned(),
        name: name.to_owned(),
        icon: Some(id.to_owned()),
        units: vec![format!("app-unbound-{id}-1.scope")],
        bytes: megabytes * 1_000_000,
    }
}

struct Dialog {
    view: LowMemoryView,
    clock: Rc<Cell<Instant>>,
}

impl Dialog {
    fn new(scheme: Scheme) -> Self {
        let disc = repository_icon("apps/monitor/data/org.unbound.ActivityMonitor.svg", 36);
        let mut view =
            LowMemoryView::new(Palette::for_scheme(scheme), TextSystem::new(), Some(disc));
        let clock = Rc::new(Cell::new(Instant::now()));
        let now = Rc::clone(&clock);
        view.ui().set_clock(move || now.get());
        let icons = HashMap::from([
            (
                "org.unbound.Terminal".to_owned(),
                repository_icon("apps/terminal/data/org.unbound.Terminal.svg", 36),
            ),
            (
                "org.unbound.Files".to_owned(),
                repository_icon("apps/files/data/org.unbound.Files.svg", 36),
            ),
            (
                "org.unbound.Settings".to_owned(),
                repository_icon("apps/settings/data/org.unbound.Settings.svg", 36),
            ),
        ]);
        view.set_apps(
            vec![
                app("org.unbound.Terminal", "Terminal", 6_420),
                app("org.unbound.Files", "Files", 1_180),
                app("org.unbound.Settings", "Settings", 240),
            ],
            icons,
        );
        Self { view, clock }
    }

    fn press(&mut self, key: Key) -> Vec<Effect> {
        let (_, effects) = self.view.handle(&Event::Key {
            key,
            modifiers: Modifiers::default(),
        });
        effects.into_iter().map(|(effect, _)| effect).collect()
    }

    fn check(&mut self, name: &str) {
        let (width, height) = LOW_MEMORY_SIZE;
        self.clock.set(self.clock.get() + SETTLED);
        self.view.scene(width, height);
        self.clock.set(self.clock.get() + SETTLED);
        let scene = self.view.scene(width, height);
        let frame = CpuRenderer::new().render(&scene).expect("frame");
        unbound_golden::assert_golden_rgba!(name, (width, height), frame.rgba);
    }
}

#[test]
fn the_low_memory_dialog_lists_the_heaviest_apps_and_asks_before_a_force_quit() {
    let mut light = Dialog::new(Scheme::Light);
    assert!(light.press(Key::Enter).is_empty());
    assert!(!light.view.ui().has_sheet());
    light.view.select(Some("org.unbound.Terminal".to_owned()));
    light.check("low-memory-light");

    let mut dark = Dialog::new(Scheme::Dark);
    dark.view.select(Some("org.unbound.Terminal".to_owned()));
    assert!(dark.press(Key::Enter).is_empty());
    assert!(dark.view.ui().has_sheet());
    dark.check("low-memory-confirm-dark");

    assert!(dark.press(Key::Escape).is_empty());
    assert!(!dark.view.ui().has_sheet());
    assert_eq!(dark.press(Key::Escape), [Effect::Close]);
}
