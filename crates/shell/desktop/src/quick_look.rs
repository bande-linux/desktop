use std::os::fd::BorrowedFd;
use std::time::Instant;

use accesskit::TreeUpdate;
use bande_quick_look::{Action, Panel, QuickLookLink};
use bande_toolkit::ui::{Event, frame_time};
use bande_toolkit::{
    App, Color, Edges, KeyboardFocus, LayerKind, LayerOptions, Rect, Role, Scene, TextSystem,
    Transfer, Window, WindowEvent,
};
use bande_xdg::{file_uri, launch_with};

const NAMESPACE: &str = "quick-look";

pub fn quick_look_role() -> Role {
    Role::Layer(LayerOptions {
        layer: LayerKind::Overlay,
        anchor: Edges::ALL,
        exclusive_zone: -1,
        keyboard: KeyboardFocus::None,
        namespace: NAMESPACE.to_owned(),
    })
}

pub struct QuickLookSurface {
    panel: Panel,
}

impl QuickLookSurface {
    pub fn new(window: &Window, text: TextSystem, link: QuickLookLink) -> Self {
        Self {
            panel: Panel::new(link, text, window.appearance(), false),
        }
    }

    fn act(&mut self, window: &mut Window) {
        for action in self.panel.take_actions() {
            match action {
                Action::Open { app, path } => {
                    let token = window.activation_token(app.app_id()).ok();
                    if let Err(error) = launch_with(&app, &[path], token.as_deref()) {
                        tracing::warn!("cannot open with {}: {error}", app.id);
                    }
                }
                Action::Copy(path) => {
                    window.copy_transfer(Transfer::uris(vec![file_uri(&path)]));
                }
                Action::Move | Action::Resize(_) | Action::FullScreen(_) => {}
            }
        }
    }
}

impl App for QuickLookSurface {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let now = frame_time().presentation;
        let (width, height) = window.size();
        let bounds = Rect::new(0.0, 0.0, width as f32, height as f32);
        let items = self
            .panel
            .paint(bounds, window.scale() as f32, now)
            .unwrap_or_default();
        window.set_input_region(Some(&self.panel.input_region()));
        Scene {
            width,
            height,
            background: Color::TRANSPARENT,
            items,
        }
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let now = frame_time().on_screen();
        let handled = match event {
            WindowEvent::Input(input) => {
                if let Event::PointerMove(pointer) | Event::PointerEnter(pointer) = &input
                    && let Some(cursor) = self.panel.cursor(pointer.position)
                {
                    window.set_cursor(cursor);
                }
                self.panel.handle(&input, now)
            }
            WindowEvent::Accessibility(request) => self.panel.perform(&request, now),
            WindowEvent::AppearanceChanged(appearance) => {
                self.panel.set_appearance(appearance);
                true
            }
            _ => false,
        };
        self.act(window);
        if handled {
            window.request_redraw();
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.panel.sources()
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        if self.panel.source_ready(window, index) {
            window.request_redraw();
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.panel.deadline()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        if self.panel.tick(now) {
            window.request_redraw();
        }
    }

    fn animating(&self) -> bool {
        self.panel.animating(frame_time().presentation)
    }

    fn should_close(&mut self, _window: &mut Window) -> bool {
        false
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        self.panel.accessibility()
    }
}
