#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs::File;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bande_lock::{BackdropSource, Fit, Look, Variant, load_backdrop};

fn looks_shown(picture: &[u8], patience: u64) -> Vec<Look> {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("picture");
    std::fs::write(&path, picture).unwrap();
    let source = BackdropSource::Picture(Some(Arc::new(File::open(&path).unwrap())), Fit::Fill);
    let looks = vec![Look::Blurred, Look::Sharp];
    let mut inbox = load_backdrop(source, Variant::Dark, (64, 40), looks).unwrap();
    let until = Instant::now() + Duration::from_secs(patience);
    let mut shown = Vec::new();
    while shown.len() < 2 && Instant::now() < until {
        shown.extend(inbox.drain().into_iter().map(|(look, _)| look));
        std::thread::sleep(Duration::from_millis(20));
    }
    shown
}

fn use_worker(program: &str) {
    // SAFETY: this binary's only test reads the environment only through std::env, which
    // serialises with set_var.
    unsafe { std::env::set_var("BANDE_PREVIEW_WORKER", program) };
}

#[test]
fn pictures_are_decoded_in_a_confined_process_that_may_fail_alone() {
    let mut jpeg = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_pixel(800, 500, image::Rgb([200, 40, 90]))
        .write_to(&mut jpeg, image::ImageFormat::Jpeg)
        .unwrap();
    use_worker(env!("CARGO_BIN_EXE_bande-lock"));
    assert_eq!(
        looks_shown(jpeg.get_ref(), 20),
        vec![Look::Blurred, Look::Sharp]
    );
    assert!(looks_shown(&[], 2).is_empty());
    use_worker("/bin/false");
    assert!(
        looks_shown(jpeg.get_ref(), 2).is_empty(),
        "nothing is decoded in this process"
    );
}
