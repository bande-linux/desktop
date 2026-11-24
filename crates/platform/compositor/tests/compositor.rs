#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs::{File, OpenOptions};
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use bande_compositor::background;
use bande_compositor::{
    Compositor, KEYBOARD_MONITOR_NAME, OutputConfig, SCREEN_READER_NAME, ScreenElement,
    SessionReport, UNRESPONSIVE_AFTER, serve_keyboard_monitor,
};
use bande_render::{OffscreenTarget, VulkanRenderer};
use bande_test_bus::SessionBus;
use bande_test_wayland::keys::{
    BTN_LEFT, KEY_8, KEY_A, KEY_DOWN, KEY_ENTER, KEY_ESC, KEY_L, KEY_LEFTALT, KEY_LEFTCTRL,
    KEY_LEFTMETA, KEY_Q, KEY_SPACE, KEY_TAB, KEY_W,
};
use bande_test_wayland::{
    BLUE, ConstraintAdjustment, Frames, PopupRequest, RED, SeparateApp, ShmImage, TestClient,
    act_as_separate_app, chord, grant_consent, headless, roundtrip,
};
use smithay::backend::allocator::dmabuf::{AsDmabuf, Dmabuf};
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Allocator, Buffer, Fourcc, Modifier};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::{Element, RenderElement};
use smithay::backend::renderer::{Bind, Offscreen};
use smithay::utils::{Rectangle, Transform};
use wayland_client::protocol::wl_shm::Format;

const MONITOR: &str = "org.freedesktop.a11y.KeyboardMonitor";
const MONITOR_PATH: &str = "/org/freedesktop/a11y/Manager";

fn focused_app() -> (Compositor, TestClient) {
    let mut server = headless(560, 520);
    let mut editor = TestClient::window(&mut server, "org.example.Editor");
    server.move_pointer_to((280.0, 260.0).into(), 0);
    server.press_button(BTN_LEFT, true, 0);
    server.press_button(BTN_LEFT, false, 0);
    roundtrip(&mut server, &mut [&mut editor]);
    assert!(editor.keyboard_focus_on(0));
    (server, editor)
}

fn locked(server: &mut Compositor) -> TestClient {
    let mut lock = TestClient::connect_as_lock_screen(server);
    lock.use_seat(server);
    lock.lock_session(server);
    lock.show_lock_surface(server, BLUE);
    lock
}

#[test]
fn the_browser_row_gives_the_well_to_the_compositor_and_the_rest_to_the_browser() {
    let mut server = headless(560, 520);
    let bytes = RED.repeat(320 * 200);
    let mut browser = TestClient::connect(&mut server);
    browser.use_seat(&mut server);
    browser.show_window(
        &mut server,
        &ShmImage {
            width: 320,
            height: 200,
            stride: 1280,
            format: Format::Xrgb8888,
            bytes: &bytes,
        },
    );
    browser.set_app_id(&mut server, 0, "org.bande.Browser");
    roundtrip(&mut server, &mut [&mut browser]);
    let rect = server.windows()[0].geometry().expect("mapped");
    let right = rect.loc.x + rect.size.w;
    let click = |server: &mut Compositor, from_right: i32| {
        server.move_pointer_to(
            (f64::from(right - from_right), f64::from(rect.loc.y + 24)).into(),
            0,
        );
        server.press_button(BTN_LEFT, true, 0);
        server.press_button(BTN_LEFT, false, 0);
    };
    click(&mut server, 90);
    roundtrip(&mut server, &mut [&mut browser]);
    assert_eq!(browser.buttons(), [(BTN_LEFT, true), (BTN_LEFT, false)]);
    click(&mut server, 66);
    roundtrip(&mut server, &mut [&mut browser]);
    assert_eq!(browser.buttons().len(), 2);
    assert!(server.windows()[0].is_minimized());
}

#[test]
fn while_locked_no_popup_grab_input_method_or_virtual_keyboard_takes_keys_from_the_lock_surface() {
    let (mut server, mut editor) = focused_app();
    let _consent = grant_consent(
        &mut server,
        "read-keys sandboxed org.example.Ime\ntype-keys sandboxed org.example.Typist\n",
    );
    let mut input_method = TestClient::connect_sandboxed(&mut server, "org.example.Ime");
    input_method.use_seat(&mut server);
    input_method.become_input_method(&mut server);
    assert!(!input_method.input_method_unavailable());
    editor.enable_text_input(&mut server);
    input_method.grab_keyboard(&mut server);
    let mut typist = TestClient::connect_sandboxed(&mut server, "org.example.Typist");
    typist.use_seat(&mut server);
    assert!(typist.create_virtual_keyboard(&mut server).is_ok());
    let mut lock = locked(&mut server);
    roundtrip(
        &mut server,
        &mut [&mut lock, &mut editor, &mut input_method, &mut typist],
    );
    assert!(lock.lock_surface_has_keyboard());
    let popup = editor.open_popup(
        &mut server,
        &PopupRequest {
            parent: 0,
            anchor_rect: (0, 0, 1, 1),
            size: (20, 20),
            adjustment: ConstraintAdjustment::empty(),
            grab: true,
            color: RED,
        },
    );
    let clicks_before = editor.buttons().len();

    chord(&mut server, &[KEY_A]);
    typist.type_on_virtual_keyboard(&mut server, KEY_Q);
    server.move_pointer_to((280.0, 260.0).into(), 3);
    server.press_button(BTN_LEFT, true, 4);
    server.press_button(BTN_LEFT, false, 5);
    roundtrip(
        &mut server,
        &mut [&mut lock, &mut editor, &mut input_method, &mut typist],
    );

    assert!(editor.popup_dismissed(popup));
    assert!(editor.keys().is_empty());
    assert_eq!(editor.buttons().len(), clicks_before);
    assert!(input_method.grabbed_keys().is_empty());
    assert!(lock.lock_surface_has_keyboard());
    assert!(lock.keys().contains(&(KEY_A, true)));
    assert!(!lock.keys().contains(&(KEY_Q, true)));
}

#[test]
fn super_l_on_the_lock_screen_locks_again_instead_of_waking_it() {
    let (mut server, mut editor) = focused_app();
    let (reports, reported) = std::sync::mpsc::channel();
    server.report_session(reports);
    let mut lock = locked(&mut server);
    roundtrip(&mut server, &mut [&mut lock, &mut editor]);

    chord(&mut server, &[KEY_LEFTMETA, KEY_L]);
    roundtrip(&mut server, &mut [&mut lock, &mut editor]);

    assert!(!lock.keys().contains(&(KEY_L, true)));
    assert!(
        reported
            .try_iter()
            .any(|report| report == SessionReport::LockStarted)
    );
}

