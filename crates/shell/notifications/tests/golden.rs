#![allow(clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use bande_datetime::{DateFormats, posix_locale};
use jiff::civil::date;

use bande_notifications::{Screen, Sender};
use bande_notify::{Notification, Picture, Presentation, Stack};
use bande_toolkit::{
    Color, CpuRenderer, DisplayEdge, Image, Palette, Rect, Scene, Scheme, TextSystem,
};

const SIZE: (u32, u32) = (420, 460);
const SETTLED: Duration = Duration::from_secs(2);
const DESKTOP: Color = Color::hex(0x7C8698);

fn notification(id: u32, app: &str, summary: &str, body: &str) -> Notification {
    Notification {
        id,
        app_name: app.to_owned(),
        summary: summary.to_owned(),
        body: body.to_owned(),
        ..Notification::default()
    }
}

fn stack(group: &str, notifications: Vec<Notification>) -> Stack {
    Stack {
        group: group.to_owned(),
        notifications,
    }
}

fn stacks() -> Vec<Stack> {
    let mut reply = notification(
        3,
        "Mail",
        "Anna Berg",
        "Draft of the bar spec attached. Could you look at the panel section before Friday?",
    );
    reply.actions = vec![
        ("reply".to_owned(), "Reply".to_owned()),
        ("archive".to_owned(), "Archive".to_owned()),
    ];
    vec![
        stack(
            "files",
            vec![notification(
                1,
                "Files",
                "Copy finished",
                "1,204 items copied to “Field recordings”",
            )],
        ),
        stack(
            "mail",
            vec![reply, notification(2, "Mail", "Jonas", "Lunch?")],
        ),
    ]
}

fn screen(presentation: Presentation, now: Instant) -> Screen {
    let mut screen = Screen::new(
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
        Box::new(|notification: &Notification, _: &str| Sender {
            name: notification.app_name.clone(),
            icon: None,
            picture: match &notification.picture {
                Some(Picture::Pixels {
                    width,
                    height,
                    rgba,
                }) => Image::from_rgba(*width, *height, rgba.clone()),
                _ => None,
            },
        }),
        DateFormats::for_locale(&posix_locale("en_GB")),
    );
    screen.set_size(SIZE.0, SIZE.1);
    screen.present(presentation, now);
    screen
}

fn check_golden(name: &str, mut scene: Scene) {
    scene.background = DESKTOP;
    let frame = CpuRenderer::new().render(&scene).expect("frame");
    bande_golden::assert_golden_rgba!(name, (frame.width, frame.height), frame.rgba);
}

#[test]
fn banners_match_their_golden() {
    let now = Instant::now();
    let presentation = Presentation {
        banners: stacks(),
        center: Vec::new(),
        ..Presentation::default()
    };
    let mut screen = screen(presentation, now);
    check_golden("banners-light", screen.scene(now + SETTLED));
}

#[test]
fn banners_beside_a_left_bar_match_their_golden() {
    let now = Instant::now();
    let presentation = Presentation {
        banners: stacks(),
        center: Vec::new(),
        ..Presentation::default()
    };
    let mut screen = screen(presentation, now);
    let clock = Rect::new(-64.0, SIZE.1 as f32 - 60.0, 64.0, 52.0);
    screen.set_anchor(DisplayEdge::Left, clock, now);
    check_golden("banners-left-light", screen.scene(now + SETTLED));
}

#[test]
fn the_center_matches_its_golden() {
    let now = Instant::now();
    let presentation = Presentation {
        banners: Vec::new(),
        center: stacks(),
        ..Presentation::default()
    };
    let mut screen = screen(presentation, now);
    screen.set_today(date(2026, 10, 31), now);
    screen.open(now);
    check_golden("center-light", screen.scene(now + SETTLED));
}

fn landscape_photo(width: u32, height: u32) -> Picture {
    let rgba = (0..height)
        .flat_map(|y| {
            (0..width).flat_map(move |x| {
                let sky = y < height / 2;
                let sun = (x as i32 - width as i32 / 2).pow(2)
                    + (y as i32 - height as i32 / 3).pow(2)
                    < (height as i32 / 6).pow(2);
                match (sun, sky) {
                    (true, _) => [255, 196, 64, 255],
                    (false, true) => [96, 160, 230, 255],
                    (false, false) => [64, 140, (x * 255 / width) as u8, 255],
                }
            })
        })
        .collect();
    Picture::Pixels {
        width,
        height,
        rgba,
    }
}

#[test]
fn a_banner_with_a_picture_matches_its_golden() {
    let now = Instant::now();
    let mut photo = notification(
        7,
        "Messages",
        "Lena Holm",
        "Look at the view from the cabin this morning, the lake is completely still",
    );
    photo.picture = Some(landscape_photo(96, 64));
    let presentation = Presentation {
        banners: vec![stack("messages", vec![photo])],
        ..Presentation::default()
    };
    let mut screen = screen(presentation, now);
    check_golden("banner-picture-light", screen.scene(now + SETTLED));
}
