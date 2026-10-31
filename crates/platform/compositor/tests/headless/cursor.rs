use std::fs;
use std::path::Path;
use std::time::Duration;

use crate::support::*;
use smithay::utils::{Point, Size};
use unbound_compositor::{
    BACKGROUND, Compositor, CursorTheme, Renderer, Snapshot, UNRESPONSIVE_AFTER,
};
use unbound_test_wayland::{BLUE, CursorShape, RED, ShmImage, TestClient};
use wayland_client::protocol::wl_shm::Format;

const POINTER: (i32, i32) = (100, 80);

struct Image {
    nominal: u32,
    side: u32,
    hotspot: (u32, u32),
    rgb: [u8; 3],
}

fn xcursor(images: &[Image]) -> Vec<u8> {
    let header = 16u32;
    let toc = 12 * images.len() as u32;
    let mut bytes = Vec::new();
    let put = |bytes: &mut Vec<u8>, word: u32| bytes.extend(word.to_le_bytes());
    bytes.extend(b"Xcur");
    put(&mut bytes, header);
    put(&mut bytes, 0x0001_0000);
    put(&mut bytes, images.len() as u32);
    let mut position = header + toc;
    for image in images {
        put(&mut bytes, 0xfffd_0002);
        put(&mut bytes, image.nominal);
        put(&mut bytes, position);
        position += 36 + image.side * image.side * 4;
    }
    for image in images {
        for word in [
            36,
            0xfffd_0002,
            image.nominal,
            1,
            image.side,
            image.side,
            image.hotspot.0,
            image.hotspot.1,
            0,
        ] {
            put(&mut bytes, word);
        }
        let [r, g, b] = image.rgb;
        for _ in 0..image.side * image.side {
            bytes.extend([b, g, r, 0xff]);
        }
    }
    bytes
}

fn install(root: &Path, theme: &str, index: &str, cursors: &[(&str, &[Image])]) {
    let directory = root.join(theme);
    fs::create_dir_all(directory.join("cursors")).expect("theme directory");
    fs::write(directory.join("index.theme"), index).expect("index.theme");
    for (name, images) in cursors {
        fs::write(directory.join("cursors").join(name), xcursor(images)).expect("cursor file");
    }
}

fn frame_with_pointer(theme: Option<CursorTheme>) -> Snapshot {
    let mut server = Compositor::new(Size::from((256, 192))).expect("headless compositor");
    if let Some(theme) = theme {
        server.use_cursor_theme(theme);
    }
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    server.move_pointer_to(Point::from((f64::from(POINTER.0), f64::from(POINTER.1))), 1);
    renderer
        .render(&mut server)
        .expect("frame with the pointer");
    renderer.snapshot(&server).expect("read back")
}

#[test]
fn the_pointer_is_the_themes_default_cursor_with_its_hotspot_on_the_pointer() {
    let root = tempfile::tempdir().expect("icon directory");
    let green = [0x20, 0xc0, 0x40];
    install(
        root.path(),
        "Fixture",
        "[Icon Theme]\nName=Fixture\n",
        &[(
            "default",
            &[Image {
                nominal: 24,
                side: 24,
                hotspot: (3, 5),
                rgb: green,
            }],
        )],
    );

    let frame = frame_with_pointer(Some(CursorTheme {
        search_path: vec![root.path().to_path_buf()],
        name: "Fixture".to_owned(),
        size: 24,
    }));

    let (x, y) = POINTER;
    assert_eq!(rgb_at(&frame, x - 3, y - 5), green);
    assert_eq!(rgb_at(&frame, x + 20, y + 18), green);
    assert_eq!(rgb_at(&frame, x - 4, y - 5), BACKGROUND);
    assert_eq!(rgb_at(&frame, x - 3, y - 6), BACKGROUND);
    assert_eq!(rgb_at(&frame, x + 21, y), BACKGROUND);
}