#[test]
fn a_lock_behind_dark_displays_is_ready_once_its_surface_is_drawn() {
    let (mut server, mut editor) = focused_app();
    let (reports, reported) = std::sync::mpsc::channel();
    server.report_session(reports);
    let mut power = TestClient::connect(&mut server);
    power.turn_the_display_off(&mut server);

    let mut lock = locked(&mut server);
    roundtrip(&mut server, &mut [&mut lock, &mut editor, &mut power]);

    assert!(
        reported
            .try_iter()
            .any(|report| report == SessionReport::Locked(true))
    );
}

#[test]
fn a_consented_input_method_sees_keys_only_in_a_text_field_that_is_no_password() {
    let (mut server, mut editor) = focused_app();
    let _consent = grant_consent(&mut server, "read-keys sandboxed org.example.Ime\n");
    let mut input_method = TestClient::connect_sandboxed(&mut server, "org.example.Ime");
    input_method.use_seat(&mut server);
    input_method.become_input_method(&mut server);
    input_method.grab_keyboard(&mut server);

    chord(&mut server, &[KEY_A]);
    roundtrip(&mut server, &mut [&mut input_method, &mut editor]);
    assert!(input_method.grabbed_keys().is_empty());

    editor.enable_text_input(&mut server);
    input_method.grab_keyboard(&mut server);
    chord(&mut server, &[KEY_Q]);
    roundtrip(&mut server, &mut [&mut input_method, &mut editor]);
    assert!(input_method.grabbed_keys().contains(&(KEY_Q, true)));
    assert!(!editor.keys().contains(&(KEY_Q, true)));

    editor.mark_text_input_as_password(&mut server);
    chord(&mut server, &[KEY_W]);
    roundtrip(&mut server, &mut [&mut input_method, &mut editor]);
    assert!(!input_method.grabbed_keys().contains(&(KEY_W, true)));
    assert!(editor.keys().contains(&(KEY_W, true)));
}

#[test]
fn without_consent_nothing_types_and_turning_it_off_hands_the_keys_back_at_once() {
    let (mut server, mut editor) = focused_app();
    let mut typist = TestClient::connect(&mut server);
    typist.use_seat(&mut server);
    assert!(typist.create_virtual_keyboard(&mut server).is_err());

    let consent = grant_consent(&mut server, "read-keys sandboxed org.example.Ime\n");
    let mut input_method = TestClient::connect_sandboxed(&mut server, "org.example.Ime");
    input_method.use_seat(&mut server);
    input_method.become_input_method(&mut server);
    assert!(!input_method.input_method_unavailable());
    editor.enable_text_input(&mut server);
    input_method.grab_keyboard(&mut server);
    chord(&mut server, &[KEY_A]);
    roundtrip(&mut server, &mut [&mut input_method, &mut editor]);
    assert!(input_method.grabbed_keys().contains(&(KEY_A, true)));

    let replacement = consent.path().join("replacement");
    std::fs::write(&replacement, "").unwrap();
    std::fs::rename(&replacement, consent.path().join("consent")).unwrap();
    server.dispatch().unwrap();
    chord(&mut server, &[KEY_Q]);
    roundtrip(&mut server, &mut [&mut editor]);
    input_method.read_what_arrived();

    assert!(editor.keys().contains(&(KEY_Q, true)));
    assert!(!input_method.grabbed_keys().contains(&(KEY_Q, true)));
}

#[test]
fn only_the_lock_screen_may_lock_and_a_sandbox_gets_no_privileged_protocol() {
    let mut server = headless(560, 520);
    let shell = TestClient::connect_as_shell(&mut server);
    assert!(!shell.sees_global("ext_session_lock_manager_v1"));
    let lock = TestClient::connect_as_lock_screen(&mut server);
    assert!(lock.sees_global("ext_session_lock_manager_v1"));
    let sandboxed = TestClient::connect_sandboxed(&mut server, "org.example.App");
    for privileged in [
        "ext_session_lock_manager_v1",
        "bande_app_control_v1",
        "zwlr_layer_shell_v1",
        "zwlr_output_manager_v1",
        "zwlr_output_power_manager_v1",
        "ext_idle_notifier_v1",
        "ext_data_control_manager_v1",
        "ext_image_copy_capture_manager_v1",
        "wp_security_context_manager_v1",
    ] {
        assert!(!sandboxed.sees_global(privileged), "{privileged}");
    }
}

#[test]
fn only_the_screen_reader_may_grab_the_keyboard_and_it_lets_go_when_it_leaves() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let (mut server, mut app) = focused_app();
    let handle = serve_keyboard_monitor(&bus.connect().expect("connection")).expect("monitor");
    server.monitor_keyboard(handle);
    let grab = |connection: &zbus::blocking::Connection| {
        connection.call_method(
            Some(KEYBOARD_MONITOR_NAME),
            MONITOR_PATH,
            Some(MONITOR),
            "GrabKeyboard",
            &(),
        )
    };
    let stranger = bus.connect().expect("other connection");
    assert!(grab(&stranger).is_err());

    let reader = bus.connect().expect("screen reader connection");
    reader
        .request_name(SCREEN_READER_NAME)
        .expect("screen reader name");
    grab(&reader).expect("the screen reader grabs the keyboard");
    chord(&mut server, &[KEY_A]);
    roundtrip(&mut server, &mut [&mut app]);
    assert!(app.keys().is_empty());

    drop(reader);
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.keys().is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
        chord(&mut server, &[KEY_A]);
        roundtrip(&mut server, &mut [&mut app]);
    }
    assert!(!app.keys().is_empty());
}

