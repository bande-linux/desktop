use accesskit::TreeUpdate;
use unbound_graphics::{Color, Scene, TextSystem};
use unbound_toolkit_platform::{
    App, Edges, KeyboardFocus, LayerKind, LayerOptions, Role, Window, WindowError, WindowEvent, run,
};
use unbound_widgets::{Alert, Context, FreeLayout, Ui};

struct Asking {
    ui: Ui,
}

impl App for Asking {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let mut scene = self.ui.scene(width, height);
        scene.background = Color::TRANSPARENT;
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let changed = match event {
            WindowEvent::Input(input) => self.ui.handle(&input),
            WindowEvent::Accessibility(request) => self.ui.perform(&request),
            WindowEvent::AppearanceChanged(appearance) => {
                self.ui.set_appearance(appearance);
                true
            }
            _ => false,
        };
        if changed {
            window.request_redraw();
        }
    }

    fn finished(&self) -> bool {
        !self.ui.has_sheet() && !self.ui.is_animating()
    }

    fn should_close(&mut self, _window: &mut Window) -> bool {
        true
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }
}

pub fn ask(namespace: &str, alert: impl FnOnce(&Window) -> Alert) -> Result<(), WindowError> {
    let mut window = Window::open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Overlay,
            anchor: Edges::ALL,
            exclusive_zone: -1,
            keyboard: KeyboardFocus::Exclusive,
            namespace: namespace.to_owned(),
        }),
        (0, 0),
    )?;
    let appearance = window.appearance();
    let mut ui = Ui::new(
        Box::new(FreeLayout::new()),
        Context::new(TextSystem::new(), appearance.palette()),
    );
    ui.set_appearance(appearance);
    ui.present_dialog(alert(&window));
    run(&mut window, &mut Asking { ui })
}