#[test]
fn a_cursor_missing_from_the_theme_comes_from_the_theme_it_inherits() {
    let root = tempfile::tempdir().expect("icon directory");
    let blue = [0x30, 0x50, 0xe0];
    install(
        root.path(),
        "Fixture",
        "[Icon Theme]\nName=Fixture\nInherits=Missing, Base\n",
        &[],
    );
    install(
        root.path(),
        "Base",
        "[Icon Theme]\nName=Base\n",
        &[(
            "left_ptr",
            &[Image {
                nominal: 24,
                side: 24,
                hotspot: (0, 0),
                rgb: blue,
            }],
        )],
    );

    let frame = frame_with_pointer(Some(CursorTheme {
        search_path: vec![root.path().to_path_buf()],
        name: "Fixture".to_owned(),
        size: 24,
    }));

    assert_eq!(rgb_at(&frame, POINTER.0 + 10, POINTER.1 + 10), blue);
}

#[test]
fn the_image_whose_size_is_nearest_the_requested_size_is_drawn() {
    let root = tempfile::tempdir().expect("icon directory");
    let (small, large) = ([0xe0, 0x30, 0x30], [0x30, 0x30, 0xe0]);
    install(
        root.path(),
        "Fixture",
        "[Icon Theme]\nName=Fixture\n",
        &[(
            "default",
            &[
                Image {
                    nominal: 16,
                    side: 16,
                    hotspot: (0, 0),
                    rgb: small,
                },
                Image {
                    nominal: 32,
                    side: 32,
                    hotspot: (0, 0),
                    rgb: large,
                },
            ],
        )],
    );

    let frame = frame_with_pointer(Some(CursorTheme {
        search_path: vec![root.path().to_path_buf()],
        name: "Fixture".to_owned(),
        size: 30,
    }));

    assert_eq!(rgb_at(&frame, POINTER.0, POINTER.1), large);
    assert_eq!(rgb_at(&frame, POINTER.0 + 30, POINTER.1 + 30), large);
}

#[test]
fn a_window_that_asks_for_the_text_shape_gets_the_themes_text_cursor() {
    let root = tempfile::tempdir().expect("icon directory");
    let (arrow, beam) = ([0x20, 0xc0, 0x40], [0xc0, 0x20, 0xa0]);
    let image = |rgb| Image {
        nominal: 24,
        side: 24,
        hotspot: (12, 12),
        rgb,
    };
    install(
        root.path(),
        "Fixture",
        "[Icon Theme]\nName=Fixture\n",
        &[("default", &[image(arrow)]), ("text", &[image(beam)])],
    );
    let mut server = Compositor::new(Size::from((256, 192))).expect("headless compositor");
    server.use_cursor_theme(CursorTheme {
        search_path: vec![root.path().to_path_buf()],
        name: "Fixture".to_owned(),
        size: 24,
    });
    let mut client = TestClient::connect(&mut server);
    client.use_seat(&mut server);
    client.show_window(&mut server, &xrgb(&solid(RED, (96, 96)), (96, 96)));
    let origin = server.windows()[0].location();
    let pointer = (origin.x + 48, origin.y + 48);
    server.move_pointer_to(Point::from((f64::from(pointer.0), f64::from(pointer.1))), 1);
    client.exchange(&mut server);

    client.set_cursor_shape(&mut server, CursorShape::Text);
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    renderer
        .render(&mut server)
        .expect("frame with the pointer");
    let frame = renderer.snapshot(&server).expect("read back");

    assert_eq!(client.protocol_error(), None);
    assert_eq!(rgb_at(&frame, pointer.0, pointer.1), beam);
}