#[test]
fn a_hung_app_is_force_quit_from_the_keyboard() {
    if act_as_separate_app() {
        return;
    }
    let mut server = headless(560, 520);
    let mut bar = TestClient::connect_as_shell(&mut server);
    bar.join_shell(&mut server);
    let mut stuck = SeparateApp::launch(&mut server, "a_hung_app_is_force_quit_from_the_keyboard");
    stuck.show_window_of(&mut server, 64, "org.example.Stuck");
    let mut editor = TestClient::window(&mut server, "org.example.Editor");
    roundtrip(&mut server, &mut [&mut editor]);

    chord(&mut server, &[KEY_LEFTCTRL, KEY_LEFTALT, KEY_ESC]);
    let start = Duration::from_secs(10);
    server.watchdog_tick(start);
    roundtrip(&mut server, &mut [&mut editor]);
    server.watchdog_tick(start + UNRESPONSIVE_AFTER);
    server.force_quit_tick(Instant::now());
    assert_eq!(
        server.force_quit_rows(),
        ["Editor", "Stuck (Not Responding)"]
    );
    chord(&mut server, &[KEY_DOWN]);
    chord(&mut server, &[KEY_ENTER]);
    assert!(server.force_quit_asks());
    chord(&mut server, &[KEY_ENTER]);
    assert!(server.force_quit_asks());
    chord(&mut server, &[KEY_TAB]);
    chord(&mut server, &[KEY_SPACE]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while server
        .windows()
        .iter()
        .any(|window| window.app_id() == "org.example.Stuck")
    {
        assert!(Instant::now() < deadline, "the hung app is still running");
        server.dispatch().expect("server dispatch");
        thread::sleep(Duration::from_millis(5));
    }
    server.force_quit_tick(Instant::now());
    roundtrip(&mut server, &mut [&mut editor]);

    assert_eq!(server.force_quit_rows(), ["Editor"]);
    assert!(
        editor
            .keys()
            .iter()
            .all(|(key, _)| ![KEY_ESC, KEY_DOWN, KEY_ENTER, KEY_TAB, KEY_SPACE].contains(key))
    );
}

fn idle(frames: &mut Frames, server: &mut Compositor, time: Duration) {
    let until = Instant::now() + time;
    frames.run_until(server, || Instant::now() > until);
}

fn expect_frame(
    frames: &mut Frames,
    server: &mut Compositor,
    client: &mut TestClient,
    before: u32,
) {
    let patience = Instant::now() + Duration::from_secs(2);
    frames.run_until(server, || {
        assert!(Instant::now() < patience, "no frame callback came");
        client.frames_done() > before
    });
}

#[test]
fn an_idle_window_is_not_drawn_again_and_still_gets_a_frame_when_it_asks() {
    let mut server = headless(640, 480);
    let mut client = TestClient::window(&mut server, "org.bande.FrameTest");
    let child = client.add_child(0);
    let mut frames = Frames::start();
    idle(&mut frames, &mut server, Duration::from_millis(1200));
    let drawn = frames.rendered();
    assert!(drawn > 0, "the window was never drawn");
    idle(&mut frames, &mut server, Duration::from_millis(1200));
    assert_eq!(frames.rendered(), drawn, "an idle window was drawn again");

    let before = client.frames_done();
    client.ask_for_frame(0);
    expect_frame(&mut frames, &mut server, &mut client, before);

    let before = client.frames_done();
    client.ask_for_child_frame(child);
    idle(&mut frames, &mut server, Duration::from_millis(1200));
    let bytes = BLUE.repeat(32 * 32);
    let image = ShmImage {
        width: 32,
        height: 32,
        stride: 128,
        format: Format::Xrgb8888,
        bytes: &bytes,
    };
    client.fill_child(child, &image);
    expect_frame(&mut frames, &mut server, &mut client, before);

    idle(&mut frames, &mut server, Duration::from_millis(1200));
    let before = client.frames_done();
    client.ask_for_child_frame(child);
    expect_frame(&mut frames, &mut server, &mut client, before);
}

#[test]
fn hdr_content_is_taken_only_while_a_display_shows_hdr_and_stays_visible_elsewhere() {
    let grey = [148, 148, 148, 0xff].repeat(64 * 64);
    let image = ShmImage {
        width: 64,
        height: 64,
        stride: 256,
        format: Format::Argb8888,
        bytes: &grey,
    };
    let mut server = headless(64, 64);
    let mut client = TestClient::connect(&mut server);
    assert!(!client.offered_colour_management());
    let output = server.main_output();
    let config = OutputConfig {
        hdr: Some(bande_edid::Hdr {
            max: Some(400.0),
            max_average: None,
            min: Some(0.05),
        }),
        ..OutputConfig::virtual_display(&output, (64, 64).into())
    };
    let identity = config.identity();
    server.refit_output(config);
    let hdr = |on: bool| {
        bande_config::Config::parse(&format!("display \"{identity}\" {{ hdr #{on} }}"))
            .expect("a display setting")
    };
    server.set_display_config(hdr(true));
    client.exchange(&mut server);
    assert!(client.offered_colour_management());

    client.show_window(&mut server, &image);
    let centre = |server: &mut Compositor| {
        let mut renderer = bande_compositor::Renderer::new().expect("a renderer");
        renderer.render(server).expect("a frame");
        let frame = renderer.snapshot(server).expect("a snapshot");
        let at = ((frame.size.h / 2 * frame.size.w + frame.size.w / 2) * 4) as usize;
        frame.rgba[at]
    };
    client.make_full_screen(&mut server, 0);
    client.commit_buffer(0, &image, (0, 0, 64, 64));
    idle(&mut Frames::start(), &mut server, Duration::from_secs(1));
    let mut vulkan = VulkanRenderer::new(None).expect("a renderer");
    let mut shown = |server: &mut Compositor| {
        let elements = server.render_elements_on(&mut vulkan, &output);
        let scanned = elements
            .iter()
            .filter_map(|element| element.underlying_storage(&mut vulkan))
            .count();
        (elements.iter().any(ScreenElement::shows_hdr), scanned)
    };
    assert_eq!(shown(&mut server), (false, 1), "a plain buffer for a plane");
    assert_eq!(centre(&mut server), 148, "plain content as it is");
    let described = client.describe_as_pq(&mut server, 0, 203);
    client.commit_buffer(0, &image, (0, 0, 64, 64));
    client.exchange(&mut server);
    assert_eq!(
        shown(&mut server),
        (true, 0),
        "HDR content goes to no plane"
    );
    assert!(
        centre(&mut server) >= 253,
        "PQ 203 cd/m² is the white of an ordinary frame"
    );
    described.unset_image_description();
    client.commit_buffer(0, &image, (0, 0, 64, 64));
    client.exchange(&mut server);
    assert_eq!(shown(&mut server), (false, 1));
    assert_eq!(centre(&mut server), 148);

    let mut late = TestClient::connect(&mut server);
    late.show_window(&mut server, &image);
    client.follow_preferences(&mut server, 0);
    server.set_display_config(hdr(false));
    late.describe_as_pq(&mut server, 0, 203);
    client.exchange(&mut server);
    assert_eq!(client.preferences_changed(), 1);
    assert!(!TestClient::connect(&mut server).offered_colour_management());
    server.set_display_config(hdr(true));
    client.exchange(&mut server);
    assert_eq!(client.preferences_changed(), 2);
}

#[test]
fn a_window_dragged_over_scanout_buffers_leaves_the_frame_a_full_redraw_draws() {
    drag_over_scanout_buffers(None);
}

#[test]
fn a_window_dragged_on_a_wide_gamut_display_leaves_the_frame_a_full_redraw_draws() {
    let converted = bande_colour::Profile::display_p3()
        .and_then(|profile| bande_colour::conversion(&profile, [1.0; 3]))
        .expect("a Display P3 conversion");
    drag_over_scanout_buffers(Some(bande_render::OutputColour {
        matrix: converted.matrix,
        encode: bande_render::Encode::Table(Arc::from(converted.encode)),
        extended: false,
    }));
}

fn drag_over_scanout_buffers(colour: Option<bande_render::OutputColour>) {
    let Some((mut vulkan, gbm)) = hardware() else {
        println!("skipped: no hardware Vulkan device with a DRM render node");
        return;
    };
    let mut formats: Vec<_> = vulkan.render_formats().iter().copied().collect();
    formats.sort_by_key(|format| std::cmp::Reverse(u64::from(format.modifier)));
    let Some(format) = formats
        .into_iter()
        .find(|format| format.code == Fourcc::Argb8888 && format.modifier != Modifier::Linear)
    else {
        println!("skipped: the GPU cannot render into a tiled ARGB8888 scanout buffer");
        return;
    };
    println!("drawing into {format:?} on {}", vulkan.device_info());
    let (width, height) = (640, 480);
    let mut allocator = GbmAllocator::new(
        gbm.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    let mut slots: Vec<_> = (0..3)
        .map(|_| {
            let buffer = allocator
                .create_buffer(width, height, format.code, &[format.modifier])
                .expect("scanout buffer");
            let dmabuf = buffer.export().expect("scanout dma-buf");
            (buffer, dmabuf, None::<usize>)
        })
        .collect();
    let mut server = headless(640, 480);
    let _back = TestClient::window(&mut server, "org.bande.Back");
    let _front = TestClient::window(&mut server, "org.bande.Front");
    let mut frames = Frames::start();
    idle(&mut frames, &mut server, Duration::from_millis(300));
    let output = server.main_output();
    let mut damage = OutputDamageTracker::new((640, 480), 1.0, Transform::Normal);
    let mut step = 0;
    let mut draw = |server: &mut Compositor, step: &mut usize| {
        let elements = server.render_elements_on(&mut vulkan, &output);
        let slot = *step % slots.len();
        let (_, dmabuf, used) = &mut slots[slot];
        let age = used.map_or(0, |used| *step - used);
        vulkan
            .converting(colour.as_ref(), |vulkan| {
                let mut framebuffer = vulkan.bind(dmabuf).expect("bind the scanout buffer");
                damage
                    .render_output(vulkan, &mut framebuffer, age, &elements, background())
                    .expect("render into the scanout buffer")
                    .sync
            })
            .expect("the conversion table loads")
            .wait()
            .expect("the GPU finishes the frame");
        *used = Some(*step);
        *step += 1;
        slot
    };
    let frame = server.windows()[1]
        .geometry()
        .expect("the front window is mapped");
    let mut at = (
        f64::from(frame.loc.x + frame.size.w / 2),
        f64::from(frame.loc.y + frame.size.h / 2),
    );
    server.move_pointer_to(at.into(), 0);
    server.key(KEY_LEFTMETA, true, 0);
    server.press_button(BTN_LEFT, true, 0);
    let mut last = 0;
    for time in 1..=30 {
        at = (at.0 + 9.0, at.1 + 5.0);
        server.move_pointer_to(at.into(), time * 10);
        idle(&mut frames, &mut server, Duration::from_millis(5));
        last = draw(&mut server, &mut step);
    }
    server.press_button(BTN_LEFT, false, 400);
    server.key(KEY_LEFTMETA, false, 400);
    for time in 0..20 {
        let hover = (f64::from(40 + time * 25), f64::from(60 + time * 17));
        server.move_pointer_to(hover.into(), 500 + time as u32 * 10);
        idle(&mut frames, &mut server, Duration::from_millis(5));
        last = draw(&mut server, &mut step);
    }
    let drawn = read_back(&gbm, &slots[last].1, (width, height));
    let fresh = allocator
        .create_buffer(width, height, format.code, &[format.modifier])
        .expect("scanout buffer");
    let mut fresh = fresh.export().expect("scanout dma-buf");
    let elements = server.render_elements_on(&mut vulkan, &output);
    vulkan
        .converting(colour.as_ref(), |vulkan| {
            let mut framebuffer = vulkan.bind(&mut fresh).expect("bind the fresh buffer");
            OutputDamageTracker::new((640, 480), 1.0, Transform::Normal)
                .render_output(vulkan, &mut framebuffer, 0, &elements, background())
                .expect("a full redraw")
                .sync
        })
        .expect("the conversion table loads")
        .wait()
        .expect("the GPU finishes the full redraw");
    let full = read_back(&gbm, &fresh, (width, height));
    let wrong = drawn
        .as_chunks::<4>()
        .0
        .iter()
        .zip(full.as_chunks::<4>().0)
        .filter(|(drawn, full)| drawn.iter().zip(*full).any(|(a, b)| a.abs_diff(*b) > 2))
        .count();
    assert_eq!(
        wrong, 0,
        "{wrong} pixels of {format:?} differ from a full redraw"
    );
    assert_eq!(bande_render::vulkan_errors(), 0, "Vulkan reported errors");
}

fn hardware() -> Option<(VulkanRenderer, GbmDevice<Arc<File>>)> {
    let mut nodes: Vec<_> = std::fs::read_dir("/dev/dri")
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.to_string_lossy().contains("renderD"))
        .collect();
    nodes.sort();
    nodes.into_iter().find_map(|node| {
        let file = OpenOptions::new().read(true).write(true).open(node).ok()?;
        let device = file.metadata().ok()?.rdev();
        let mut vulkan = VulkanRenderer::new(Some(device)).ok()?;
        let gbm = GbmDevice::new(Arc::new(file)).ok()?;
        let info = vulkan.device_info();
        if info.software || info.render_device != Some(device) {
            return None;
        }
        let mut allocator = GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING);
        vulkan.prove_unexported(|format| {
            let buffer = allocator
                .create_buffer(64, 64, format.code, &[format.modifier])
                .ok()?;
            (buffer.format() == format)
                .then(|| buffer.export().ok())
                .flatten()
        });
        Some((vulkan, gbm))
    })
}

