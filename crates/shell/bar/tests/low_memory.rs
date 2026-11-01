#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::Cell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bande_bar::low_memory::{Effect, LOW_MEMORY_SIZE, LowMemoryView};
use bande_status::memory::AppMemory;
use bande_toolkit::ui::{Event, Key, Modifiers};
use bande_toolkit::{CpuRenderer, Image, Palette, Scheme, TextSystem};

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
        units: vec![format!("app-bande-{id}-1.scope")],
        bytes: megabytes * 1_000_000,
    }
}

struct Dialog {
    view: LowMemoryView,
    clock: Rc<Cell<Instant>>,
}

impl Dialog {
    fn new(scheme: Scheme) -> Self {
        let disc = repository_icon(
            "../data/icons/Bande/scalable/apps/org.bande.ActivityMonitor.svg",
            36,
        );
        let mut view = LowMemoryView::new(
            Palette::for_scheme(scheme),
            TextSystem::new(),
            Some(disc),
            true,
        );
        let clock = Rc::new(Cell::new(Instant::now()));
        let now = Rc::clone(&clock);
        view.ui().set_clock(move || now.get());
        let icons = HashMap::from([
            (
                "org.bande.Terminal".to_owned(),
                repository_icon(
                    "../data/icons/Bande/scalable/apps/org.bande.Terminal.svg",
                    36,
                ),
            ),
            (
                "org.bande.Files".to_owned(),
                repository_icon("apps/files/data/org.bande.Files.svg", 36),
            ),
            (
                "org.bande.Settings".to_owned(),
                repository_icon("apps/settings/data/org.bande.Settings.svg", 36),
            ),
        ]);
        view.set_apps(
            vec![
                app("org.bande.Terminal", "Terminal", 6_420),
                app("org.bande.Files", "Files", 1_180),
                app("org.bande.Settings", "Settings", 240),
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
        bande_golden::assert_golden_rgba!(name, (width, height), frame.rgba);
    }
}

#[test]
fn the_low_memory_dialog_lists_the_heaviest_apps_and_asks_before_a_force_quit() {
    let mut light = Dialog::new(Scheme::Light);
    assert!(light.press(Key::Enter).is_empty());
    assert!(!light.view.ui().has_sheet());
    light.view.select(Some("org.bande.Terminal".to_owned()));
    light.check("low-memory-light");

    let mut dark = Dialog::new(Scheme::Dark);
    dark.view.select(Some("org.bande.Terminal".to_owned()));
    assert!(dark.press(Key::Enter).is_empty());
    assert!(dark.view.ui().has_sheet());
    dark.check("low-memory-confirm-dark");

    assert!(dark.press(Key::Escape).is_empty());
    assert!(!dark.view.ui().has_sheet());
    assert_eq!(dark.press(Key::Escape), [Effect::Close]);
}

#[test]
fn without_activity_monitor_the_low_memory_dialog_offers_no_way_to_open_it() {
    let labels = |monitor: bool| {
        let mut view = LowMemoryView::new(
            Palette::for_scheme(Scheme::Light),
            TextSystem::new(),
            None,
            monitor,
        );
        view.scene(LOW_MEMORY_SIZE.0, LOW_MEMORY_SIZE.1);
        view.ui()
            .accessibility()
            .nodes
            .iter()
            .filter_map(|(_, node)| node.label().map(str::to_owned))
            .collect::<Vec<_>>()
    };

    assert!(labels(true).iter().any(|label| label == "Activity Monitor"));
    assert!(
        !labels(false)
            .iter()
            .any(|label| label == "Activity Monitor")
    );
}
