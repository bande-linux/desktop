#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use unbound_controlcenter::media::{Playback, Player};
use unbound_controlcenter::{Changes, Choice, PANEL_WIDTH, Status, panel};
use unbound_toolkit::{CpuRenderer, Image, Palette, Scheme, TextSystem};

const MIN_SSIM: f64 = 0.99;

fn check_golden(name: &str, width: u32, height: u32, rgba: Vec<u8>) {
    let actual = image::RgbaImage::from_raw(width, height, rgba).expect("frame");
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

fn cover() -> Image {
    let size = 64;
    let rgba = (0..size * size)
        .flat_map(|index| {
            let (x, y) = (index % size, index / size);
            [
                (40 + x * 3) as u8,
                (90 + y * 2) as u8,
                (200 - x - y) as u8,
                255,
            ]
        })
        .collect();
    Image::from_rgba(size, size, rgba).expect("cover")
}

fn choice(key: &str, title: &str) -> Choice {
    Choice {
        key: key.to_owned(),
        title: title.to_owned(),
    }
}

fn status() -> Status {
    Status {
        wifi: Some(true),
        network: Some("Home".to_owned()),
        bluetooth: Some(true),
        volume: Some(0.6),
        outputs: vec![
            choice("speakers", "MacBook Pro Speakers"),
            choice("hdmi", "Television"),
        ],
        output: Some("speakers".to_owned()),
        brightness: Some(0.7),
        keyboard_brightness: Some(0.4),
        inputs: vec![
            choice("mic", "MacBook Pro Microphone"),
            choice("usb", "Studio Headset"),
        ],
        input: Some("mic".to_owned()),
        input_muted: Some(false),
        input_level: Rc::new(Cell::new(0.4)),
        player: Some(Player {
            bus_name: "org.mpris.MediaPlayer2.tunes".to_owned(),
            identity: "Tunes".to_owned(),
            desktop_entry: None,
            title: "Harbour Lights at Dusk".to_owned(),
            artist: "The Quiet Engines".to_owned(),
            art_url: None,
            track: None,
            playback: Playback::Paused,
            can_play: true,
            can_pause: true,
            can_go_next: true,
            can_go_previous: true,
            can_seek: true,
            can_raise: true,
            position: Some(Duration::from_secs(83)),
            length: Some(Duration::from_secs(245)),
            rate: 1.0,
            sampled: Instant::now(),
        }),
        artwork: Some(cover()),
        players: vec![
            choice("org.mpris.MediaPlayer2.tunes", "Tunes"),
            choice("org.mpris.MediaPlayer2.video", "Video"),
        ],
        ..Status::default()
    }
}

#[test]
fn control_center_with_now_playing_matches_its_golden_in_light_and_dark() {
    for (scheme, name) in [(Scheme::Light, "panel-light"), (Scheme::Dark, "panel-dark")] {
        let (mut ui, height) = panel(
            &status(),
            Palette::for_scheme(scheme),
            TextSystem::new(),
            &Changes::default(),
        );
        let (width, height) = (PANEL_WIDTH as u32, height as u32);
        let scene = ui.scene(width, height);
        let frame = CpuRenderer::new().render(&scene).expect("frame");
        check_golden(name, width, height, frame.rgba);
    }
}
