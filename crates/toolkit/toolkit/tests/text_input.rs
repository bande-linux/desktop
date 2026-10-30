#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, TreeId};
use smithay::utils::Size;
use unbound_compositor::{FrameClock, FrameScheduler, Headless, REFRESH, Renderer};
use unbound_test_wayland::{RED, ShmImage, TestClient};
use unbound_toolkit::ui::{Context, Event, TextField, Ui, Widget};
use unbound_toolkit::{
    App, HEADER_HEIGHT, Palette, Rect, Role, SHEET_INSET, Scene, Scheme, TextSystem, Window,
    WindowEvent,
};
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (320, 240);
const TIMEOUT: Duration = Duration::from_secs(60);
const READING: &str = "にほん";
const COMPOSED: &str = "日本";
const CANDIDATES: (i32, i32) = (40, 20);

struct Composing {
    area: Rect,
    value: String,
}

struct Field {
    ui: Ui,
    text: Rc<RefCell<String>>,
    composing: mpsc::Sender<Composing>,
}

impl App for Field {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.ui.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let preedit =
            matches!(&event, WindowEvent::Input(Event::Preedit { text, .. }) if !text.is_empty());
        window.drive(&mut self.ui, event).expect("drive the ui");
        if preedit {
            let scene = self.scene(window);
            window.draw(&scene).expect("draw the composition");
            if let Some(area) = self.ui.text_input_area() {
                let value = self.text.borrow().clone();
                self.composing.send(Composing { area, value }).ok();
            }
        }
    }

    fn finished(&self) -> bool {
        *self.text.borrow() == COMPOSED
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }
}

fn pixel(renderer: &mut Renderer, server: &mut Headless, x: i32, y: i32) -> [u8; 3] {
    renderer.render(server).expect("frame");
    let snapshot = renderer.snapshot(server).expect("read back");
    let at = ((y * snapshot.size.w + x) * 4) as usize;
    [
        snapshot.rgba[at],
        snapshot.rgba[at + 1],
        snapshot.rgba[at + 2],
    ]
}

#[test]
fn an_input_method_composes_in_place_with_candidates_below_the_text_and_commits() {
    let mut server = Headless::new(Size::from(OUTPUT)).expect("headless compositor");
    let mut input_method = TestClient::connect(&mut server);
    input_method.use_seat(&mut server);
    input_method.become_input_method(&mut server);

    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let (composing, compositions) = mpsc::channel();
    let app = thread::spawn(move || {
        let mut window = Window::open_on(
            client_end,
            Role::toplevel("Field", "unbound-toolkit-test"),
            (200, 60),
        )
        .expect("window");
        let text = Rc::new(RefCell::new(String::new()));
        let changed = text.clone();
        let field = TextField::new("Name", "", move |value| {
            *changed.borrow_mut() = value.to_owned();
        });
        let field_id = field.id();
        let mut ui = Ui::new(
            Box::new(field),
            Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light)),
        );
        ui.perform(&ActionRequest {
            action: Action::Focus,
            target_tree: TreeId::ROOT,
            target_node: field_id.into(),
            data: None,
        });
        let mut app = Field {
            ui,
            text,
            composing,
        };
        unbound_toolkit::run(&mut window, &mut app).expect("run the window");
        app.text.take()
    });

    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let mut scheduler = FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH));
    let start = Instant::now();
    let mut pump = |server: &mut Headless, input_method: &mut TestClient| {
        assert!(start.elapsed() < TIMEOUT, "the field never got the text");
        server.dispatch().expect("server dispatch");
        scheduler
            .poll(|| start.elapsed(), server, &mut renderer)
            .expect("compositor frame");
        input_method.exchange(server);
        thread::sleep(Duration::from_millis(1));
    };
    while !input_method.input_method_active() {
        pump(&mut server, &mut input_method);
    }
    input_method.set_preedit(
        &mut server,
        READING,
        (READING.len() as i32, READING.len() as i32),
    );
    let composition = loop {
        pump(&mut server, &mut input_method);
        if let Ok(composition) = compositions.try_recv() {
            break composition;
        }
    };
    assert_eq!(
        composition.value, "",
        "the reading is shown but not yet part of the value"
    );
    assert!(
        composition.area.width > 13.0 * 2.0,
        "the input area spans the reading being composed"
    );

    let candidates = RED.repeat((CANDIDATES.0 * CANDIDATES.1) as usize);
    input_method.show_candidates(
        &mut server,
        &ShmImage {
            width: CANDIDATES.0,
            height: CANDIDATES.1,
            stride: CANDIDATES.0 * 4,
            format: Format::Xrgb8888,
            bytes: &candidates,
        },
    );
    for _ in 0..20 {
        pump(&mut server, &mut input_method);
    }
    let window = server.windows()[0].location();
    let area = composition.area;
    let below = (
        window.x + SHEET_INSET as i32 + area.x.round() as i32 + 4,
        window.y + HEADER_HEIGHT as i32 + (area.y + area.height).round() as i32 + 4,
    );
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    assert_eq!(
        pixel(&mut renderer, &mut server, below.0, below.1),
        [RED[2], RED[1], RED[0]],
        "the candidates open just below the text being composed"
    );

    input_method.commit_text(&mut server, COMPOSED);
    while !app.is_finished() {
        pump(&mut server, &mut input_method);
    }
    assert_eq!(app.join().expect("app thread"), COMPOSED);
}
