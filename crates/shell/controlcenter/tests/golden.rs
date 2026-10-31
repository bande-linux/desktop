#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bande_controlcenter::media::{Playback, Player};
use bande_controlcenter::{Changes, Choice, PANEL_WIDTH, Status, panel, panel_items};
use bande_toolkit::{Color, CpuRenderer, Image, Palette, Rect, Scene, Scheme, TextSystem};

const MARGIN: f32 = 24.0;
const DESKTOP: Color = Color::hex(0x7C8698);

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
        person: Some("Mikael Hug".to_owned()),
        icon_dirs: vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data")],
        ..Status::default()
    }
}

#[test]
fn control_center_with_now_playing_matches_its_golden() {
    let palette = Palette::for_scheme(Scheme::Light);
    let (mut ui, height) = panel(&status(), palette, TextSystem::new(), &Changes::default());
    let rect = Rect::new(MARGIN, MARGIN, PANEL_WIDTH, height);
    let scene = Scene {
        width: (PANEL_WIDTH + 2.0 * MARGIN) as u32,
        height: (height + 2.0 * MARGIN) as u32,
        background: DESKTOP,
        items: panel_items(&mut ui, &palette, rect),
    };
    let frame = CpuRenderer::new().render(&scene).expect("frame");
    bande_golden::assert_golden_rgba!("panel-light", (frame.width, frame.height), frame.rgba);
}
