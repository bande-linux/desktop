#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::time::Instant;

use accesskit::{Node, NodeId, Role};
use bande_datetime::{DateFormats, posix_locale};
use bande_lock::{LockScreen, Peer, PowerAction, Prompt, Purpose, Refusal, User};
use bande_toolkit::ui::{Event, Key, Modifiers, Pointer};
use bande_toolkit::{Appearance, Item, Point, Rect, TextSize, TextSystem};

const DISPLAYS: [(u32, u32); 4] = [(1024, 600), (1280, 800), (1366, 768), (2560, 1440)];
const SCALES: [f64; 3] = [1.0, 1.25, 2.0];
const LONG_NAME: &str = "Wilhelmina Alexandrina Konstantinopolitanska-Oxenstierna von und zu Hohenlohe-Schillingsfürst-Waldenburg the Younger";
const POWERS: [PowerAction; 3] = [
    PowerAction::Sleep,
    PowerAction::Restart,
    PowerAction::ShutDown,
];

#[derive(Debug, Clone, Copy)]
enum Flow {
    Password,
    Wrong,
    Expired,
    Other,
    AskingToRestart,
}

const FLOWS: [Flow; 5] = [
    Flow::Password,
    Flow::Wrong,
    Flow::Expired,
    Flow::Other,
    Flow::AskingToRestart,
];

fn screen(name: &str, purpose: Purpose, appearance: Appearance, text: &TextSystem) -> LockScreen {
    let user = User {
        uid: 1000,
        login: "ada".to_owned(),
        full_name: name.to_owned(),
    };
    let mut screen = LockScreen::new(
        &user,
        DateFormats::for_locale(&posix_locale("en_GB.UTF-8")),
        &"2026-09-30T09:41:00[UTC]".parse().expect("a zoned time"),
        appearance,
        text.sharing_fonts(),
        purpose,
    );
    screen.set_input_source(Some("Svenska".to_owned()));
    screen
}

fn login_window(
    names: &[&str],
    flow: Flow,
    appearance: Appearance,
    text: &TextSystem,
) -> LockScreen {
    let other = matches!(flow, Flow::Other);
    let name = match other {
        true => "Other User",
        false => names[0],
    };
    let mut screen = screen(name, Purpose::Login, appearance, text);
    screen.offer_power(&POWERS);
    let peers = names.iter().copied().chain(["Other…"]).map(|name| Peer {
        name: name.to_owned(),
        initial: name.chars().take(1).collect(),
        picture: None,
    });
    screen.set_peers(peers.collect(), if other { names.len() } else { 0 });
    let now = Instant::now();
    match flow {
        Flow::Password | Flow::AskingToRestart => screen.ask(None),
        Flow::Wrong => {
            screen.refuse(Refusal::WrongAnswer, now);
            screen.refuse(Refusal::TooManyAttempts, now);
        }
        Flow::Expired => {
            screen.set_message(Some(Refusal::PasswordExpired.message()));
            screen.set_message(Some(
                "BAD PASSWORD: The password is too similar to the old one".to_owned(),
            ));
            screen.ask(Some(&Prompt {
                text: "Retype new password:".to_owned(),
                visible: false,
            }));
        }
        Flow::Other => {
            screen.set_message(Some(Refusal::Unknown.message()));
            screen.ask(Some(&Prompt {
                text: "User Name".to_owned(),
                visible: true,
            }));
        }
    }
    screen.advance(now + bande_lock::SHAKE_DURATION);
    screen
}

fn bounds(node: &Node) -> Rect {
    let bounds = node.bounds().expect("bounds");
    Rect::new(
        bounds.x0 as f32,
        bounds.y0 as f32,
        bounds.width() as f32,
        bounds.height() as f32,
    )
}

fn nodes(screen: &LockScreen, role: Role) -> Vec<(NodeId, Node)> {
    let nodes = screen.accessibility().nodes.into_iter();
    nodes
        .filter(|(_, node)| node.role() == role && !node.is_hidden() && bounds(node).width > 0.0)
        .collect()
}

fn contains(outer: Rect, inner: Rect) -> bool {
    inner.x >= outer.x - 1.0
        && inner.y >= outer.y - 1.0
        && inner.x + inner.width <= outer.x + outer.width + 1.0
        && inner.y + inner.height <= outer.y + outer.height + 1.0
}

fn overlap(a: Rect, b: Rect) -> bool {
    a.x + 1.0 < b.x + b.width
        && b.x + 1.0 < a.x + a.width
        && a.y + 1.0 < b.y + b.height
        && b.y + 1.0 < a.y + a.height
}

fn drawn(items: &[Item], found: &mut Vec<(String, Rect)>) {
    for item in items {
        match item {
            Item::Text { origin, paragraph } => {
                let extents = paragraph.line_extents();
                let left = extents
                    .iter()
                    .map(|extent| extent.0)
                    .fold(f32::MAX, f32::min);
                let rect = Rect::new(
                    origin.x + left,
                    origin.y,
                    paragraph.painted_width() - left,
                    paragraph.height(),
                );
                found.push((paragraph.text().to_owned(), rect));
            }
            Item::Fill { shape, .. }
                if shape.rect.width == shape.rect.height && shape.rect.width >= 40.0 =>
            {
                found.push((String::new(), shape.rect));
            }
            Item::Opacity { items, .. } => drawn(items, found),
            _ => {}
        }
    }
}

