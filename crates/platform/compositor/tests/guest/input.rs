use std::time::{Duration, Instant};

use unbound_compositor::Compositor;
use unbound_test_wayland::{RED, ShmImage, TestClient};
use wayland_client::protocol::wl_shm::Format;

use crate::session::Session;
use crate::uinput::{BTN_LEFT, EV_KEY, EV_REL, KEY_A, REL_X, VirtualDevice};

fn window(client: &mut TestClient, server: &mut Compositor, width: i32, height: i32) {
    let pixels = RED.repeat((width * height) as usize);
    client.show_window(
        server,
        &ShmImage {
            width,
            height,
            stride: width * 4,
            format: Format::Xrgb8888,
            bytes: &pixels,
        },
    );
}

#[test]
#[ignore = "needs the ci/vm guest"]
fn uinput_events_reach_the_window_under_the_pointer_after_a_click() {
    let mut session = Session::open();
    let input = session
        .output
        .input_backend()
        .expect("libinput on the seat");
    session
        .events
        .handle()
        .insert_source(input, |event, _, shared| {
            shared.server.handle_input(event).expect("deliver input");
        })
        .expect("libinput events");
    session.client.use_seat(&mut session.shared.server);
    window(&mut session.client, &mut session.shared.server, 400, 300);
    let mut front = TestClient::connect(&mut session.shared.server);
    front.use_seat(&mut session.shared.server);
    window(&mut front, &mut session.shared.server, 32, 32);
    front.exchange(&mut session.shared.server);
    assert!(front.keyboard_focus_on(0));

    let mut device = VirtualDevice::keyboard_and_mouse();
    let start = session.shared.server.pointer_location();
    let deadline = Instant::now() + Duration::from_secs(5);
    while session.shared.server.pointer_location() == start {
        assert!(
            Instant::now() < deadline,
            "libinput never opened the device"
        );
        device.frame(&[(EV_REL, REL_X, 1)]);
        session
            .events
            .dispatch(Some(Duration::from_millis(20)), &mut session.shared)
            .expect("dispatch events");
    }
    device.frame(&[(EV_REL, REL_X, 100)]);
    device.frame(&[(EV_KEY, BTN_LEFT, 1)]);
    device.frame(&[(EV_KEY, BTN_LEFT, 0)]);
    device.frame(&[(EV_KEY, KEY_A, 1)]);
    device.frame(&[(EV_KEY, KEY_A, 0)]);
    session.dispatch_until("the button and key releases", |session| {
        session.client.buttons().len() >= 2 && session.client.keys().len() >= 2
    });
    front.exchange(&mut session.shared.server);

    assert!(session.client.pointer_focus().is_some());
    assert_eq!(
        session.client.buttons(),
        [(u32::from(BTN_LEFT), true), (u32::from(BTN_LEFT), false)]
    );
    assert!(session.client.keyboard_focus_on(0));
    assert_eq!(
        session.client.keys(),
        [(u32::from(KEY_A), true), (u32::from(KEY_A), false)]
    );
    assert!(!front.keyboard_focus_on(0));
    assert!(front.keys().is_empty());
}
