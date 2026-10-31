use crate::support::*;
use bande_compositor::{BACKGROUND, Compositor, Renderer};
use bande_test_wayland::{
    Anchor, KeyboardInteractivity, Layer, LayerRequest, RED, SessionEnd, TestClient, ToplevelState,
    roundtrip,
};
use smithay::utils::Point;

const WINDOW: i32 = 64;
const KEY_F2: u32 = 60;
const KEY_F3: u32 = 61;

fn shell(server: &mut Compositor) -> TestClient {
    let mut shell = TestClient::connect_as_shell(server);
    shell.use_seat(server);
    shell.join_shell(server);
    shell
}

fn super_key(server: &mut Compositor, key: u32) {
    server.key(KEY_LEFTMETA, true, 1);
    server.key(key, true, 2);
    server.key(key, false, 3);
    server.key(KEY_LEFTMETA, false, 4);
}

#[test]
fn a_hidden_app_fades_out_loses_the_keyboard_and_comes_back_when_activated() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = window_of(&mut server, RED, (WINDOW, WINDOW), EDITOR);
    let mut dock = shell(&mut server);
    settle(&mut server, 1000);

    dock.hide_app(&mut server, EDITOR);
    settle(&mut server, 1000);
    editor.exchange(&mut server);
    let hidden = pixel(&mut renderer, &mut server, 110, 80);
    let typing_while_hidden = editor.keyboard_focus_on(0);
    dock.activate_app(&mut server, EDITOR);
    settle(&mut server, 1000);
    editor.exchange(&mut server);

    assert_eq!(hidden, BACKGROUND);
    assert!(!typing_while_hidden);
    assert_eq!(pixel(&mut renderer, &mut server, 110, 80), [0xff, 0, 0]);
    assert!(editor.keyboard_focus_on(0));
}

#[test]
fn super_h_hides_the_focused_app_and_show_app_brings_it_back_without_the_keyboard() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = window_of(&mut server, RED, (WINDOW, WINDOW), EDITOR);
    let mut menu_bar = shell(&mut server);
    settle(&mut server, 1000);

    super_key(&mut server, KEY_H);
    settle(&mut server, 1000);
    editor.exchange(&mut server);
    let hidden = pixel(&mut renderer, &mut server, 110, 80);
    menu_bar.show_app(&mut server, EDITOR);
    settle(&mut server, 1000);
    editor.exchange(&mut server);

    assert_eq!(hidden, BACKGROUND);
    assert_eq!(pixel(&mut renderer, &mut server, 110, 80), [0xff, 0, 0]);
    assert!(!editor.keyboard_focus_on(0));
}

#[test]
fn quitting_an_app_from_the_dock_asks_its_windows_to_close() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, (WINDOW, WINDOW), EDITOR);
    let mut dock = shell(&mut server);

    dock.quit_app(&mut server, EDITOR);
    editor.exchange(&mut server);

    assert!(editor.close_requested(0));
}

#[test]
fn a_hidden_app_is_told_it_is_suspended_until_it_shows_again() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, (WINDOW, WINDOW), EDITOR);
    let mut menu_bar = shell(&mut server);
    let suspended = |editor: &TestClient| {
        editor
            .configured(0)
            .is_some_and(|configured| configured.states.contains(&ToplevelState::Suspended))
    };

    menu_bar.hide_app(&mut server, EDITOR);
    editor.exchange(&mut server);
    let while_hidden = suspended(&editor);
    menu_bar.show_app(&mut server, EDITOR);
    editor.exchange(&mut server);

    assert!(while_hidden);
    assert!(!suspended(&editor));
}

#[test]
fn an_ordinary_app_can_ask_another_app_to_quit_without_seeing_the_shell() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, (WINDOW, WINDOW), EDITOR);
    let mut monitor = window_of(&mut server, RED, (WINDOW, WINDOW), "org.bande.Monitor");

    monitor.join_app_control(&mut server);
    monitor.ask_app_to_quit(&mut server, EDITOR);
    editor.exchange(&mut server);

    assert!(editor.close_requested(0));
    assert!(!monitor.close_requested(0));
    assert!(!monitor.advertises("bande_shell_v1"));
}

#[test]
fn the_dock_learns_which_windows_are_minimised_and_can_bring_one_back() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = window_of(&mut server, RED, (WINDOW, WINDOW), EDITOR);
    let mut dock = shell(&mut server);
    roundtrip(&mut server, &mut [&mut editor, &mut dock]);
    let identifier = dock
        .window_identifiers(EDITOR)
        .pop()
        .expect("a listed window");

    super_key(&mut server, KEY_M);
    settle(&mut server, 1000);
    dock.exchange(&mut server);
    let minimized = pixel(&mut renderer, &mut server, 110, 80);
    let late = shell(&mut server);
    dock.activate_window(&mut server, &identifier);
    settle(&mut server, 1000);
    roundtrip(&mut server, &mut [&mut editor, &mut dock]);

    assert_eq!(minimized, BACKGROUND);
    assert_eq!(
        dock.minimized_windows(),
        [(identifier.clone(), true), (identifier.clone(), false)]
    );
    assert_eq!(late.minimized_windows(), [(identifier, true)]);
    assert_eq!(pixel(&mut renderer, &mut server, 110, 80), [0xff, 0, 0]);
    assert!(editor.keyboard_focus_on(0));
}

