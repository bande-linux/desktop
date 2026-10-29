use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use unbound_apps::{Locale, applications, current_desktops, data_dirs};
use unbound_config::{Appearance, Config, config_path};
use unbound_menubar::{HEIGHT, MenuBar, app_name, clock_text};
use unbound_toolkit::{
    App, Edges, KeyboardFocus, LayerKind, LayerOptions, Palette, Role, Scene, Scheme, TextSystem,
    Window, WindowEvent,
};

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
        if let WindowEvent::ActiveApp(app_id) = event
            && self.bar.set_app(app_name(&app_id, &self.names))
        {
            window.request_redraw();
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

fn scheme() -> Scheme {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config_home = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let config = config_path(config_home.as_deref(), home.as_deref())
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|source| Config::parse(&source).ok())
        .unwrap_or_default();
    match config.appearance {
        Appearance::Dark => Scheme::Dark,
        Appearance::Light | Appearance::Automatic => Scheme::Light,
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
            Palette::for_scheme(scheme()),
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
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("unbound-menubar: {error}");
            ExitCode::FAILURE
        }
    }
}
