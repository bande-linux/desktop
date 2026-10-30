#![allow(clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use unbound_notifications::{Screen, Sender};
use unbound_notify::{Notification, Presentation, Stack};
use unbound_toolkit::{Color, CpuRenderer, Palette, Scene, Scheme, TextSystem};

const MIN_SSIM: f64 = 0.99;
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
        }),
    );
    screen.set_size(SIZE.0, SIZE.1);
    screen.present(presentation, now);
    screen
}

fn check_golden(name: &str, mut scene: Scene) {
    scene.background = DESKTOP;
    let frame = CpuRenderer::new().render(&scene).expect("frame");
    let actual = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba).expect("frame");
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

#[test]
fn banners_match_their_golden() {
    let now = Instant::now();
    let presentation = Presentation {
        banners: stacks(),
        center: Vec::new(),
        do_not_disturb: false,
    };
    let mut screen = screen(presentation, now);
    check_golden("banners-light", screen.scene(now + SETTLED));
}

#[test]
fn the_center_matches_its_golden() {
    let now = Instant::now();
    let presentation = Presentation {
        banners: Vec::new(),
        center: stacks(),
        do_not_disturb: false,
    };
    let mut screen = screen(presentation, now);
    screen.open(now);
    check_golden("center-light", screen.scene(now + SETTLED));
}
