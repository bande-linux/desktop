use std::io::Read;
use std::ops::Range;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::Sender;

use smithay::utils::{Logical, Point};
use unbound_graphics::{Color, Event, Palette, Scene, Scheme, TextSystem};
use unbound_toolkit_platform::{App, HEADER_HEIGHT, Role, SHEET_INSET, Window, WindowEvent};
use unbound_widgets::{Context, TextField, Ui};

use crate::desktop::{Client, Desktop};

pub const WINDOW: (u32, u32) = (200, 140);
pub const TEXT_PREFIX: &str = "text:";
pub const BTN_LEFT: u32 = 0x110;

pub fn at(x: i32, y: i32) -> Point<f64, Logical> {
    Point::from((f64::from(x), f64::from(y)))
}

fn presses(events: &[WindowEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, WindowEvent::Input(Event::PointerDown(_))))
        .count()
}

pub struct Note {
    events: Sender<WindowEvent>,
    commands: UnixStream,
    finished: bool,
    ui: Ui,
}

pub fn note(
    text: &str,
    selected: Range<usize>,
) -> impl FnOnce(Sender<WindowEvent>, UnixStream) -> Note + Send + 'static {
    let text = text.to_owned();
    move |events, commands| {
        let changes = events.clone();
        let mut field = TextField::new("", &text, move |text| {
            let _ = changes.send(WindowEvent::MenuAction(format!("{TEXT_PREFIX}{text}")));
        });
        field.select(selected);
        let mut ui = Ui::new(
            Box::new(field),
            Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light)),
        );
        ui.layout(unbound_widgets::Size::new(WINDOW.0 as f32, WINDOW.1 as f32));
        Note {
            events,
            commands,
            finished: false,
            ui,
        }
    }
}

impl App for Note {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let (mut items, above) = self.ui.paint_layers(width, height);
        items.extend(above);
        Scene {
            width,
            height,
            background: Color::hex(0xFFFFFF),
            items,
        }
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let _ = self.events.send(event.clone());
        window.drive(&mut self.ui, event).expect("drive the ui");
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.commands.as_fd()]
    }

    fn source_ready(&mut self, _window: &mut Window, _index: usize) {
        let mut command = [0u8; 1];
        if self.commands.read(&mut command).unwrap_or(0) == 0 {
            self.finished = true;
        }
    }

    fn finished(&self) -> bool {
        self.finished
    }
}

pub fn text_of(events: &[WindowEvent]) -> Option<&str> {
    events.iter().rev().find_map(|event| match event {
        WindowEvent::MenuAction(action) => action.strip_prefix(TEXT_PREFIX),
        _ => None,
    })
}

impl Desktop {
    pub fn open_named<A: App>(
        &mut self,
        name: &str,
        app: impl FnOnce(Sender<WindowEvent>, UnixStream) -> A + Send + 'static,
    ) -> Client {
        let app_id = format!("test.{name}");
        let mut client = self.open_app(Role::toplevel(name, &app_id), false, WINDOW, app);
        self.wait_for(
            &mut [&mut client],
            |events| events.contains(&WindowEvent::Focus(true)),
            |desktop| desktop.window_of(&app_id).is_some() && desktop.mapped(),
        );
        client
    }

    pub fn window_of(&self, app_id: &str) -> Option<Point<i32, Logical>> {
        self.server
            .windows()
            .iter()
            .find(|window| window.app_id() == app_id && window.size().is_some())
            .map(|window| window.location())
    }

    pub fn slide(&mut self, client: &mut Client, name: &str, by: i32) {
        let app_id = format!("test.{name}");
        let before = self.window_of(&app_id).expect("mapped");
        let grip = at(before.x + 100, before.y + 14);
        self.server.move_pointer_to(grip, 0);
        self.server.press_button(BTN_LEFT, true, 0);
        self.settle(&mut [&mut *client]);
        self.server
            .move_pointer_to(at(before.x + 100 + by, before.y + 14), 0);
        self.server.press_button(BTN_LEFT, false, 0);
        self.wait_for(
            &mut [client],
            |_| true,
            |desktop| desktop.window_of(&app_id) == Some((before.x + by, before.y).into()),
        );
    }

    pub fn content_of(&self, name: &str, x: i32, y: i32) -> Point<f64, Logical> {
        let origin = self.window_of(&format!("test.{name}")).expect("mapped");
        at(
            origin.x + SHEET_INSET as i32 + x,
            origin.y + HEADER_HEIGHT as i32 + y,
        )
    }

    pub fn next_click_reaches(&mut self, client: &mut Client) {
        assert!(!self.server.pointer_grabbed(), "a grab outlived the drag");
        let before = presses(&client.seen);
        self.server.press_button(BTN_LEFT, true, 0);
        self.server.press_button(BTN_LEFT, false, 0);
        self.wait_for(&mut [client], |events| presses(events) > before, |_| true);
    }
}
