#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs::File;
use std::sync::Arc;

use bande_lock::{BackdropSource, Look, Variant, load_backdrop};
use rustix::event::{PollFd, PollFlags, Timespec, poll};

#[test]
fn a_shared_picture_is_drawn_after_the_greeter_lets_go_of_its_file() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("picture.png");
    image::RgbaImage::from_pixel(8, 8, image::Rgba([200, 40, 40, 255]))
        .save(&path)
        .unwrap();
    let picture = Arc::new(File::open(&path).unwrap());
    let source = BackdropSource::Picture(Some(Arc::clone(&picture)));
    let mut drawn = load_backdrop(source, Variant::Dark, (16, 16), vec![Look::Sharp]).unwrap();
    drop(picture);
    std::fs::remove_file(&path).unwrap();
    let ready = drawn.fd();
    let mut fds = [PollFd::new(&ready, PollFlags::IN)];
    let timeout = Timespec {
        tv_sec: 10,
        tv_nsec: 0,
    };
    assert_eq!(poll(&mut fds, Some(&timeout)).unwrap(), 1);
    let looks: Vec<Look> = drawn.drain().into_iter().map(|(look, _)| look).collect();
    assert_eq!(looks, [Look::Sharp]);
}