fn read_back(gbm: &GbmDevice<Arc<File>>, dmabuf: &Dmabuf, (width, height): (u32, u32)) -> Vec<u8> {
    let mut fds = [None; 4];
    let (mut strides, mut offsets) = ([0; 4], [0; 4]);
    for (index, ((fd, offset), stride)) in dmabuf
        .handles()
        .zip(dmabuf.offsets())
        .zip(dmabuf.strides())
        .enumerate()
    {
        fds[index] = Some(fd);
        offsets[index] = offset as i32;
        strides[index] = stride as i32;
    }
    let imported = gbm
        .import_buffer_object_from_dma_buf_with_modifiers::<()>(
            dmabuf.num_planes() as u32,
            fds,
            width,
            height,
            dmabuf.format().code,
            GbmBufferFlags::RENDERING,
            strides,
            offsets,
            dmabuf.format().modifier,
        )
        .expect("import the scanout buffer into GBM");
    imported
        .map(0, 0, width, height, |mapped| {
            let stride = mapped.stride() as usize;
            mapped
                .buffer()
                .chunks(stride)
                .take(height as usize)
                .flat_map(|row| row[..width as usize * 4].to_vec())
                .collect()
        })
        .expect("map the scanout buffer")
}

fn act_as_a_remote_viewer(socket: std::os::unix::net::UnixStream) {
    use reis::ei;
    use reis::event::{DeviceCapability, EiEvent};

    let context = ei::Context::new(socket).unwrap();
    let (connection, events) = context
        .handshake_blocking("viewer", ei::handshake::ContextType::Sender)
        .unwrap();
    let (mut pointer, mut keys, mut sequence) = (None, None, 0);
    for event in events {
        match event.unwrap() {
            EiEvent::SeatAdded(added) => added.seat.bind_capabilities(
                DeviceCapability::PointerAbsolute
                    | DeviceCapability::Button
                    | DeviceCapability::Text,
            ),
            EiEvent::DeviceResumed(resumed) => {
                sequence += 1;
                resumed
                    .device
                    .device()
                    .start_emulating(connection.serial(), sequence);
                match resumed.device.has_capability(DeviceCapability::Text) {
                    true => keys = Some(resumed.device),
                    false => pointer = Some(resumed.device),
                }
            }
            _ => {}
        }
        connection.flush().unwrap();
        if let (Some(pointer), Some(keys)) = (&pointer, &keys) {
            let absolute = pointer.interface::<ei::PointerAbsolute>().unwrap();
            let button = pointer.interface::<ei::Button>().unwrap();
            absolute.motion_absolute(280.0, 260.0);
            button.button(BTN_LEFT, ei::button::ButtonState::Press);
            pointer.device().frame(connection.serial(), 1);
            button.button(BTN_LEFT, ei::button::ButtonState::Released);
            pointer.device().frame(connection.serial(), 2);
            let text = keys.interface::<ei::Text>().unwrap();
            text.keysym(u32::from('a'), ei::keyboard::KeyState::Press);
            keys.device().frame(connection.serial(), 3);
            text.keysym(u32::from('a'), ei::keyboard::KeyState::Released);
            keys.device().frame(connection.serial(), 4);
            text.keysym(u32::from('q'), ei::keyboard::KeyState::Press);
            keys.device().frame(connection.serial(), 5);
            connection.flush().unwrap();
            return;
        }
    }
}

