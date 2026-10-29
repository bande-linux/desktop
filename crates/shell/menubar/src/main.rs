use std::process::ExitCode;
use std::time::{Duration, Instant};

use unbound_menubar::{HEIGHT, MenuBar, app_name, clock_text};
use unbound_toolkit::ui::Event;
use unbound_toolkit::{
    App, Edges, KeyboardFocus, LayerKind, LayerOptions, Role, Scene, TextSystem, Window,
    WindowEvent,
};
use unbound_xdg::{Locale, applications, current_desktops, data_dirs};

const CLOCK_TICK: Duration = Duration::from_secs(1);

struct Bar {
    bar: MenuBar,
    names: Vec<(String, String)>,
    next_clock: Instant,
}

impl App for Bar {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.bar.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::ActiveApp(app_id) => {
                if self.bar.set_app(app_name(&app_id, &self.names)) {
                    window.request_redraw();
                }
            }
            WindowEvent::Input(Event::PointerUp(point)) if self.bar.control_center_hit(point) => {
                if let Err(error) = window.toggle_control_center() {
                    tracing::warn!("cannot open Control Center: {error}");
                }
            }
            WindowEvent::AppearanceChanged(appearance) => {
                self.bar.set_palette(appearance.palette());
            }
            _ => {}
        }
    }

    fn deadline(&self) -> Option<Instant> {
        Some(self.next_clock)
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        self.next_clock = now + CLOCK_TICK;
        if self.bar.set_clock(clock_text(&jiff::Zoned::now())) {
            window.request_redraw();
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let names = applications(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    )
    .into_iter()
    .map(|entry| (entry.app_id().to_owned(), entry.name))
    .collect();
    let mut window = Window::open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Top,
            anchor: Edges {
                top: true,
                left: true,
                right: true,
                bottom: false,
            },
            exclusive_zone: HEIGHT as i32,
            keyboard: KeyboardFocus::None,
            namespace: "menubar".to_owned(),
        }),
        (0, HEIGHT),
    )?;
    let mut bar = Bar {
        bar: MenuBar::new(
            window.appearance().palette(),
            TextSystem::new(),
            clock_text(&jiff::Zoned::now()),
        ),
        names,
        next_clock: Instant::now() + CLOCK_TICK,
    };
    unbound_toolkit::run(&mut window, &mut bar)?;
    Ok(())
}

fn main() -> ExitCode {
    unbound_log::init("unbound-menubar");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
