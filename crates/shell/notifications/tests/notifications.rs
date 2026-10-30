#![allow(clippy::expect_used)]

use std::time::{Duration, Instant};

use jiff::Zoned;
use unbound_datetime::{DateFormats, posix_locale};
use unbound_notifications::{BANNER_WIDTH, CENTER_WIDTH, Request, Screen, Sender, age_label};
use unbound_notify::{Notification, Presentation, Stack};
use unbound_toolkit::ui::{Event, Key, Modifiers, Pointer};
use unbound_toolkit::{DisplayEdge, Palette, Point, Rect, Scheme, TextSystem};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 776;
const SETTLED: Duration = Duration::from_secs(2);

fn notification(id: u32, app: &str, summary: &str, actions: &[(&str, &str)]) -> Notification {
    Notification {
        id,
        app_name: app.to_owned(),
        summary: summary.to_owned(),
        body: "body".to_owned(),
        actions: actions
            .iter()
            .map(|(key, label)| ((*key).to_owned(), (*label).to_owned()))
            .collect(),
        ..Notification::default()
    }
}

fn stack(group: &str, notifications: Vec<Notification>) -> Stack {
    Stack {
        group: group.to_owned(),
        notifications,
    }
}

fn screen() -> Screen {
    let mut screen = Screen::new(
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
        Box::new(|notification: &Notification, _: &str| Sender {
            name: notification.app_name.clone(),
            icon: None,
        }),
    );
    screen.set_size(WIDTH, HEIGHT);
    screen
}

fn banners(stacks: Vec<Stack>) -> Presentation {
    Presentation {
        banners: stacks.clone(),
        center: stacks,
        do_not_disturb: false,
    }
}

fn region(screen: &Screen) -> Vec<Rect> {
    screen.input_region().expect("only the banners take input")
}

fn center_of(rect: Rect) -> Point {
    Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
}

fn pointer(event: fn(Pointer) -> Event, at: Point) -> Event {
    event(Pointer::at(at))
}

fn key(key: Key) -> Event {
    Event::Key {
        key,
        modifiers: Modifiers::default(),
    }
}

#[test]
fn a_banner_slides_in_from_the_right_edge_and_rests_at_the_top_right() {
    let start = Instant::now();
    let mut screen = screen();
    screen.present(
        banners(vec![stack(
            "mail",
            vec![notification(1, "Mail", "Hi", &[])],
        )]),
        start,
    );

    assert!(screen.visible(start));
    assert!(screen.animating(start));
    let resting = region(&screen)[0];
    assert!(resting.x + resting.width <= WIDTH as f32);
    assert!(resting.x + resting.width > WIDTH as f32 - 24.0);
    assert!(resting.y < 24.0);
    assert!(!screen.animating(start + SETTLED));
}

#[test]
fn stacks_of_different_apps_lie_below_each_other_newest_first() {
    let start = Instant::now();
    let mut screen = screen();
    screen.present(
        banners(vec![
            stack("chat", vec![notification(2, "Chat", "Yo", &[])]),
            stack(
                "mail",
                vec![
                    notification(3, "Mail", "Second", &[]),
                    notification(1, "Mail", "First", &[]),
                ],
            ),
        ]),
        start,
    );

    let placed = region(&screen);
    assert_eq!(placed.len(), 2, "one banner per app");
    assert!(placed[0].y + placed[0].height <= placed[1].y + 2.0);
}

#[test]
fn clicking_a_banner_opens_it_and_its_buttons_run_their_actions() {
    let now = Instant::now() + SETTLED;
    let mut screen = screen();
    screen.present(
        banners(vec![stack(
            "org.example.Mail",
            vec![notification(7, "Mail", "Hi", &[("reply", "Reply")])],
        )]),
        now - SETTLED,
    );
    let card = region(&screen)[0];
    let body = Point::new(card.x + 60.0, card.y + 24.0);

    screen.handle(&pointer(Event::PointerDown, body), now);
    screen.handle(&pointer(Event::PointerUp, body), now);
    let button = Point::new(card.x + 40.0, card.y + card.height - 20.0);
    screen.handle(&pointer(Event::PointerDown, button), now);
    screen.handle(&pointer(Event::PointerUp, button), now);

    assert_eq!(
        screen.take_requests(),
        [
            Request::Activate {
                id: 7,
                action: "default".to_owned(),
                group: "org.example.Mail".to_owned()
            },
            Request::Activate {
                id: 7,
                action: "reply".to_owned(),
                group: "org.example.Mail".to_owned()
            },
        ]
    );
}

#[test]
fn resting_the_pointer_on_a_banner_holds_it_and_leaving_lets_it_go() {
    let now = Instant::now() + SETTLED;
    let mut screen = screen();
    screen.present(
        banners(vec![stack(
            "mail",
            vec![notification(1, "Mail", "Hi", &[])],
        )]),
        now - SETTLED,
    );
    let card = region(&screen)[0];

    screen.handle(&pointer(Event::PointerMove, center_of(card)), now);
    screen.handle(&pointer(Event::PointerMove, center_of(card)), now);
    screen.handle(&Event::PointerLeave, now);

    assert_eq!(
        screen.take_requests(),
        [Request::HoldBanners(true), Request::HoldBanners(false)]
    );
}