#[test]
fn a_screen_sharing_viewer_clicks_and_types_and_its_keys_are_released_when_it_leaves() {
    let mut server = headless(560, 520);
    let mut editor = TestClient::window(&mut server, "org.example.Editor");
    let mut shell = TestClient::connect_as_shell(&mut server);
    let refused = shell.connect_remote_input(&mut server);
    let mut screenshot = TestClient::connect_as_screenshot_tool(&mut server);
    let socket = screenshot.connect_remote_input(&mut server);
    let viewer = thread::spawn(move || act_as_a_remote_viewer(socket));
    let deadline = Instant::now() + Duration::from_secs(30);
    while !(editor.keys().contains(&(KEY_A, false))
        && editor.buttons().contains(&(BTN_LEFT, false))
        && editor.keys().contains(&(KEY_Q, false)))
    {
        assert!(
            Instant::now() < deadline,
            "the viewer's input never arrived"
        );
        roundtrip(&mut server, &mut [&mut editor, &mut screenshot]);
        thread::sleep(Duration::from_millis(5));
    }
    viewer.join().unwrap();
    assert!(editor.keyboard_focus_on(0));
    assert!(editor.keys().contains(&(KEY_A, true)));
    assert!(editor.buttons().contains(&(BTN_LEFT, true)));
    assert!(
        editor.keys().contains(&(KEY_Q, true)),
        "the key the viewer still held is released when it leaves"
    );
    let mut nothing = [0; 1];
    assert_eq!(
        std::io::Read::read(&mut &refused, &mut nothing).unwrap(),
        0,
        "only the screenshot tool gets remote input"
    );
}

#[test]
fn a_screen_sharing_viewer_only_watches_while_the_session_is_locked() {
    let (mut server, mut editor) = focused_app();
    let mut lock = locked(&mut server);
    let mut screenshot = TestClient::connect_as_screenshot_tool(&mut server);
    let socket = screenshot.connect_remote_input(&mut server);
    let viewer = thread::spawn(move || act_as_a_remote_viewer(socket));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !viewer.is_finished() {
        assert!(Instant::now() < deadline, "the viewer never finished");
        roundtrip(&mut server, &mut [&mut editor, &mut lock, &mut screenshot]);
        thread::sleep(Duration::from_millis(5));
    }
    viewer.join().unwrap();
    roundtrip(&mut server, &mut [&mut editor, &mut lock, &mut screenshot]);
    assert!(
        lock.keys().is_empty(),
        "no remote key reaches the lock screen"
    );
    assert!(
        lock.buttons().is_empty(),
        "no remote click reaches the lock screen"
    );
    assert!(!editor.keys().contains(&(KEY_A, true)));
}

