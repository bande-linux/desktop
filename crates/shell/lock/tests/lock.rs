#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use unbound_datetime::{DateFormats, posix_locale};
use unbound_lock::{LockScreen, SHAKE_DURATION, User, clock_texts, display_name, shake_offset};
use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{Appearance, CpuRenderer, Image, Scheme, TextSystem};

const OUTPUT: (u32, u32) = (1024, 768);
const MIN_SSIM: f64 = 0.99;
const NONE: Modifiers = Modifiers {
    shift: false,
    alt: false,
    control: false,
    logo: false,
};

fn morning() -> jiff::Zoned {
    "2026-10-28T09:41:00[UTC]".parse().expect("a zoned time")
}

fn ada() -> User {
    User {
        uid: 1000,
        login: "ada".to_owned(),
        full_name: "Ada Lovelace".to_owned(),
    }
}

fn british() -> DateFormats {
    DateFormats::for_locale(&posix_locale("en_GB.UTF-8"))
}

fn screen() -> LockScreen {
    LockScreen::new(
        &ada(),
        british(),
        &morning(),
        Appearance::default(),
        TextSystem::new(),
    )
}

fn type_text(screen: &mut LockScreen, text: &str) {
    for character in text.chars() {
        screen.handle(&Event::Key {
            key: Key::Character(character),
            modifiers: NONE,
        });
    }
}

fn press_return(screen: &mut LockScreen) {
    screen.handle(&Event::Key {
        key: Key::Enter,
        modifiers: NONE,
    });
}

