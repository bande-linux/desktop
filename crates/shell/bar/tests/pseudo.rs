#![allow(clippy::expect_used)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use unbound_bar::dialogs::{CONFIRMATION_SIZE, session_ending_scene};
use unbound_bar::menu_bar::StatusView;
use unbound_bar::rail::{RailView, surface_size};
use unbound_datetime::{DateFormats, posix_locale};
use unbound_status::session::{Ending, SessionEnding};
use unbound_status::status::{Battery, Power, Sound, Wifi};
use unbound_toolkit::{Palette, Scene, Scheme, TextSystem};

const ICON: f32 = 40.0;
const HEIGHT: u32 = 720;
const DATA: [&str; 8] = [
    "Files", "Terminal", "File", "Edit", "Thursday", "October", "Thu", "PM",
];

fn data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data")
}

fn problems(name: &str, scene: &Scene) -> Vec<String> {
    unbound_golden::pseudo_problems!(name, scene, &DATA)
}

fn rail(expanded: bool) -> Scene {
    let mut view = RailView::new(
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
        vec![data()],
        ICON,
    );
    let status = view.status_mut();
    status.wifi = Some(Wifi {
        enabled: true,
        networks: Vec::new(),
        sign_in: false,
    });
    status.sound = Some(Sound {
        volume: 0.6,
        muted: false,
        outputs: Vec::new(),
        default_output: None,
    });
    status.battery = Some(Battery {
        percent: 84.0,
        power: Power::Battery { remaining: None },
    });
    view.set_app("Files");
    view.set_workspaces(3, 0);
    view.set_expanded(expanded);
    view.set_app_menus(["File", "Edit"].map(str::to_owned).to_vec());
    let at = "2026-10-29T14:32:00[America/New_York]"
        .parse()
        .expect("time");
    view.set_time(&at, &DateFormats::for_locale(&posix_locale("en_US")));
    view.scene(surface_size(ICON).0, HEIGHT, 1.0)
}

#[test]
fn the_bar_and_its_dialogs_fit_longer_translations_and_have_no_hard_coded_text() {
    unbound_golden::use_pseudo_locale();
    let start = Instant::now();
    let mut found = problems("bar", &rail(false));
    found.extend(problems("bar-expanded", &rail(true)));
    for (ending, name) in [
        (Ending::ShutDown, "shut-down"),
        (Ending::Restart, "restart"),
        (Ending::LogOut, "log-out"),
    ] {
        let mut session = SessionEnding::new(ending, start);
        let palette = Palette::for_scheme(Scheme::Light);
        let at = start + Duration::from_secs(5);
        found.extend(problems(
            name,
            &session_ending_scene(&session, palette.clone(), CONFIRMATION_SIZE, at),
        ));
        session.confirm(start);
        session.apps_running("Terminal".to_owned(), 2, start);
        found.extend(problems(
            &format!("{name}-blocked"),
            &session_ending_scene(&session, palette, CONFIRMATION_SIZE, start),
        ));
    }
    assert!(found.is_empty(), "{found:#?}");
}
