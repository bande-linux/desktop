use unbound_menubar::{MenuBar, app_name, clock_text};
use unbound_toolkit::{Item, Palette, Point, Scheme, TextSystem};

#[test]
fn the_clock_reads_like_the_macos_menu_bar() {
    let now: jiff::Zoned = "2026-10-29T09:05:00+02:00[+02:00]".parse().expect("time");
    assert_eq!(clock_text(&now), "Thu 29 Oct 09:05");
}

#[test]
fn the_active_app_is_named_by_its_desktop_entry() {
    let names = vec![("unbound-term".to_owned(), "Terminal".to_owned())];
    assert_eq!(app_name("unbound-term", &names), "Terminal");
    assert_eq!(
        app_name("org.example.Unknown", &names),
        "org.example.Unknown"
    );
    assert_eq!(app_name("", &names), "");
}

#[test]
fn the_app_name_is_drawn_at_the_left_and_the_clock_at_the_right() {
    let mut bar = MenuBar::new(
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
        "Thu 29 Oct 09:05".to_owned(),
    );
    assert!(bar.set_app("Terminal"));
    assert!(!bar.set_app("Terminal"));

    let scene = bar.scene(1024, 24);
    let texts = scene
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Text { origin, paragraph } => Some((origin.x, paragraph.width())),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [(app_x, _), (clock_x, clock_width)] = texts.as_slice() else {
        panic!("{texts:?}");
    };
    assert!(*app_x < 32.0);
    assert!(clock_x + clock_width <= 1024.0 && clock_x + clock_width > 990.0);
}

#[test]
fn control_center_sits_left_of_the_clock_and_takes_only_its_own_clicks() {
    let mut bar = MenuBar::new(
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
        "Thu 29 Oct 09:05".to_owned(),
    );
    let scene = bar.scene(1024, 24);
    let clock_x = scene
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Text { origin, .. } => Some(origin.x),
            _ => None,
        })
        .fold(f32::MIN, f32::max);

    assert!(bar.control_center_hit(Point::new(clock_x - 20.0, 12.0)));
    assert!(!bar.control_center_hit(Point::new(clock_x + 4.0, 12.0)));
    assert!(!bar.control_center_hit(Point::new(clock_x - 60.0, 12.0)));
}
