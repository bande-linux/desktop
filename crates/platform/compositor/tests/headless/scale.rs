use crate::support::*;
use bande_compositor::{Compositor, Renderer};
use bande_test_wayland::{CaptureOutcome, RED, TestClient, roundtrip};
use smithay::utils::{Physical, Size};

const LOGICAL: (i32, i32) = (256, 192);
const WINDOW: (i32, i32) = (64, 48);
const SCALES: [f64; 2] = [2.0, 1.5];

struct Scene {
    server: Compositor,
    renderer: Renderer,
    app: TestClient,
    shell: TestClient,
    scale: f64,
    physical: Size<i32, Physical>,
}

fn scene(scale: f64) -> Scene {
    let physical = Size::from((
        (f64::from(LOGICAL.0) * scale).round() as i32,
        (f64::from(LOGICAL.1) * scale).round() as i32,
    ));
    let mut server = Compositor::new(physical).expect("headless compositor");
    server.set_output_scale(scale);
    let mut shell = top_bar(&mut server);
    let mut app = window(&mut server, RED, WINDOW);
    roundtrip(&mut server, &mut [&mut app, &mut shell]);
    Scene {
        server,
        renderer: Renderer::new().expect("renderer"),
        app,
        shell,
        scale,
        physical,
    }
}

impl Scene {
    fn pixel(&self, rgba: &[u8], logical: (f64, f64)) -> [u8; 3] {
        let x = (logical.0 * self.scale).floor() as i32;
        let y = (logical.1 * self.scale).floor() as i32;
        let at = ((y * self.physical.w + x) * 4) as usize;
        [rgba[at], rgba[at + 1], rgba[at + 2]]
    }

    fn window_origin(&self) -> (f64, f64) {
        let location = self.server.windows()[0].location();
        (f64::from(location.x), f64::from(location.y))
    }

    fn check(&self, rgba: &[u8]) {
        let (x, y) = self.window_origin();
        let red = [0xff, 0, 0];
        let green = [0, 0xff, 0];
        let scale = self.scale;
        assert_eq!(self.pixel(rgba, (x + 32.0, y + 24.0)), red, "{scale}");
        assert_eq!(self.pixel(rgba, (x + 62.5, y + 24.0)), red, "{scale}");
        assert_eq!(self.pixel(rgba, (x + 32.0, y + 47.5)), red, "{scale}");
        assert_ne!(self.pixel(rgba, (x + 64.5, y + 24.0)), red, "{scale}");
        assert_eq!(self.pixel(rgba, (2.0, 12.0)), green, "{scale}");
        assert_eq!(
            self.pixel(rgba, (f64::from(LOGICAL.0) - 2.0, 12.0)),
            green,
            "{scale}"
        );
        assert_ne!(
            self.pixel(rgba, (f64::from(LOGICAL.0) - 2.0, 26.0)),
            green,
            "{scale}"
        );
    }
}

#[test]
fn windows_and_layers_are_drawn_at_the_output_scale() {
    for scale in SCALES {
        let mut scene = scene(scale);

        scene.renderer.render(&mut scene.server).expect("frame");
        let snapshot = scene.renderer.snapshot(&scene.server).expect("read back");

        scene.check(&snapshot.rgba);
    }
}

#[test]
fn a_capture_shows_the_screen_at_the_output_scale() {
    for scale in SCALES {
        let mut scene = scene(scale);
        let size = (scene.physical.w, scene.physical.h);
        scene.shell.start_screen_capture(&mut scene.server, false);
        scene.shell.capture_frame(&mut scene.server, size);
        scene.renderer.render(&mut scene.server).expect("frame");
        scene.shell.exchange(&mut scene.server);
        let argb = match scene.shell.capture_outcome() {
            Some(CaptureOutcome::Ready(pixels)) => pixels.clone(),
            other => panic!("capture did not complete: {other:?}"),
        };
        let rgba: Vec<u8> = argb
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|&[blue, green, red, alpha]| [red, green, blue, alpha])
            .collect();

        scene.check(&rgba);
    }
}

#[test]
fn damage_is_repainted_in_output_pixels() {
    for scale in SCALES {
        let mut scene = scene(scale);
        scene.renderer.render(&mut scene.server).expect("frame");
        let pixels = solid(RED, WINDOW);

        scene
            .app
            .commit_buffer(0, &xrgb(&pixels, WINDOW), (16, 16, 8, 8));
        scene.app.exchange(&mut scene.server);
        let stats = scene.renderer.render(&mut scene.server).expect("frame");

        let side = (8.0 * scale).ceil() as u64;
        assert!(
            stats.repainted_pixels >= side * side && stats.repainted_pixels <= (side + 2).pow(2),
            "{scale}: {}",
            stats.repainted_pixels
        );
    }
}

#[test]
fn the_pointer_reaches_the_window_point_under_it() {
    for scale in SCALES {
        let mut scene = scene(scale);
        let (x, y) = scene.window_origin();

        scene.server.move_pointer_to((x + 10.0, y + 20.0).into(), 0);
        roundtrip(&mut scene.server, &mut [&mut scene.app]);

        assert_eq!(scene.app.pointer_focus(), Some((0, 10.0, 20.0)), "{scale}");
    }
}

#[test]
fn changing_the_scale_keeps_the_pointer_where_it_is_on_screen() {
    let mut server = Compositor::new(Size::from((512, 384))).expect("headless compositor");
    server.move_pointer_to((100.0, 50.0).into(), 0);

    server.set_output_scale(2.0);
    assert_eq!(server.pointer_location(), (50.0, 25.0).into());

    server.set_output_scale(1.0);
    assert_eq!(server.pointer_location(), (100.0, 50.0).into());
}
