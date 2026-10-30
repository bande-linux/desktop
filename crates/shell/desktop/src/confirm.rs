use std::cell::Cell;
use std::rc::Rc;

use accesskit::TreeUpdate;
use unbound_toolkit::ui::{Alert, Context, FreeLayout, Ui};
use unbound_toolkit::{
    App, Color, Edges, KeyboardFocus, LayerKind, LayerOptions, Role, Scene, TextSystem, Window,
    WindowError, WindowEvent,
};

use crate::L10N;

pub const CONFIRM_DELETE_FLAG: &str = "--confirm-delete";

struct Confirmation {
    ui: Ui,
    confirmed: Rc<Cell<bool>>,
}

impl App for Confirmation {
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

pub fn confirm_delete(count: usize) -> Result<bool, WindowError> {
    let mut window = Window::open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Overlay,
            anchor: Edges::ALL,
            exclusive_zone: -1,
            keyboard: KeyboardFocus::Exclusive,
            namespace: "desktop-confirmation".to_owned(),
        }),
        (0, 0),
    )?;
    let appearance = window.appearance();
    let mut ui = Ui::new(
        Box::new(FreeLayout::new()),
        Context::new(TextSystem::new(), appearance.palette()),
    );
    ui.set_appearance(appearance);
    let confirmed = Rc::new(Cell::new(false));
    let chosen = confirmed.clone();
    ui.present_dialog(
        Alert::new(
            &L10N.tr_args("delete-title", [("count", count.into())]),
            &L10N.tr("cannot-undo"),
            move |action| chosen.set(action == "delete"),
        )
        .cancel(&L10N.tr("cancel"), "cancel")
        .primary(&L10N.tr("delete"), "delete"),
    );
    let mut confirmation = Confirmation { ui, confirmed };
    unbound_toolkit::run(&mut window, &mut confirmation)?;
    Ok(confirmation.confirmed.get())
}
