#![allow(clippy::expect_used)]

use std::time::{Duration, Instant};

use bande_datetime::{DateFormats, posix_locale};
use bande_notifications::{BANNER_WIDTH, CENTER_WIDTH, Request, Screen, Sender, age_label};
use bande_notify::{Notification, Presentation, Stack};
use bande_toolkit::ui::{Event, Key, Modifiers, Pointer};
use bande_toolkit::{DisplayEdge, Item, Palette, Point, Rect, Scene, Scheme, TextSystem};
use jiff::Zoned;
use jiff::civil::date;

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
            picture: None,
        }),
        DateFormats::for_locale(&posix_locale("en_US")),
    );
    screen.set_size(WIDTH, HEIGHT);
    screen
}

fn banners(stacks: Vec<Stack>) -> Presentation {
    Presentation {
        banners: stacks.clone(),
        center: stacks,
        ..Presentation::default()
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

    screen.set_anchor(
        DisplayEdge::Top,
        Rect::new(WIDTH as f32 - 120.0, -24.0, 100.0, 24.0),
        start,
    );
    let under_the_clock = region(&screen)[0];
    assert!(under_the_clock.x + under_the_clock.width <= WIDTH as f32 - 20.0 + 1.0);
    assert!(under_the_clock.x + under_the_clock.width > WIDTH as f32 - 40.0);
    assert!(under_the_clock.y < 24.0);
}

const LEFT_CLOCK: Rect = Rect {
    x: -64.0,
    y: 700.0,
    width: 64.0,
    height: 52.0,
};

fn two_stacks() -> Presentation {
    banners(vec![
        stack("chat", vec![notification(2, "Chat", "Yo", &[])]),
        stack("mail", vec![notification(1, "Mail", "Hi", &[])]),
    ])
}

#[test]
fn a_left_bar_shows_banners_beside_it_with_the_newest_nearest_the_clock() {
    let start = Instant::now();
    let mut screen = screen();
    screen.set_anchor(DisplayEdge::Left, LEFT_CLOCK, start);
    screen.present(two_stacks(), start);

    let placed = region(&screen);
    let newest = placed[0];
    let older = placed[1];
    let clock_bottom = LEFT_CLOCK.y + LEFT_CLOCK.height;
    assert!(
        placed
            .iter()
            .all(|rect| rect.x + rect.width > 8.0 + BANNER_WIDTH - 1.0
                && rect.x + rect.width < 8.0 + BANNER_WIDTH + 1.0)
    );
    assert!(newest.y > older.y, "the newest lies lowest");
    assert!((newest.y + newest.height - clock_bottom).abs() < 1.0);
    let entering = leftmost(&screen.scene(start).items);
    let rested = leftmost(&screen.scene(start + SETTLED).items);
    assert!(entering < rested, "it slides out from under the bar");
}

#[test]
fn a_bottom_bar_stacks_banners_up_from_its_clock_at_the_right() {
    let start = Instant::now();
    let mut screen = screen();
    let clock = Rect::new(WIDTH as f32 - 90.0, HEIGHT as f32 + 8.0, 80.0, 48.0);
    screen.set_anchor(DisplayEdge::Bottom, clock, start);
    screen.present(two_stacks(), start);

    let placed = region(&screen);
    assert!((placed[0].x + placed[0].width - (clock.x + clock.width)).abs() < 1.0);
    assert!(placed[0].y > placed[1].y);
    assert!(placed[0].y + placed[0].height <= HEIGHT as f32);
    assert!(placed[0].y + placed[0].height > HEIGHT as f32 - 24.0);
}

#[test]
fn swiping_a_banner_towards_a_left_bar_puts_it_away_and_away_from_it_springs_back() {
    let start = Instant::now() + SETTLED;
    let mut screen = screen();
    screen.set_anchor(DisplayEdge::Left, LEFT_CLOCK, start - SETTLED);
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
    let wrong_way = Point::new(from.x + BANNER_WIDTH * 0.6, from.y);
    let later = start + Duration::from_millis(400);
    screen.handle(&pointer(Event::PointerMove, wrong_way), later);
    screen.handle(&pointer(Event::PointerUp, wrong_way), later);
    assert!(!screen.take_requests().contains(&Request::HideBanner(1)));
    assert_eq!(region(&screen)[0].x, card.x);

    let begin = later + SETTLED;
    screen.handle(&pointer(Event::PointerDown, from), begin);
    let far = Point::new(from.x - BANNER_WIDTH * 0.6, from.y);
    let end = begin + Duration::from_millis(400);
    screen.handle(&pointer(Event::PointerMove, far), end);
    screen.handle(&pointer(Event::PointerUp, far), end);
    assert!(screen.take_requests().contains(&Request::HideBanner(1)));
    assert!(region(&screen).is_empty());
}

#[test]
fn a_banner_that_times_out_on_a_left_bar_leaves_towards_it() {
    let start = Instant::now();
    let mut screen = screen();
    screen.set_anchor(DisplayEdge::Left, LEFT_CLOCK, start);
    screen.present(
        banners(vec![stack(
            "mail",
            vec![notification(1, "Mail", "Hi", &[])],
        )]),
        start,
    );
    let rested = start + SETTLED;
    let resting = screen.scene(rested);
    screen.present(Presentation::default(), rested);
    let leaving = screen.scene(rested + Duration::from_millis(120));
    let left_of = |scene: &Scene| leftmost(&scene.items);
    assert!(left_of(&leaving) < left_of(&resting) - 4.0);
}

fn leftmost(items: &[Item]) -> f32 {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Opacity { items, .. } => Some(leftmost(items)),
            Item::Translate { offset, items } => Some(offset.x + leftmost(items)),
            Item::Fill { shape, .. } => Some(shape.rect.x),
            _ => None,
        })
        .fold(f32::MAX, f32::min)
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
            ..Presentation::default()
        },
        now,
    );
    screen.open(now);

    for _ in 0..4 {
        screen.handle(&key(Key::Down), now);
    }
    screen.handle(&key(Key::Enter), now);
    assert!(screen.take_requests().is_empty(), "Enter expands the stack");

    screen.handle(&key(Key::Down), now);
    screen.handle(&key(Key::Down), now);
    screen.handle(&key(Key::Delete), now);
    screen.handle(&key(Key::Up), now);
    screen.handle(&key(Key::Enter), now);
    for _ in 0..4 {
        screen.handle(&key(Key::Up), now);
    }
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
    screen.set_anchor(DisplayEdge::Left, Rect::new(0.0, 724.0, 64.0, 52.0), now);
    screen.open(now);
    let inside = Point::new(64.0 + 8.0 + 40.0, 30.0);
    screen.handle(&pointer(Event::PointerDown, inside), now + SETTLED);
    assert!(screen.is_open());
    let right = Point::new(WIDTH as f32 - 40.0, 30.0);
    screen.handle(&pointer(Event::PointerDown, right), now + SETTLED);
    assert!(!screen.is_open());
}

