#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs::{File, OpenOptions};
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use bande_compositor::background;
use bande_compositor::{
    Compositor, KEYBOARD_MONITOR_NAME, SCREEN_READER_NAME, SessionReport, UNRESPONSIVE_AFTER,
    serve_keyboard_monitor,
};
use bande_render::VulkanRenderer;
use bande_test_bus::SessionBus;
use bande_test_wayland::keys::{
    BTN_LEFT, KEY_A, KEY_DOWN, KEY_ENTER, KEY_ESC, KEY_L, KEY_LEFTALT, KEY_LEFTCTRL, KEY_LEFTMETA,
    KEY_Q, KEY_SPACE, KEY_TAB, KEY_W,
};
use bande_test_wayland::{
    BLUE, ConstraintAdjustment, Frames, PopupRequest, RED, SeparateApp, ShmImage, TestClient,
    act_as_separate_app, chord, grant_consent, headless, roundtrip,
};
use smithay::backend::allocator::dmabuf::{AsDmabuf, Dmabuf};
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Allocator, Buffer, Fourcc, Modifier};
use smithay::backend::renderer::Bind;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::utils::Transform;
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
        encode: Some(Arc::from(converted.encode)),
    }));
}

fn drag_over_scanout_buffers(colour: Option<bande_render::OutputColour>) {
    let Some((mut vulkan, gbm)) = hardware() else {
        println!("skipped: no hardware Vulkan device with a DRM render node");
        return;
    };
    vulkan.set_output_colour(colour);
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
        let mut framebuffer = vulkan.bind(dmabuf).expect("bind the scanout buffer");
        damage
            .render_output(&mut vulkan, &mut framebuffer, age, &elements, background())
            .expect("render into the scanout buffer");
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
    let mut framebuffer = vulkan.bind(&mut fresh).expect("bind the fresh buffer");
    OutputDamageTracker::new((640, 480), 1.0, Transform::Normal)
        .render_output(&mut vulkan, &mut framebuffer, 0, &elements, background())
        .expect("a full redraw");
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
