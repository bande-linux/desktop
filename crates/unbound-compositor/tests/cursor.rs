use std::fs;
use std::path::Path;

use smithay::utils::{Point, Size};
use unbound_compositor::{BACKGROUND, CursorTheme, Headless, Renderer, Snapshot};

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
    let mut server = Headless::new(Size::from((256, 192))).expect("headless compositor");
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

fn rgb(snapshot: &Snapshot, x: i32, y: i32) -> [u8; 3] {
    let at = ((y * snapshot.size.w + x) * 4) as usize;
    [
        snapshot.rgba[at],
        snapshot.rgba[at + 1],
        snapshot.rgba[at + 2],
    ]
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
    assert_eq!(rgb(&frame, x - 3, y - 5), green);
    assert_eq!(rgb(&frame, x + 20, y + 18), green);
    assert_eq!(rgb(&frame, x - 4, y - 5), BACKGROUND);
    assert_eq!(rgb(&frame, x - 3, y - 6), BACKGROUND);
    assert_eq!(rgb(&frame, x + 21, y), BACKGROUND);
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

    assert_eq!(rgb(&frame, POINTER.0 + 10, POINTER.1 + 10), blue);
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

    assert_eq!(rgb(&frame, POINTER.0, POINTER.1), large);
    assert_eq!(rgb(&frame, POINTER.0 + 30, POINTER.1 + 30), large);
}