#[test]
fn the_calendar_opens_on_today_and_the_keyboard_turns_its_months() {
    let now = Instant::now();
    let mut screen = screen();
    screen.set_today(date(2026, 10, 31), now);
    screen.open(now);
    assert_eq!(screen.shown_month(), date(2026, 10, 1));

    screen.handle(&key(Key::PageDown), now);
    assert_eq!(screen.shown_month(), date(2026, 11, 1));
    screen.handle(&key(Key::PageUp), now);
    screen.handle(&key(Key::PageUp), now);
    assert_eq!(screen.shown_month(), date(2026, 9, 1));
    screen.handle(&key(Key::Home), now);
    assert_eq!(screen.shown_month(), date(2026, 10, 1));

    screen.handle(&key(Key::Tab), now);
    screen.handle(&key(Key::Tab), now);
    screen.handle(&key(Key::Enter), now);
    screen.handle(&key(Key::Enter), now);
    assert_eq!(screen.shown_month(), date(2026, 12, 1));
    screen.handle(&key(Key::Tab), now);
    screen.handle(&key(Key::Space), now);
    assert_eq!(screen.shown_month(), date(2026, 11, 1));

    screen.close(now);
    screen.open(now);
    assert_eq!(screen.shown_month(), date(2026, 10, 1));
}

#[test]
fn the_calendar_follows_midnight_unless_another_month_is_shown() {
    let now = Instant::now();
    let mut screen = screen();
    screen.set_today(date(2026, 10, 31), now);
    screen.open(now);

    screen.set_today(date(2026, 11, 1), now);
    assert_eq!(screen.shown_month(), date(2026, 11, 1));

    screen.handle(&key(Key::PageUp), now);
    screen.handle(&key(Key::PageUp), now);
    screen.set_today(date(2026, 11, 2), now);
    assert_eq!(screen.shown_month(), date(2026, 9, 1));
}

#[test]
fn banners_go_quiet_only_while_the_display_in_use_shows_a_full_screen_app() {
    let mut screen = screen();
    screen.set_active_output("eDP-1");

    screen.set_full_screen("HDMI-A-1", true);
    assert!(screen.take_requests().is_empty());

    screen.set_full_screen("eDP-1", true);
    assert_eq!(screen.take_requests(), [Request::QuietForFullScreen(true)]);

    screen.set_active_output("HDMI-A-1");
    assert!(screen.take_requests().is_empty());

    screen.set_full_screen("HDMI-A-1", false);
    assert_eq!(screen.take_requests(), [Request::QuietForFullScreen(false)]);

    screen.set_active_output("eDP-1");
    assert_eq!(screen.take_requests(), [Request::QuietForFullScreen(true)]);
}

#[test]
fn control_center_opening_closes_the_center_and_puts_the_banners_away_into_it() {
    let now = Instant::now();
    let mut screen = screen();
    screen.present(two_stacks(), now);
    assert_eq!(region(&screen).len(), 2);

    screen.make_way(now);
    assert!(region(&screen).is_empty());
    let requests = screen.take_requests();
    assert!(requests.contains(&Request::HideBanner(1)));
    assert!(requests.contains(&Request::HideBanner(2)));
    assert!(!requests.iter().any(|request| matches!(
        request,
        Request::Dismiss(_) | Request::Clear(_) | Request::ClearAll
    )));

    screen.open(now);
    assert!(screen.is_open());
    screen.make_way(now);
    assert!(!screen.is_open());
}