#[test]
fn a_closing_window_keeps_the_buffer_it_still_shows_until_it_is_gone() {
    let mut server = headless(640, 480);
    server.set_reduce_motion(false);
    let mut client = TestClient::window(&mut server, "org.bande.Closing");
    let mut frames = Frames::start();
    let red = RED.repeat(64 * 64);
    let blue = BLUE.repeat(64 * 64);
    let image = |bytes| ShmImage {
        width: 64,
        height: 64,
        stride: 256,
        format: Format::Xrgb8888,
        bytes,
    };
    let shown = client.commit_buffer(0, &image(&red), (0, 0, 64, 64));
    idle(&mut frames, &mut server, Duration::from_millis(600));

    let _next = client.commit_buffer(0, &image(&blue), (0, 0, 64, 64));
    client.withdraw_window(0);
    roundtrip(&mut server, &mut [&mut client]);
    assert!(
        !client.released(&shown),
        "the app got back the buffer its closing window is still drawn from"
    );

    idle(&mut frames, &mut server, Duration::from_millis(600));
    roundtrip(&mut server, &mut [&mut client]);
    assert!(
        client.released(&shown),
        "the closed window never let go of its buffer"
    );
}

#[test]
fn the_overview_opens_on_the_gpu_over_windows_updated_in_pieces() {
    if hardware().is_none() {
        println!("skipped: no hardware Vulkan device with a DRM render node");
        return;
    }
    let mut server = headless(2928, 1818);
    server.set_output_scale(1.5);
    server.set_reduce_motion(false);
    let mut frames = Frames::start();
    let mut windows = Vec::new();
    for (index, (width, height)) in [(1002, 741), (1602, 963)].into_iter().enumerate() {
        let bytes = BLUE.repeat((width * height) as usize);
        let image = ShmImage {
            width,
            height,
            stride: width * 4,
            format: Format::Argb8888,
            bytes: &bytes,
        };
        let mut client = TestClient::connect(&mut server);
        client.use_seat(&mut server);
        client.show_window(&mut server, &image);
        client.set_app_id(&mut server, 0, &format!("org.bande.Piece{index}"));
        roundtrip(&mut server, &mut [&mut client]);
        idle(&mut frames, &mut server, Duration::from_millis(100));
        for (x, y, w, h) in [(870, 27, 105, 42), (825, 27, 42, 42), (27, 18, 113, 60)] {
            client.commit_buffer(0, &image, (x, y, w, h));
            idle(&mut frames, &mut server, Duration::from_millis(30));
        }
        windows.push(client);
    }
    idle(&mut frames, &mut server, Duration::from_millis(500));
    for _ in 0..2 {
        chord(&mut server, &[KEY_LEFTMETA, KEY_W]);
        idle(&mut frames, &mut server, Duration::from_millis(800));
        chord(&mut server, &[KEY_LEFTMETA, KEY_W]);
        idle(&mut frames, &mut server, Duration::from_millis(800));
    }
    assert_eq!(bande_render::vulkan_errors(), 0, "Vulkan reported errors");
}

#[test]
fn moving_the_pointer_redraws_only_the_display_it_is_on() {
    let mut server = headless(640, 480);
    server.add_output(OutputConfig::virtual_display(
        "HEADLESS-2",
        (640, 480).into(),
    ));
    let outputs = server.outputs();
    assert_eq!(outputs.len(), 2, "the second display is not arranged");
    let mut frames = Frames::start();
    idle(&mut frames, &mut server, Duration::from_millis(200));
    for output in &outputs {
        server.take_redraw_on(output);
    }
    server.move_pointer_to((100.0, 100.0).into(), 0);
    server.move_pointer_to((120.0, 110.0).into(), 1);
    let redrawn: Vec<bool> = outputs
        .iter()
        .map(|output| server.take_redraw_on(output))
        .collect();
    assert_eq!(redrawn, [true, false], "a pointer move redrew {outputs:?}");
}

#[test]
fn moving_the_pointer_redraws_the_display_the_magnifier_leaves() {
    let mut server = headless(640, 480);
    server.add_output(OutputConfig::virtual_display(
        "HEADLESS-2",
        (640, 480).into(),
    ));
    let outputs = server.outputs();
    server.move_pointer_to((900.0, 200.0).into(), 0);
    let mut beside = TestClient::window(&mut server, "org.bande.Beside");
    server.move_pointer_to((100.0, 100.0).into(), 1);
    let mut here = TestClient::window(&mut server, "org.bande.Here");
    chord(&mut server, &[KEY_LEFTALT, KEY_LEFTMETA, KEY_8]);
    chord(&mut server, &[KEY_LEFTALT, KEY_TAB]);
    roundtrip(&mut server, &mut [&mut here, &mut beside]);
    let mut frames = Frames::start();
    idle(&mut frames, &mut server, Duration::from_millis(500));
    let levels = |server: &Compositor| -> Vec<Option<f64>> {
        outputs
            .iter()
            .map(|output| server.render_scale(output))
            .collect()
    };
    assert_eq!(levels(&server), [Some(1.0), Some(2.0)]);
    for output in &outputs {
        server.take_redraw_on(output);
    }
    server.move_pointer_to((110.0, 100.0).into(), 2);
    assert_eq!(levels(&server), [Some(2.0), Some(1.0)]);
    for output in &outputs {
        assert!(server.take_redraw_on(output), "{output} is not redrawn");
    }
}

struct DrawnOutput {
    vulkan: VulkanRenderer,
    name: String,
    target: OffscreenTarget,
    damage: OutputDamageTracker,
    frames: usize,
}

impl DrawnOutput {
    fn new(server: &Compositor) -> Option<Self> {
        Self::of(server.main_output(), server, hardware()?.0)
    }

    fn of(name: String, server: &Compositor, mut vulkan: VulkanRenderer) -> Option<Self> {
        let settings = server.output_settings(&name)?;
        let size = settings.mode.size;
        let target =
            Offscreen::create_buffer(&mut vulkan, Fourcc::Abgr8888, (size.w, size.h).into())
                .expect("a target");
        Some(Self {
            vulkan,
            name,
            target,
            damage: OutputDamageTracker::new(size, settings.scale, Transform::Normal),
            frames: 0,
        })
    }

    fn draw(&mut self, server: &mut Compositor) {
        server.dispatch().expect("server dispatch");
        let age = usize::from(self.frames > 0);
        self.vulkan.measure_frames_of(Some(&self.name));
        draw_output(
            &mut self.vulkan,
            (&mut self.target, &mut self.damage, age),
            server,
            &self.name,
        );
        self.vulkan.measure_frames_of(None);
        self.frames += 1;
    }

