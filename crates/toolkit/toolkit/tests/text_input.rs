use std::cell::RefCell;
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, TreeId};
use smithay::utils::Size;
use unbound_compositor::{FrameClock, FrameScheduler, Headless, REFRESH, Renderer};
use unbound_test_wayland::TestClient;
use unbound_toolkit::ui::{Context, TextField, Ui, Widget};
use unbound_toolkit::{App, Palette, Rect, Role, Scene, Scheme, TextSystem, Window, WindowEvent};

const OUTPUT: (i32, i32) = (320, 240);
const TIMEOUT: Duration = Duration::from_secs(60);
const COMPOSED: &str = "日本語";

struct Field {
    ui: Ui,
    text: Rc<RefCell<String>>,
}

impl App for Field {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.ui.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        if let WindowEvent::Input(input) = event
            && self.ui.handle(&input)
        {
            window.request_redraw();
        }
    }

    fn finished(&self) -> bool {
        *self.text.borrow() == COMPOSED
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }
}

#[test]
fn text_committed_by_an_input_method_lands_in_the_focused_field() {
    let mut server = Headless::new(Size::from(OUTPUT)).expect("headless compositor");
    let mut input_method = TestClient::connect(&mut server);
    input_method.use_seat(&mut server);
    input_method.become_input_method(&mut server);

    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
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
        let mut app = Field { ui, text };
        unbound_toolkit::run(&mut window, &mut app).expect("run the window");
        app.text.take()
    });

    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let mut scheduler = FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH));
    let start = Instant::now();
    let mut committed = false;
    while !app.is_finished() {
        assert!(start.elapsed() < TIMEOUT, "the field never got the text");
        server.dispatch().expect("server dispatch");
        scheduler
            .poll(|| start.elapsed(), &mut server, &mut renderer)
            .expect("compositor frame");
        input_method.exchange(&mut server);
        if input_method.input_method_active() && !committed {
            input_method.commit_text(&mut server, COMPOSED);
            committed = true;
        }
        thread::sleep(Duration::from_millis(1));
    }

    assert!(committed, "the focused field activated the input method");
    assert_eq!(app.join().expect("app thread"), COMPOSED);
}
