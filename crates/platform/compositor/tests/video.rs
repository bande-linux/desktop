#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::thread;
use std::time::Duration;

use bande_compositor::Renderer;
use bande_test_wayland::{Nv12Frame, TestClient, headless};

#[test]
fn an_nv12_video_buffer_shows_its_colour() {
    let mut server = headless(400, 300);
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let mut client = TestClient::connect(&mut server);
    let red = Nv12Frame {
        width: 128,
        height: 96,
        luma: 63,
        blue_difference: 102,
        red_difference: 240,
    };
    if !client.show_nv12(&mut server, &red) {
        return;
    }
    let mut seen = Vec::new();
    for _ in 0..50 {
        renderer.render(&mut server).expect("render");
        client.exchange(&mut server);
        let snapshot = renderer.snapshot(&server).expect("snapshot");
        seen = snapshot.rgba;
        if seen
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[0] > 240 && pixel[1] < 20 && pixel[2] < 20)
        {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let brightest = seen
        .as_chunks::<4>()
        .0
        .iter()
        .max_by_key(|pixel| pixel[0])
        .map(|pixel| pixel.to_vec());
    panic!("no red in the frame, brightest {brightest:?}");
}