fn assert_laid_out(screen: &mut LockScreen, width: u32, height: u32, context: &str) -> Vec<String> {
    let scene = screen.scene(width, height);
    let display = Rect::new(0.0, 0.0, width as f32, height as f32);
    let peers = nodes(screen, Role::ListBoxOption);
    let mut boxes: Vec<(String, Rect)> = nodes(screen, Role::Button)
        .into_iter()
        .chain(peers)
        .chain(nodes(screen, Role::PasswordInput))
        .chain(nodes(screen, Role::TextInput))
        .map(|(_, node)| (format!("{:?}", node.role()), bounds(&node)))
        .collect();
    let mut found = Vec::new();
    drawn(&scene.items, &mut found);
    let mut texts = Vec::new();
    found.sort_by_key(|(text, _)| !text.is_empty());
    for (text, rect) in found {
        texts.push(text.clone());
        let one_letter = text.chars().count() == 1;
        if !one_letter && !boxes.iter().any(|(_, outer)| contains(*outer, rect)) {
            boxes.push((text, rect));
        }
    }
    for (index, (name, rect)) in boxes.iter().enumerate() {
        assert!(
            contains(display, *rect),
            "{context}: {name:?} {rect:?} leaves the screen"
        );
        for (other, beside) in &boxes[index + 1..] {
            assert!(
                !overlap(*rect, *beside),
                "{context}: {name:?} {rect:?} overlaps {other:?} {beside:?}"
            );
        }
    }
    texts
}

fn press_tab(screen: &mut LockScreen) {
    screen.handle(&Event::Key {
        key: Key::Tab,
        modifiers: Modifiers::default(),
    });
}

#[test]
fn nothing_overlaps_or_leaves_a_small_display_and_tab_reaches_everything() {
    let text = TextSystem::new();
    let many: Vec<&str> = [
        "Ada Lovelace",
        "Bo",
        "Eva",
        LONG_NAME,
        "Maximilianus",
        "Åsa",
    ]
    .into();
    let people: [&[&str]; 5] = [
        &many[..1],
        &many[..2],
        &many[..5],
        &many,
        &[LONG_NAME, "Bo"],
    ];
    for (pixels_wide, pixels_high) in DISPLAYS {
        for scale in SCALES {
            let width = (f64::from(pixels_wide) / scale) as u32;
            let height = (f64::from(pixels_high) / scale) as u32;
            for names in people {
                for flow in FLOWS {
                    let context = format!("{width}x{height}, {} people, {flow:?}", names.len());
                    let mut screen = login_window(names, flow, Appearance::default(), &text);
                    screen.set_scale(scale);
                    let texts = assert_laid_out(&mut screen, width, height, &context);
                    match flow {
                        Flow::Wrong | Flow::Other => assert!(
                            texts.iter().any(|text| text.ends_with("Try again.")
                                || text.ends_with("Wait a moment and try again.")),
                            "{context}: {texts:?}"
                        ),
                        Flow::Expired => assert!(
                            texts.iter().any(|text| text.starts_with("Your password")
                                && text.ends_with("old one")),
                            "{context}: {texts:?}"
                        ),
                        Flow::Password | Flow::AskingToRestart => {}
                    }
                    let start: String = names[0].chars().take(10).collect();
                    assert!(
                        matches!(flow, Flow::Other)
                            || texts.iter().any(|text| text.starts_with(&start)),
                        "{context}: {start} is not readable in {texts:?}"
                    );
                    if let Flow::AskingToRestart = flow {
                        let (_, restart) = nodes(&screen, Role::Button)
                            .into_iter()
                            .find(|(_, node)| node.label() == Some("Restart"))
                            .expect("a Restart button");
                        let middle = bounds(&restart);
                        let at = Pointer::at(Point::new(
                            middle.x + middle.width / 2.0,
                            middle.y + middle.height / 2.0,
                        ));
                        screen.handle(&Event::PointerDown(at));
                        screen.handle(&Event::PointerUp(at));
                        screen.take_power();
                        assert_laid_out(&mut screen, width, height, &context);
                    }
                    if !matches!(flow, Flow::Password) || names.len() < 5 {
                        continue;
                    }
                    let expected = 1 + 1 + names.len() + 1 + POWERS.len();
                    let mut reached = HashSet::new();
                    for _ in 0..expected {
                        let focused = screen.accessibility().focus;
                        let node = screen.accessibility().nodes.into_iter();
                        let visible = node
                            .filter(|(id, _)| *id == focused)
                            .all(|(_, node)| bounds(&node).width > 0.0);
                        assert!(visible, "{context}: the keyboard is on something not shown");
                        reached.insert(focused);
                        press_tab(&mut screen);
                        screen.scene(width, height);
                    }
                    assert_eq!(reached.len(), expected, "{context}");
                }
            }
            let mut locked = screen(LONG_NAME, Purpose::Lock, Appearance::default(), &text);
            locked.set_note("Found this computer? Please call the front desk at extension 1234, or bring it to the third floor.");
            for message in [None, Some(Refusal::WrongAnswer.message())] {
                locked.set_message(message);
                let context = format!("{width}x{height}, locked");
                assert_laid_out(&mut locked, width, height, &context);
            }
        }
    }
    let large = Appearance {
        text_size: TextSize::LARGEST,
        ..Appearance::default()
    };
    for (width, height) in [(1024, 600), (1280, 800), (2560, 1440)] {
        for flow in FLOWS {
            let context = format!("{width}x{height}, largest text, {flow:?}");
            let mut screen = login_window(&many, flow, large, &text);
            assert_laid_out(&mut screen, width, height, &context);
        }
    }
}
