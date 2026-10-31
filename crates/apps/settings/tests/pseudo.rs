mod support;

use std::path::PathBuf;

use support::SIZE;
use unbound_settings::{PaneId, SettingsView, sidebar_width};
use unbound_toolkit::{
    FrameRegions, HEADER_HEIGHT, Palette, Scheme, TextSize, TextSystem, framed_scene,
};

const PANES: [PaneId; 25] = [
    PaneId::Wifi,
    PaneId::Bluetooth,
    PaneId::Network,
    PaneId::Appearance,
    PaneId::Bar,
    PaneId::Notifications,
    PaneId::Accessibility,
    PaneId::Displays,
    PaneId::NightLight,
    PaneId::Sound,
    PaneId::Keyboard,
    PaneId::KeyboardShortcuts,
    PaneId::TrackpadMouse,
    PaneId::Power,
    PaneId::PrivacySecurity,
    PaneId::Camera,
    PaneId::Microphone,
    PaneId::Screenshots,
    PaneId::ScreenSharing,
    PaneId::Location,
    PaneId::Apps,
    PaneId::LanguageTime,
    PaneId::About,
    PaneId::Storage,
    PaneId::SoftwareUpdate,
];
const DATA: [&str; 3] = ["Settings", "Unbound", "Debian"];

#[test]
fn every_pane_fits_longer_translations_and_has_no_hard_coded_text() {
    unbound_golden::use_pseudo_locale();
    let mut problems = Vec::new();
    for pane in PANES {
        let palette = Palette::for_scheme(Scheme::Light);
        let mut view = SettingsView::new(palette.clone(), TextSystem::new());
        view.set_icon_dirs(vec![
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data"),
        ]);
        view.select_pane(pane);
        let content = view.scene(SIZE.0, SIZE.1 - HEADER_HEIGHT);
        let scene = framed_scene(
            &content,
            "Settings",
            Some(FrameRegions {
                sidebar: sidebar_width(TextSize::DEFAULT),
                toolbar: 0.0,
            }),
            None,
            &palette,
        );
        let name = format!("settings-{}", pane.keyword());
        problems.extend(unbound_golden::pseudo_problems!(&name, &scene, &DATA));
    }
    assert!(problems.is_empty(), "{problems:#?}");
}