#[test]
fn the_pointer_over_an_app_that_stopped_answering_is_the_themes_wait_cursor() {
    let root = tempfile::tempdir().expect("icon directory");
    let (arrow, wait) = ([0x20, 0xc0, 0x40], [0x30, 0x40, 0xd0]);
    let image = |rgb| Image {
        nominal: 24,
        side: 24,
        hotspot: (12, 12),
        rgb,
    };
    install(
        root.path(),
        "Fixture",
        "[Icon Theme]\nName=Fixture\n",
        &[("default", &[image(arrow)]), ("wait", &[image(wait)])],
    );
    let mut server = Compositor::new(Size::from((256, 192))).expect("headless compositor");
    server.use_cursor_theme(CursorTheme {
        search_path: vec![root.path().to_path_buf()],
        name: "Fixture".to_owned(),
        size: 24,
    });
    let mut client = TestClient::connect(&mut server);
    client.use_seat(&mut server);
    client.show_window(&mut server, &xrgb(&solid(RED, (96, 96)), (96, 96)));
    client.hang();
    let origin = server.windows()[0].location();
    let pointer = (origin.x + 48, origin.y + 48);
    server.move_pointer_to(Point::from((f64::from(pointer.0), f64::from(pointer.1))), 1);
    let start = Duration::from_secs(5);
    server.watchdog_tick(start);
    client.exchange(&mut server);
    server.watchdog_tick(start + UNRESPONSIVE_AFTER);

    let mut renderer = Renderer::new().expect("Vulkan renderer");
    renderer
        .render(&mut server)
        .expect("frame with the pointer");
    let frame = renderer.snapshot(&server).expect("read back");

    assert_eq!(rgb_at(&frame, pointer.0, pointer.1), wait);
}

#[test]
fn the_pointer_shows_what_a_drop_would_do() {
    let root = tempfile::tempdir().expect("icon directory");
    let (arrow, copy, refused) = ([0x20, 0xc0, 0x40], [0xc0, 0x20, 0xa0], [0x30, 0x40, 0xd0]);
    let image = |rgb| Image {
        nominal: 24,
        side: 24,
        hotspot: (0, 0),
        rgb,
    };
    install(
        root.path(),
        "Fixture",
        "[Icon Theme]\nName=Fixture\n",
        &[
            ("default", &[image(arrow)]),
            ("move", &[image(arrow)]),
            ("copy", &[image(copy)]),
            ("no-drop", &[image(refused)]),
        ],
    );
    let shown = |accepts: bool, option: bool| {
        let mut server = Compositor::new(Size::from((256, 192))).expect("headless compositor");
        server.use_cursor_theme(CursorTheme {
            search_path: vec![root.path().to_path_buf()],
            name: "Fixture".to_owned(),
            size: 24,
        });
        let mut client = TestClient::connect(&mut server);
        client.use_seat(&mut server);
        client.join_seat(&mut server);
        client.accept_drops(accepts);
        let pixels = RED.repeat(96 * 96);
        let window = ShmImage {
            width: 96,
            height: 96,
            stride: 96 * 4,
            format: Format::Xrgb8888,
            bytes: &pixels,
        };
        client.show_window(&mut server, &window);
        let origin = server.windows()[0].location();
        let pointer = Point::from((f64::from(origin.x + 30), f64::from(origin.y + 30)));
        server.move_pointer_to(pointer, 1);
        server.press_button(BTN_LEFT, true, 2);
        client.exchange(&mut server);
        let icon = BLUE.repeat(4 * 4);
        let icon = ShmImage {
            width: 4,
            height: 4,
            stride: 16,
            format: Format::Argb8888,
            bytes: &icon,
        };
        client.start_drag(&mut server, &icon, (-8, -8));
        server.move_pointer_to(pointer, 3);
        client.exchange(&mut server);
        if option {
            server.key(KEY_LEFTALT, true, 4);
            client.exchange(&mut server);
        }
        let mut renderer = Renderer::new().expect("Vulkan renderer");
        renderer.render(&mut server).expect("frame");
        let frame = renderer.snapshot(&server).expect("read back");
        rgb_at(&frame, origin.x + 42, origin.y + 42)
    };

    assert_eq!(shown(true, false), arrow);
    assert_eq!(shown(true, true), copy);
    assert_eq!(shown(false, false), refused);
}