#[test]
fn swiping_a_banner_right_puts_it_away_and_a_short_drag_springs_back() {
    let start = Instant::now() + SETTLED;
    let mut screen = screen();
    screen.present(
        banners(vec![stack(
            "mail",
            vec![notification(1, "Mail", "Hi", &[])],
        )]),
        start - SETTLED,
    );
    let card = region(&screen)[0];
    let from = center_of(card);

    screen.handle(&pointer(Event::PointerDown, from), start);
    let nudge = Point::new(from.x + 30.0, from.y);
    let later = start + Duration::from_millis(300);
    screen.handle(&pointer(Event::PointerMove, nudge), later);
    screen.handle(&pointer(Event::PointerUp, nudge), later);
    assert!(
        !screen
            .take_requests()
            .iter()
            .any(|request| matches!(request, Request::HideBanner(_) | Request::Activate { .. }))
    );
    assert_eq!(region(&screen)[0].x, card.x);

    let begin = later + SETTLED;
    screen.handle(&pointer(Event::PointerDown, from), begin);
    let far = Point::new(from.x + BANNER_WIDTH * 0.6, from.y);
    let end = begin + Duration::from_millis(400);
    screen.handle(&pointer(Event::PointerMove, far), end);
    screen.handle(&pointer(Event::PointerUp, far), end);
    assert!(screen.take_requests().contains(&Request::HideBanner(1)));
    assert!(region(&screen).is_empty());
}

#[test]
fn the_center_takes_the_whole_screen_while_open_and_escape_or_a_click_beside_it_closes_it() {
    let now = Instant::now();
    let mut screen = screen();
    screen.present(
        banners(vec![stack(
            "mail",
            vec![notification(1, "Mail", "Hi", &[])],
        )]),
        now,
    );

    screen.open(now);
    assert!(screen.wants_keyboard());
    assert_eq!(screen.input_region(), None);
    screen.handle(&key(Key::Escape), now);
    assert!(!screen.is_open());
    assert!(screen.visible(now), "it slides away first");
    assert!(!screen.visible(now + SETTLED) || !screen.animating(now + SETTLED));

    screen.open(now + SETTLED);
    let beside = Point::new(WIDTH as f32 - CENTER_WIDTH - 100.0, 300.0);
    screen.handle(&pointer(Event::PointerDown, beside), now + SETTLED);
    assert!(!screen.is_open());
}

#[test]
fn the_keyboard_walks_the_center_expands_a_stack_and_clears_it() {
    let now = Instant::now();
    let mut screen = screen();
    screen.present(
        Presentation {
            banners: Vec::new(),
            center: vec![stack(
                "mail",
                vec![
                    notification(3, "Mail", "Second", &[]),
                    notification(1, "Mail", "First", &[]),
                ],
            )],
            do_not_disturb: false,
        },
        now,
    );
    screen.open(now);

    screen.handle(&key(Key::Down), now);
    screen.handle(&key(Key::Down), now);
    screen.handle(&key(Key::Enter), now);
    assert!(screen.take_requests().is_empty(), "Enter expands the stack");

    screen.handle(&key(Key::Down), now);
    screen.handle(&key(Key::Down), now);
    screen.handle(&key(Key::Delete), now);
    screen.handle(&key(Key::Up), now);
    screen.handle(&key(Key::Enter), now);
    screen.handle(&key(Key::Up), now);
    screen.handle(&key(Key::Up), now);
    screen.handle(&key(Key::Enter), now);

    assert_eq!(
        screen.take_requests(),
        [
            Request::Dismiss(3),
            Request::Clear("mail".to_owned()),
            Request::ClearAll
        ]
    );
}

#[test]
fn a_notification_tells_its_age_then_the_day_then_the_date() {
    let dates = DateFormats::for_locale(&posix_locale("en_US.UTF-8"));
    let now: Zoned = "2026-10-30T14:05:00[Europe/Stockholm]"
        .parse()
        .expect("now");
    let age = |arrived: &str| {
        let arrived: Zoned = format!("{arrived}[Europe/Stockholm]")
            .parse()
            .expect("arrival");
        age_label(&arrived, &now, &dates)
    };

    assert_eq!(age("2026-10-30T14:04:30"), "now");
    assert_eq!(age("2026-10-30T13:50:00"), "15 min ago");
    assert_eq!(age("2026-10-30T09:00:00"), "5 hours ago");
    assert_eq!(age("2026-10-29T23:30:00"), "Yesterday");
    assert_eq!(age("2026-10-27T08:00:00"), "Tuesday");
    assert_eq!(age("2026-10-12T08:00:00"), "Oct 12");
    assert_eq!(age("2025-12-24T08:00:00"), "Dec 24, 2025");
}

#[test]
fn a_left_bar_opens_the_center_beside_its_clock_instead_of_at_the_right() {
    let now = Instant::now();
    let mut screen = screen();
    screen.present(
        banners(vec![stack(
            "mail",
            vec![notification(1, "Mail", "Hi", &[])],
        )]),
        now,
    );
    screen.set_anchor(DisplayEdge::Left, Rect::new(0.0, 724.0, 64.0, 52.0));
    screen.open(now);
    let inside = Point::new(64.0 + 12.0 + 40.0, 30.0);
    screen.handle(&pointer(Event::PointerDown, inside), now + SETTLED);
    assert!(screen.is_open());
    let right = Point::new(WIDTH as f32 - 40.0, 30.0);
    screen.handle(&pointer(Event::PointerDown, right), now + SETTLED);
    assert!(!screen.is_open());
}