#[test]
fn an_app_that_asks_to_come_forward_without_a_valid_token_asks_for_attention_instead() {
    let mut server = server();
    let mut background = window_of(&mut server, RED, (WINDOW, WINDOW), "org.example.Mail");
    let mut editor = window_of(&mut server, RED, (WINDOW, WINDOW), EDITOR);
    let mut dock = shell(&mut server);
    roundtrip(&mut server, &mut [&mut background, &mut editor, &mut dock]);

    let token = background.activation_token(&mut server);
    background.activate(&mut server, 0, &token);
    roundtrip(&mut server, &mut [&mut background, &mut editor, &mut dock]);

    assert_eq!(dock.attention_requests(), ["org.example.Mail"]);
    assert!(editor.keyboard_focus_on(0));
}

#[test]
fn super_f3_gives_the_dock_the_keyboard_and_super_f2_the_menu_bar() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, (WINDOW, WINDOW), EDITOR);
    let mut dock = shell(&mut server);
    let bar = |anchor, size| LayerRequest {
        layer: Layer::Top,
        anchor,
        size,
        exclusive_zone: 0,
        keyboard: KeyboardInteractivity::OnDemand,
        color: [0x80; 4],
    };
    dock.show_named_layer(&mut server, &bar(Anchor::Bottom, (128, 24)), "dock");
    dock.show_named_layer(
        &mut server,
        &bar(Anchor::Top | Anchor::Left | Anchor::Right, (0, 16)),
        "menubar",
    );

    super_key(&mut server, KEY_F3);
    roundtrip(&mut server, &mut [&mut editor, &mut dock]);
    let dock_has_keyboard = dock.layer_has_keyboard(0);
    super_key(&mut server, KEY_F2);
    roundtrip(&mut server, &mut [&mut editor, &mut dock]);

    assert!(dock_has_keyboard);
    assert!(dock.layer_has_keyboard(1));
    assert!(!editor.keyboard_focus_on(0));
    assert_eq!(dock.shell_focus_requests(), ["dock", "menubar"]);
}

#[test]
fn the_combined_bar_takes_the_keyboard_for_both_super_f2_and_super_f3() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, (WINDOW, WINDOW), EDITOR);
    let mut bar = shell(&mut server);
    bar.show_named_layer(
        &mut server,
        &LayerRequest {
            layer: Layer::Top,
            anchor: Anchor::Top | Anchor::Left | Anchor::Bottom,
            size: (64, 0),
            exclusive_zone: 64,
            keyboard: KeyboardInteractivity::OnDemand,
            color: [0x80; 4],
        },
        "bar",
    );

    super_key(&mut server, KEY_F3);
    roundtrip(&mut server, &mut [&mut editor, &mut bar]);
    let bar_has_keyboard = bar.layer_has_keyboard(0);
    super_key(&mut server, KEY_F2);
    roundtrip(&mut server, &mut [&mut editor, &mut bar]);

    assert!(bar_has_keyboard);
    assert!(bar.layer_has_keyboard(0));
    assert_eq!(bar.shell_focus_requests(), ["dock", "menubar"]);
}

#[test]
fn clicking_the_bar_keeps_the_active_app() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, (WINDOW, WINDOW), EDITOR);
    let mut bar = shell(&mut server);
    bar.show_named_layer(
        &mut server,
        &LayerRequest {
            layer: Layer::Top,
            anchor: Anchor::Top | Anchor::Left | Anchor::Bottom,
            size: (64, 0),
            exclusive_zone: 64,
            keyboard: KeyboardInteractivity::OnDemand,
            color: [0x80; 4],
        },
        "bar",
    );
    roundtrip(&mut server, &mut [&mut editor, &mut bar]);

    server.move_pointer_to(Point::from((10.0, 60.0)), 5);
    server.press_button(BTN_LEFT, true, 6);
    server.press_button(BTN_LEFT, false, 7);
    roundtrip(&mut server, &mut [&mut editor, &mut bar]);

    assert!(bar.layer_has_keyboard(0));
    assert_eq!(bar.active_apps(), [EDITOR]);
}

#[test]
fn ending_the_session_asks_every_app_to_quit_and_names_the_one_that_stays_until_it_closes() {
    let mut server = server();
    let mut editor = window_of(&mut server, RED, (WINDOW, WINDOW), EDITOR);
    let mut viewer = window_of(&mut server, RED, (WINDOW, WINDOW), VIEWER);
    let mut dialog = shell(&mut server);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer, &mut dialog]);

    dialog.quit_all_apps(&mut server);
    editor.exchange(&mut server);
    viewer.exchange(&mut server);
    let asked = (editor.close_requested(0), viewer.close_requested(0));
    viewer.close_window(&mut server, 0);
    dialog.exchange(&mut server);
    let keeping = dialog.apps_running().last().cloned();
    editor.close_window(&mut server, 0);
    dialog.exchange(&mut server);

    assert_eq!(asked, (true, true));
    assert_eq!(dialog.apps_running().first(), Some(&(VIEWER.to_owned(), 2)));
    assert_eq!(keeping, Some((EDITOR.to_owned(), 1)));
    assert_eq!(dialog.apps_running().last(), Some(&(String::new(), 0)));
}

#[test]
fn asking_to_end_the_session_starts_the_confirmation_as_a_shell_client() {
    let mut server = server();
    let mut menu_bar = shell(&mut server);
    server.set_session_dialog(std::path::Path::new("/bin/true"));

    menu_bar.confirm_session_end(&mut server, SessionEnd::Restart);
    let upkeep = server.keep_session_clients_running();

    assert!(upkeep.failures.is_empty());
    assert!(server.session_dialog_pid().is_some());
}
