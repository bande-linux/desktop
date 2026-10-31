use std::os::fd::AsFd;
use std::thread;
use std::time::Duration;

use bande_test_wayland::{Memory, RED, ShmImage, SwSyncTimeline};
use smithay::reexports::drm::control::Device as ControlDevice;
use wayland_client::protocol::wl_shm::Format;

use crate::card::{Scanout, card_with_driver};
use crate::session::Session;

const SIZE: i32 = 64;
const UNSIGNALLED_FOR: Duration = Duration::from_millis(300);

fn red() -> Vec<u8> {
    RED.repeat((SIZE * SIZE) as usize)
}

fn image(pixels: &[u8]) -> ShmImage<'_> {
    ShmImage {
        width: SIZE,
        height: SIZE,
        stride: SIZE * 4,
        format: Format::Xrgb8888,
        bytes: pixels,
    }
}

fn mapped(session: &mut Session) -> bool {
    session
        .shared
        .server
        .windows()
        .iter()
        .any(|window| window.size().is_some())
}

fn stays_unmapped(session: &mut Session) -> bool {
    thread::sleep(UNSIGNALLED_FOR);
    session.client.exchange(&mut session.shared.server);
    !mapped(session)
}

fn window_center(session: &Session, scanout: &Scanout) -> [u8; 3] {
    let location = session.shared.server.windows()[0].location();
    let (x, y) = (
        (location.x + SIZE / 2) as usize,
        (location.y + SIZE / 2) as usize,
    );
    scanout.rgb[y * scanout.width + x]
}

#[test]
#[ignore = "needs the ci/vm guest"]
fn a_dmabuf_window_waits_for_the_write_fence_on_its_buffer() {
    let mut session = Session::open();
    let timeline = SwSyncTimeline::new().expect("a sw_sync timeline");
    let fence = timeline.fence(1).expect("a sw_sync fence");
    let pixels = red();
    session.client.show_window_in(
        &mut session.shared.server,
        &image(&pixels),
        Memory::UdmabufWrittenUntil(fence.as_fd()),
    );

    assert!(
        stays_unmapped(&mut session),
        "shown before the fence signalled"
    );
    session.scanout().save("m20-implicit-fence-pending");

    timeline.advance(1).expect("signal the fence");
    session.dispatch_until("the fenced commit", mapped);
    let scanout = session.present();
    scanout.save("m20-implicit-fence-signalled");
    assert_eq!(window_center(&session, &scanout), [0xff, 0x00, 0x00]);
}

#[test]
#[ignore = "needs the ci/vm guest"]
fn a_synced_commit_waits_for_its_acquire_point_and_signals_its_release_point() {
    let mut session = Session::on("virtio_gpu");
    let device = session.output.device_fd();
    assert!(
        session.shared.server.enable_explicit_sync(device),
        "virtio-gpu waits on syncobjs through eventfds"
    );
    session.client.exchange(&mut session.shared.server);
    assert!(session.client.offers_explicit_sync());
    let card = card_with_driver("virtio_gpu");
    let syncobj = card.create_syncobj(false).expect("a syncobj");
    let timeline = card
        .syncobj_to_fd(syncobj, false)
        .expect("the syncobj's fd");
    let pixels = red();
    session.client.show_synced_window(
        &mut session.shared.server,
        &image(&pixels),
        timeline.as_fd(),
        (1, 2),
    );

    assert!(
        stays_unmapped(&mut session),
        "shown before the acquire point"
    );
    session.scanout().save("m20-acquire-pending");

    card.syncobj_timeline_signal(&[syncobj], &[1])
        .expect("signal the acquire point");
    session.dispatch_until("the acquired commit", mapped);
    let scanout = session.present();
    scanout.save("m20-acquire-signalled");
    assert_eq!(window_center(&session, &scanout), [0xff, 0x00, 0x00]);

    session.client.close_window(&mut session.shared.server, 0);
    session.present().save("m20-buffer-released");
    session.dispatch_until("the release point", |_| {
        let mut point = [0];
        card.syncobj_timeline_query(&[syncobj], &mut point, false)
            .is_ok_and(|()| point[0] >= 2)
    });
}
