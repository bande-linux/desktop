#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use bande_compositor::{Compositor, KeyboardSettings};
use bande_config::{CapsLock, InputSource};
use bande_test_wayland::{Frames, chord, headless};
use bande_toolkit::ui::{Context, TextField, Ui, Widget};
use bande_toolkit::{App, Palette, Role, Scene, Scheme, TextSystem, Window, WindowEvent};

const KEY_ALTGR: u32 = 100;
const KEY_SHIFT: u32 = 42;
const KEY_BACKSPACE: u32 = 14;
const KEY_SPACE: u32 = 57;
const KEY_A: u32 = 30;
const KEY_E: u32 = 18;
const KEY_X: u32 = 45;
const KEY_2: u32 = 3;
const KEY_4: u32 = 5;
const KEY_7: u32 = 8;
const KEY_8: u32 = 9;
const KEY_9: u32 = 10;
const KEY_0: u32 = 11;
const KEY_PLUS: u32 = 12;
const KEY_ACUTE: u32 = 13;
const KEY_DIAERESIS: u32 = 27;
const KEY_LESS: u32 = 86;

const TYPED: &str = "~ã~/´¨^`ëx@${[]}\\|";

struct Field {
    ui: Ui,
    text: Rc<RefCell<String>>,
    focused: mpsc::Sender<()>,
}

impl App for Field {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.ui.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        if matches!(event, WindowEvent::Focus(true)) {
            let _ = self.focused.send(());
        }
        window.drive(&mut self.ui, event).expect("drive the field");
    }

    fn finished(&self) -> bool {
        self.text.borrow().chars().count() >= TYPED.chars().count()
    }
}

fn swedish() -> KeyboardSettings {
    KeyboardSettings {
        input_sources: vec![InputSource {
            layout: "se".to_owned(),
            variant: None,
        }],
        caps_lock: CapsLock::default(),
        repeat_rate: 25,
        repeat_delay: Duration::from_secs(60),
    }
}

fn press(server: &mut Compositor, keys: &[u32]) {
    chord(server, keys);
    server.dispatch().expect("server dispatch");
}

#[test]
fn dead_keys_and_altgr_symbols_type_on_a_swedish_keyboard() {
    unsafe {
        std::env::set_var("LC_ALL", "C.UTF-8");
    }
    let mut server = headless(320, 240);
    server.apply_keyboard_settings(swedish());
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let (focused, has_focus) = mpsc::channel();
    let app = thread::spawn(move || {
        let mut window = Window::open_on(
            client_end,
            Role::toplevel("Typing", "bande-typing-test"),
            (200, 120),
        )
        .expect("window");
        let text = Rc::new(RefCell::new(String::new()));
        let written = text.clone();
        let field = TextField::new("", "", move |value| {
            *written.borrow_mut() = value.to_owned()
        });
        let id = field.id();
        let mut ui = Ui::new(
            Box::new(field),
            Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light)),
        );
        ui.set_focus(id);
        let mut app = Field {
            ui,
            text: text.clone(),
            focused,
        };
        bande_toolkit::run(&mut window, &mut app).expect("run the window");
        text.take()
    });

    let mut frames = Frames::start();
    frames.run_until(&mut server, || has_focus.try_recv().is_ok());
    let tilde: &[u32] = &[KEY_ALTGR, KEY_DIAERESIS];
    let script: &[&[u32]] = &[
        tilde,
        &[KEY_SPACE],
        tilde,
        &[KEY_A],
        tilde,
        &[KEY_SHIFT, KEY_7],
        &[KEY_ACUTE],
        &[KEY_SPACE],
        &[KEY_DIAERESIS],
        &[KEY_SPACE],
        &[KEY_SHIFT, KEY_DIAERESIS],
        &[KEY_SPACE],
        &[KEY_SHIFT, KEY_ACUTE],
        &[KEY_SPACE],
        &[KEY_DIAERESIS],
        &[KEY_E],
        &[KEY_DIAERESIS],
        &[KEY_BACKSPACE],
        &[KEY_X],
        &[KEY_ALTGR, KEY_2],
        &[KEY_ALTGR, KEY_4],
        &[KEY_ALTGR, KEY_7],
        &[KEY_ALTGR, KEY_8],
        &[KEY_ALTGR, KEY_9],
        &[KEY_ALTGR, KEY_0],
        &[KEY_ALTGR, KEY_PLUS],
        &[KEY_ALTGR, KEY_LESS],
    ];
    for keys in script {
        press(&mut server, keys);
    }
    frames.run_until(&mut server, || app.is_finished());
    assert_eq!(app.join().expect("app thread"), TYPED);
}
