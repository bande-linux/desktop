#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::{Cell, RefCell};
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use bande_compositor::KeyboardSettings;
use bande_config::{CapsLock, InputSource};
use bande_test_wayland::{Frames, chord, headless};
use bande_toolkit::ui::{Context, Event, Key, TextField, Ui, Widget};
use bande_toolkit::{App, Palette, Role, Scene, Scheme, TextSystem, Window, WindowEvent};

const KEY_ALTGR: u32 = 100;
const KEY_SHIFT: u32 = 42;
const KEY_BACKSPACE: u32 = 14;
const KEY_ENTER: u32 = 28;
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
const KEY_APOSTROPHE: u32 = 40;
const KEY_LESS: u32 = 86;

struct Field {
    ui: Ui,
    entered: Rc<Cell<bool>>,
    focused: mpsc::Sender<()>,
}

impl App for Field {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.ui.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::Focus(true) => {
                let _ = self.focused.send(());
            }
            WindowEvent::Input(Event::Key {
                key: Key::Enter, ..
            }) => self.entered.set(true),
            _ => {}
        }
        window.drive(&mut self.ui, event).expect("drive the field");
    }

    fn finished(&self) -> bool {
        self.entered.get()
    }
}

fn typed(layout: &str, variant: Option<&str>, script: &[&[u32]]) -> String {
    unsafe {
        std::env::set_var("LC_ALL", "C.UTF-8");
    }
    let mut server = headless(320, 240);
    server.apply_keyboard_settings(KeyboardSettings {
        input_sources: vec![InputSource {
            layout: layout.to_owned(),
            variant: variant.map(str::to_owned),
        }],
        caps_lock: CapsLock::default(),
        repeat_rate: 25,
        repeat_delay: Duration::from_secs(60),
    });
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
            entered: Rc::default(),
            focused,
        };
        bande_toolkit::run(&mut window, &mut app).expect("run the window");
        text.take()
    });

    let mut frames = Frames::start();
    frames.run_until(&mut server, || has_focus.try_recv().is_ok());
    for keys in script.iter().copied().chain([[KEY_ENTER].as_slice()]) {
        chord(&mut server, keys);
        server.dispatch().expect("server dispatch");
    }
    frames.run_until(&mut server, || app.is_finished());
    app.join().expect("app thread")
}

#[test]
fn dead_keys_and_altgr_symbols_type_on_a_swedish_keyboard() {
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
    assert_eq!(typed("se", None, script), "~ã~/´¨^`ëx@${[]}\\|");
}

#[test]
fn a_dead_key_and_space_type_the_quote_on_a_layout_without_a_quote_key() {
    let script: &[&[u32]] = &[
        &[KEY_APOSTROPHE],
        &[KEY_SPACE],
        &[KEY_SHIFT, KEY_APOSTROPHE],
        &[KEY_SPACE],
        &[KEY_APOSTROPHE],
        &[KEY_E],
    ];
    assert_eq!(typed("us", Some("intl"), script), "'\"é");
}