    fn assert_matches_a_full_redraw(&mut self, server: &mut Compositor, moment: &str) {
        let (fresh, _) = hardware().expect("the GPU that drew the frames");
        let mut full =
            Self::of(self.name.clone(), server, fresh).expect("the output is still there");
        full.draw(server);
        let drawn = self.vulkan.read_target(&self.target).expect("read back");
        let full = full.vulkan.read_target(&full.target).expect("read back");
        let wrong = drawn
            .as_chunks::<4>()
            .0
            .iter()
            .zip(full.as_chunks::<4>().0)
            .filter(|(drawn, full)| drawn.iter().zip(*full).any(|(a, b)| a.abs_diff(*b) > 2))
            .count();
        assert_eq!(
            wrong, 0,
            "{moment}: {wrong} pixels differ from a full redraw"
        );
        assert_eq!(bande_render::vulkan_errors(), 0, "Vulkan reported errors");
    }
}

fn draw_output(
    vulkan: &mut VulkanRenderer,
    (target, damage, age): (&mut OffscreenTarget, &mut OutputDamageTracker, usize),
    server: &mut Compositor,
    name: &str,
) {
    let elements = server.render_elements_on(vulkan, name);
    let mut framebuffer = vulkan.bind(target).expect("bind the target");
    let drawn = damage
        .render_output(vulkan, &mut framebuffer, age, &elements, background())
        .expect("render a frame");
    let submitted = vulkan.submitted();
    let fence = vulkan.fence_of(submitted).expect("the frame's fence");
    server.keep_until_drawn(elements, (submitted, vulkan.completed()), fence);
    drop(drawn);
}

struct PatternWindow {
    client: TestClient,
    size: (i32, i32),
    format: Format,
    bytes: Vec<u8>,
}

impl PatternWindow {
    fn show(server: &mut Compositor, app_id: &str, size: (i32, i32), alpha: u8) -> Self {
        let mut window = Self {
            client: TestClient::connect(server),
            size,
            format: match alpha {
                u8::MAX => Format::Xrgb8888,
                _ => Format::Argb8888,
            },
            bytes: pattern(size, 0, alpha),
        };
        window.client.use_seat(server);
        window.client.show_window(
            server,
            &ShmImage {
                width: size.0,
                height: size.1,
                stride: size.0 * 4,
                format: window.format,
                bytes: &window.bytes,
            },
        );
        window.client.set_app_id(server, 0, app_id);
        roundtrip(server, &mut [&mut window.client]);
        window
    }

    fn commit(&mut self, server: &mut Compositor, damage: (i32, i32, i32, i32)) {
        self.client.commit_buffer(
            0,
            &ShmImage {
                width: self.size.0,
                height: self.size.1,
                stride: self.size.0 * 4,
                format: self.format,
                bytes: &self.bytes,
            },
            damage,
        );
        roundtrip(server, &mut [&mut self.client]);
    }
}

fn pattern(size: (i32, i32), shift: i32, alpha: u8) -> Vec<u8> {
    let opacity = u32::from(alpha);
    (0..size.1)
        .flat_map(|y| (0..size.0).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let [blue, green, red]: [u32; 3] = match (x / 17 + y / 13 + shift) % 3 {
                0 => [255, 30, 0],
                1 => [0, 220, 40],
                _ => [20, 0, 250],
            };
            let dimmed = |channel: u32| (channel * opacity / 255) as u8;
            [dimmed(blue), dimmed(green), dimmed(red), alpha]
        })
        .collect()
}

fn drag_window(server: &mut Compositor, window: usize, by: (f64, f64)) {
    let frame = server.windows()[window]
        .geometry()
        .expect("the window is mapped");
    let at = (
        f64::from(frame.loc.x + frame.size.w / 2),
        f64::from(frame.loc.y + frame.size.h / 2),
    );
    server.move_pointer_to(at.into(), 0);
    server.key(KEY_LEFTMETA, true, 0);
    server.press_button(BTN_LEFT, true, 0);
    for step in 1..=4u32 {
        let part = f64::from(step) / 4.0;
        server.move_pointer_to((at.0 + by.0 * part, at.1 + by.1 * part).into(), step * 10);
        server.dispatch().expect("server dispatch");
    }
    server.press_button(BTN_LEFT, false, 50);
    server.key(KEY_LEFTMETA, false, 50);
}

#[test]
fn blurred_backdrops_drawn_frame_by_frame_match_a_full_redraw() {
    let mut server = headless(1000, 700);
    let Some(mut output) = DrawnOutput::new(&server) else {
        println!("skipped: no hardware Vulkan device with a DRM render node");
        return;
    };
    let mut below = PatternWindow::show(&mut server, "org.bande.Below", (700, 500), u8::MAX);
    let mut glass = PatternWindow::show(&mut server, "org.bande.Glass", (600, 400), 60);
    glass.client.blur_behind(&mut server, 0, (600, 400));
    let mut upper = PatternWindow::show(&mut server, "org.bande.Upper", (500, 300), 60);
    upper.client.blur_behind(&mut server, 0, (500, 300));
    let _above = PatternWindow::show(&mut server, "org.bande.Above", (300, 200), u8::MAX);
    server.move_pointer_to((5.0, 690.0).into(), 0);
    for _ in 0..6 {
        output.draw(&mut server);
    }
    output.assert_matches_a_full_redraw(&mut server, "settled");
    for shift in 1..4 {
        below.bytes = pattern(below.size, shift, u8::MAX);
        below.commit(&mut server, (0, 0, 700, 500));
        output.draw(&mut server);
        output.assert_matches_a_full_redraw(&mut server, "the window below changed");
    }
    for _ in 0..20 {
        drag_window(&mut server, 3, (-4.0, 2.0));
        output.draw(&mut server);
    }
    output.assert_matches_a_full_redraw(&mut server, "an opaque window was dragged across");
}

#[test]
fn a_backdrop_left_alone_for_seconds_is_not_blurred_from_the_finished_frame() {
    let mut server = headless(1300, 700);
    let Some(mut output) = DrawnOutput::new(&server) else {
        println!("skipped: no hardware Vulkan device with a DRM render node");
        return;
    };
    let _below = PatternWindow::show(&mut server, "org.bande.Below", (400, 300), u8::MAX);
    let mut glass = PatternWindow::show(&mut server, "org.bande.Glass", (360, 260), 100);
    glass.client.blur_behind(&mut server, 0, (360, 260));
    let mut aside = PatternWindow::show(&mut server, "org.bande.Aside", (100, 100), u8::MAX);
    drag_window(&mut server, 2, (590.0, 190.0));
    server.move_pointer_to((5.0, 690.0).into(), 100);
    for _ in 0..6 {
        output.draw(&mut server);
    }
    glass.commit(&mut server, (100, 100, 40, 40));
    output.draw(&mut server);
    output.assert_matches_a_full_redraw(&mut server, "a piece redrawn at once");
    thread::sleep(Duration::from_millis(2500));
    for _ in 0..2 {
        aside.commit(&mut server, (0, 0, 10, 10));
        output.draw(&mut server);
    }
    glass.commit(&mut server, (100, 100, 40, 40));
    output.draw(&mut server);
    output.assert_matches_a_full_redraw(&mut server, "a piece redrawn after two idle seconds");
}

