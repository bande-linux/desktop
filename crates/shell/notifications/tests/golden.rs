#![allow(clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use unbound_notifications::{Screen, Sender};
use unbound_notify::{Notification, Presentation, Stack};
use unbound_toolkit::{Color, CpuRenderer, Palette, Scene, Scheme, TextSystem};

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
    unbound_golden::assert_golden!(
        name,
        &unbound_golden::from_rgba(frame.width, frame.height, frame.rgba)
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