fn check_golden(name: &str, size: (u32, u32), rgba: Vec<u8>) {
    let actual = image::RgbaImage::from_raw(size.0, size.1, rgba).expect("frame");
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

fn wallpaper() -> Image {
    let (width, height) = (64, 48);
    let rgba = (0..width * height)
        .flat_map(|index| {
            let (x, y) = (index % width, index / width);
            [(150 + x) as u8, (170 + y) as u8, (230 - x / 2) as u8, 255]
        })
        .collect();
    Image::from_rgba(width, height, rgba).expect("wallpaper")
}

fn render(screen: &mut LockScreen, name: &str) {
    let scene = screen.scene(OUTPUT.0, OUTPUT.1);
    for (scale, suffix) in [(1.0, ""), (1.5, "-1.5x")] {
        let size = (
            (f64::from(OUTPUT.0) * scale).round() as u32,
            (f64::from(OUTPUT.1) * scale).round() as u32,
        );
        let mut rgba = vec![0; (size.0 * size.1 * 4) as usize];
        CpuRenderer::new()
            .render_into(&scene, scale, &mut rgba)
            .expect("frame");
        check_golden(&format!("{name}{suffix}"), size, rgba);
    }
}

#[test]
fn the_lock_screen_matches_its_goldens_in_light_and_dark_appearance() {
    for (scheme, name) in [
        (Scheme::Light, "lock-screen-light"),
        (Scheme::Dark, "lock-screen-dark"),
    ] {
        let mut screen = LockScreen::new(
            &ada(),
            british(),
            &morning(),
            Appearance {
                scheme,
                ..Appearance::default()
            },
            TextSystem::new(),
        );
        screen.set_backdrop(wallpaper(), Instant::now() - Duration::from_secs(5));
        screen.advance(Instant::now());
        screen.set_position(1, 3);
        render(&mut screen, name);
    }
}

#[test]
fn the_lock_screen_without_a_wallpaper_matches_its_golden() {
    render(&mut screen(), "lock-screen");
}

#[test]
fn several_people_show_where_the_chosen_one_is_and_say_so() {
    let mut screen = screen();
    let alone = window_label(&screen);
    screen.set_position(1, 3);
    let label = window_label(&screen);
    assert!(!alone.contains("of 3"), "{alone}");
    assert!(label.contains("User 2 of 3"), "{label}");
}

#[test]
fn the_clock_shows_the_weekday_and_date_above_the_time() {
    assert_eq!(
        clock_texts(&british(), &morning()),
        ("Wednesday 28 October".to_owned(), "09:41".to_owned())
    );
}

#[test]
fn the_full_name_comes_from_the_first_gecos_field_or_falls_back_to_the_login() {
    assert_eq!(display_name("ada", "Ada Lovelace,,,"), "Ada Lovelace");
    assert_eq!(display_name("ada", ""), "ada");
    assert_eq!(display_name("ada", " ,room 1"), "ada");
}

#[test]
fn return_submits_the_password_once_and_typing_waits_for_the_verdict() {
    let mut screen = screen();
    type_text(&mut screen, "secret");
    press_return(&mut screen);
    assert_eq!(screen.take_password().as_deref(), Some("secret"));

    type_text(&mut screen, "more");
    press_return(&mut screen);
    assert_eq!(screen.take_password(), None);
}

#[test]
fn an_empty_password_is_not_submitted() {
    let mut screen = screen();
    press_return(&mut screen);
    assert_eq!(screen.take_password(), None);
}

#[test]
fn a_rejected_password_is_cleared_and_the_field_shakes_back_into_place() {
    let mut screen = screen();
    let resting = screen.field_frame(OUTPUT.0, OUTPUT.1);
    type_text(&mut screen, "wrong");
    press_return(&mut screen);
    screen.take_password();
    let rejected = Instant::now();
    screen.reject(rejected);

    let mut moved = false;
    for step in 1..12 {
        screen.advance(rejected + SHAKE_DURATION * step / 12);
        let frame = screen.field_frame(OUTPUT.0, OUTPUT.1);
        assert_eq!(frame.y, resting.y);
        moved |= frame.x != resting.x;
    }
    assert!(moved);
    screen.advance(rejected + SHAKE_DURATION);
    assert_eq!(screen.field_frame(OUTPUT.0, OUTPUT.1), resting);
    assert_eq!(screen.shaking_until(), None);

    type_text(&mut screen, "right");
    press_return(&mut screen);
    assert_eq!(screen.take_password().as_deref(), Some("right"));
}

#[test]
fn the_shake_swings_both_ways_and_dies_down() {
    let samples: Vec<f32> = (0..=48)
        .map(|step| shake_offset(SHAKE_DURATION * step / 48))
        .collect();
    assert_eq!(samples[0], 0.0);
    assert_eq!(*samples.last().expect("samples"), 0.0);
    assert!(samples.iter().any(|offset| *offset > 1.0));
    assert!(samples.iter().any(|offset| *offset < -1.0));
    assert!(samples.iter().all(|offset| offset.abs() <= 12.0));
    let early = samples[..12]
        .iter()
        .fold(0.0_f32, |max, x| max.max(x.abs()));
    let late = samples[36..]
        .iter()
        .fold(0.0_f32, |max, x| max.max(x.abs()));
    assert!(late < early);
    assert_eq!(shake_offset(SHAKE_DURATION + Duration::from_millis(1)), 0.0);
}

fn texts(screen: &mut LockScreen) -> usize {
    screen
        .scene(OUTPUT.0, OUTPUT.1)
        .items
        .iter()
        .filter(|item| matches!(item, unbound_toolkit::Item::Text { .. }))
        .count()
}

#[test]
fn the_active_input_source_is_named_under_the_field_when_there_are_several() {
    let mut screen = screen();
    let plain = texts(&mut screen);

    assert!(screen.set_input_source(Some("Swedish".to_owned())));
    assert!(!screen.set_input_source(Some("Swedish".to_owned())));
    assert_eq!(texts(&mut screen), plain + 1);
    let label = screen
        .accessibility()
        .nodes
        .iter()
        .find_map(|(_, node)| node.label().map(str::to_owned))
        .expect("labelled");
    assert!(label.contains("Swedish"), "{label}");

    assert!(screen.set_input_source(None));
    assert_eq!(texts(&mut screen), plain);
}

fn window_label(screen: &LockScreen) -> String {
    screen
        .accessibility()
        .nodes
        .iter()
        .find_map(|(_, node)| node.label().map(str::to_owned))
        .expect("labelled")
}

fn placeholder(screen: &LockScreen) -> String {
    screen
        .accessibility()
        .nodes
        .iter()
        .find_map(|(_, node)| node.placeholder().map(str::to_owned))
        .expect("a password field")
}

#[test]
fn caps_lock_shows_its_sign_in_the_field_and_says_so() {
    let mut screen = screen();
    let plain = texts(&mut screen);

    let field = |screen: &LockScreen| {
        screen
            .accessibility()
            .nodes
            .into_iter()
            .find(|(_, node)| node.placeholder().is_some())
            .and_then(|(_, node)| node.description().map(str::to_owned))
    };
    let plain_items = screen.scene(OUTPUT.0, OUTPUT.1).items;

    assert!(screen.set_caps_lock(true));
    assert!(!screen.set_caps_lock(true));
    assert_eq!(texts(&mut screen), plain);
    assert_ne!(screen.scene(OUTPUT.0, OUTPUT.1).items, plain_items);
    assert_eq!(field(&screen).as_deref(), Some("Caps Lock is on"));

    assert!(screen.set_caps_lock(false));
    assert_eq!(field(&screen), None);
}

#[test]
fn a_message_stays_until_escape_clears_it_with_the_typed_text() {
    let mut screen = screen();
    let plain = texts(&mut screen);
    screen.set_message(Some("Your password expires in 3 days".to_owned()));
    type_text(&mut screen, "half");
    assert_eq!(texts(&mut screen), plain + 1);
    assert!(window_label(&screen).contains("expires in 3 days"));

    assert!(screen.handle(&Event::Key {
        key: Key::Escape,
        modifiers: NONE,
    }));

    assert_eq!(texts(&mut screen), plain);
    type_text(&mut screen, "whole");
    press_return(&mut screen);
    assert_eq!(screen.take_password().as_deref(), Some("whole"));
}

#[test]
fn a_follow_up_prompt_names_the_field_and_takes_a_fresh_answer() {
    let mut screen = screen();
    type_text(&mut screen, "old");
    press_return(&mut screen);
    screen.take_password();

    screen.ask(Some("New password: "));
    assert_eq!(placeholder(&screen), "New password");
    type_text(&mut screen, "new");
    press_return(&mut screen);
    assert_eq!(screen.take_password().as_deref(), Some("new"));

    screen.ask(None);
    assert_eq!(placeholder(&screen), "Enter Password");
}