#[test]
fn a_mirror_keeps_its_own_blurred_backdrops() {
    let mut server = headless(1000, 700);
    let second = OutputConfig::virtual_display("HEADLESS-2", (500, 350).into());
    let displays = [
        OutputConfig::virtual_display("HEADLESS-1", (1000, 700).into()).identity(),
        second.identity(),
    ];
    server.add_output(second);
    let placed = |display: &str, mirror: Option<&str>| bande_config::Placement {
        display: display.to_owned(),
        x: 0,
        y: 0,
        main: mirror.is_none(),
        mirror: mirror.map(str::to_owned),
    };
    let config = bande_config::Config {
        arrangements: vec![vec![
            placed(&displays[0], None),
            placed(&displays[1], Some(&displays[0])),
        ]],
        ..bande_config::Config::default()
    };
    server.set_display_config(config);
    let Some(mut output) = DrawnOutput::new(&server) else {
        println!("skipped: no hardware Vulkan device with a DRM render node");
        return;
    };
    let mirror = String::from("HEADLESS-2");
    let mut mirrored: OffscreenTarget =
        Offscreen::create_buffer(&mut output.vulkan, Fourcc::Abgr8888, (500, 350).into())
            .expect("a target");
    let scale = server
        .output_settings(&mirror)
        .expect("the mirror is arranged")
        .scale;
    assert_eq!(scale, 0.5, "the second display does not mirror the first");
    let mut mirror_damage = OutputDamageTracker::new((500, 350), scale, Transform::Normal);
    let _below = PatternWindow::show(&mut server, "org.bande.Below", (700, 500), u8::MAX);
    let mut glass = PatternWindow::show(&mut server, "org.bande.Glass", (600, 400), 60);
    glass.client.blur_behind(&mut server, 0, (600, 400));
    server.move_pointer_to((5.0, 690.0).into(), 0);
    for frame in 0..6 {
        output.draw(&mut server);
        draw_output(
            &mut output.vulkan,
            (&mut mirrored, &mut mirror_damage, usize::from(frame > 0)),
            &mut server,
            &mirror,
        );
    }
    for _ in 0..3 {
        glass.commit(&mut server, (100, 100, 40, 40));
        output.draw(&mut server);
        draw_output(
            &mut output.vulkan,
            (&mut mirrored, &mut mirror_damage, 1),
            &mut server,
            &mirror,
        );
    }
    output.assert_matches_a_full_redraw(&mut server, "a piece redrawn beside a mirror");
}

#[test]
fn a_full_screen_window_stays_opaque_above_a_blurred_panel() {
    let Some((mut vulkan, _)) = hardware() else {
        println!("skipped: no hardware Vulkan device with a DRM render node");
        return;
    };
    let mut server = headless(800, 600);
    let mut window = PatternWindow::show(&mut server, "org.bande.Full", (400, 300), u8::MAX);
    let bytes = pattern((800, 40), 0, 100);
    let mut panel = TestClient::connect(&mut server);
    panel.show_blurred_panel(
        &mut server,
        &ShmImage {
            width: 800,
            height: 40,
            stride: 3200,
            format: Format::Argb8888,
            bytes: &bytes,
        },
    );
    window.client.make_full_screen(&mut server, 0);
    window.size = (800, 600);
    window.bytes = pattern(window.size, 0, u8::MAX);
    window.commit(&mut server, (0, 0, 800, 600));
    idle(&mut Frames::start(), &mut server, Duration::from_secs(1));
    let output = server.main_output();
    let opaque = server
        .render_elements_on(&mut vulkan, &output)
        .iter()
        .flat_map(|element| element.opaque_regions(1.0.into()).to_vec())
        .collect::<Vec<_>>();
    assert!(
        opaque.contains(&Rectangle::from_size((800, 600).into())),
        "the full-screen window is opaque only in {opaque:?}"
    );
}

#[test]
fn memory_of_large_uploads_is_freed_once_they_stop() {
    let mut server = headless(2000, 1500);
    let Some(mut output) = DrawnOutput::new(&server) else {
        println!("skipped: no hardware Vulkan device with a DRM render node");
        return;
    };
    let mut large = PatternWindow::show(&mut server, "org.bande.Large", (1900, 1400), u8::MAX);
    for _ in 0..6 {
        large.commit(&mut server, (0, 0, 1900, 1400));
        output.draw(&mut server);
    }
    let held = output.vulkan.memory_use().staging.1;
    assert!(held >= 1900 * 1400 * 4, "uploads held only {held} bytes");
    thread::sleep(Duration::from_millis(2500));
    server.move_pointer_to((20.0, 20.0).into(), 0);
    output.draw(&mut server);
    server.move_pointer_to((40.0, 40.0).into(), 10);
    output.draw(&mut server);
    output.vulkan.wait_idle().expect("the GPU finishes");
    let held = output.vulkan.memory_use().staging.1;
    assert!(held < 1900 * 1400 * 4, "{held} bytes are still held");
}

#[test]
fn only_frames_of_a_display_count_towards_its_gpu_time() {
    let mut server = headless(1300, 900);
    let Some(mut output) = DrawnOutput::new(&server) else {
        println!("skipped: no hardware Vulkan device with a DRM render node");
        return;
    };
    let mut window = PatternWindow::show(&mut server, "org.bande.Drawn", (800, 600), u8::MAX);
    for _ in 0..6 {
        window.commit(&mut server, (0, 0, 10, 10));
        output.draw(&mut server);
    }
    output.vulkan.wait_idle().expect("the GPU finishes");
    let display = output.vulkan.gpu_frame_time(&output.name);
    assert!(display.is_some(), "the display has no GPU time");
    let mut capture: OffscreenTarget =
        Offscreen::create_buffer(&mut output.vulkan, Fourcc::Abgr8888, (1300, 900).into())
            .expect("a target");
    for _ in 0..6 {
        let mut whole = OutputDamageTracker::new((1300, 900), 1.0, Transform::Normal);
        let name = output.name.clone();
        draw_output(
            &mut output.vulkan,
            (&mut capture, &mut whole, 0),
            &mut server,
            &name,
        );
    }
    output.vulkan.wait_idle().expect("the GPU finishes");
    assert_eq!(output.vulkan.gpu_frame_time(&output.name), display);
}
