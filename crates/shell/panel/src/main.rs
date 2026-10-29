use std::cell::RefCell;
use std::collections::HashMap;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;
use unbound_apps::{DesktopEntry, Locale, applications, current_desktops, data_dirs, find_icon};
use unbound_config::{Appearance, Config, PanelEdge, config_path};
use unbound_panel::{
    Actions, DockApp, ICON_SIZE, PanelAction, THICKNESS, clock_text, merge_apps, panel_ui,
};
use unbound_toolkit::ui::Ui;
use unbound_toolkit::{
    App, Edges, Image, KeyboardFocus, LayerKind, LayerOptions, Palette, Role, Scene, Scheme,
    TextSystem, Window, WindowError, WindowEvent,
};

const PINNED: &[&str] = &["unbound-term"];

struct Panel {
    edge: PanelEdge,
    palette: Palette,
    entries: Vec<DesktopEntry>,
    icons: HashMap<String, Option<Image>>,
    actions: Actions,
    ui: Option<Ui>,
    running: Vec<String>,
    next_minute: Instant,
}

impl Panel {
    fn icon(&mut self, name: Option<&str>) -> Option<Image> {
        let name = name?;
        self.icons
            .entry(name.to_owned())
            .or_insert_with(|| {
                let path = find_icon(name, ICON_SIZE, &[], &data_dirs())?;
                unbound_apps::load_icon(&path, ICON_SIZE)
            })
            .clone()
    }

    fn describe(&mut self, app_id: &str) -> DockApp {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.app_id() == app_id)
            .cloned();
        match entry {
            Some(entry) => DockApp {
                app_id: app_id.to_owned(),
                name: entry.name.clone(),
                icon: self.icon(entry.icon.as_deref()),
                running: false,
            },
            None => DockApp {
                app_id: app_id.to_owned(),
                name: app_id.rsplit('.').next().unwrap_or(app_id).to_owned(),
                icon: None,
                running: false,
            },
        }
    }

    fn rebuild(&mut self) {
        let pinned: Vec<DockApp> = PINNED.iter().map(|app_id| self.describe(app_id)).collect();
        let running = self.running.clone();
        let described: HashMap<String, DockApp> = running
            .iter()
            .map(|app_id| (app_id.clone(), self.describe(app_id)))
            .collect();
        let apps = merge_apps(&pinned, &running, |app_id| {
            described.get(app_id).cloned().unwrap_or_else(|| DockApp {
                app_id: app_id.to_owned(),
                name: app_id.to_owned(),
                icon: None,
                running: true,
            })
        });
        let now = jiff::Zoned::now();
        self.ui = Some(panel_ui(
            self.edge,
            apps,
            &clock_text(&now),
            self.palette,
            TextSystem::new(),
            self.actions.clone(),
        ));
    }

    fn run_actions(&mut self, window: &mut Window) {
        let actions = std::mem::take(&mut *self.actions.borrow_mut());
        for action in actions {
            match action {
                PanelAction::Open(app_id) => self.open(window, &app_id),
            }
        }
    }

    fn open(&mut self, window: &mut Window, app_id: &str) {
        if self.running.iter().any(|running| running == app_id) {
            if let Err(error) = window.activate_app(app_id) {
                eprintln!("unbound-panel: cannot activate {app_id}: {error}");
            }
            return;
        }
        let Some(entry) = self.entries.iter().find(|entry| entry.app_id() == app_id) else {
            return;
        };
        let token = window.activation_token(app_id).ok();
        if let Err(error) = unbound_apps::launch(entry, token.as_deref()) {
            eprintln!("unbound-panel: cannot open {}: {error}", entry.name);
        }
    }
}

fn next_minute() -> Instant {
    let now = jiff::Zoned::now();
    let into_minute = Duration::from_nanos(
        u64::try_from(now.second()).unwrap_or(0) * 1_000_000_000
            + u64::try_from(now.subsec_nanosecond()).unwrap_or(0),
    );
    Instant::now() + Duration::from_secs(60).saturating_sub(into_minute)
}

impl App for Panel {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        match &mut self.ui {
            Some(ui) => ui.scene(width, height),
            None => Scene {
                width,
                height,
                background: self.palette.surface_elevated,
                items: Vec::new(),
            },
        }
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::Input(input) => {
                if let Some(ui) = &mut self.ui
                    && ui.handle(&input)
                {
                    window.request_redraw();
                }
                self.run_actions(window);
            }
            WindowEvent::Accessibility(request) => {
                if let Some(ui) = &mut self.ui
                    && ui.perform(&request)
                {
                    window.request_redraw();
                }
                self.run_actions(window);
            }
            WindowEvent::AppsChanged => {
                self.running = window.running_apps();
                self.rebuild();
                window.request_redraw();
            }
            _ => {}
        }
    }

    fn deadline(&self) -> Option<Instant> {
        Some(self.next_minute)
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        self.ui.as_ref().map(Ui::accessibility)
    }

    fn tick(&mut self, window: &mut Window, _now: Instant) {
        self.next_minute = next_minute();
        self.rebuild();
        window.request_redraw();
    }
}

fn run() -> Result<(), WindowError> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let config_home = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from);
    let config = config_path(config_home.as_deref(), home.as_deref())
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|source| Config::parse(&source).ok())
        .unwrap_or_default();
    let edge = config.panel_edge;
    let anchor = match edge {
        PanelEdge::Left => Edges {
            left: true,
            top: true,
            bottom: true,
            right: false,
        },
        PanelEdge::Right => Edges {
            right: true,
            top: true,
            bottom: true,
            left: false,
        },
        PanelEdge::Top => Edges {
            top: true,
            left: true,
            right: true,
            bottom: false,
        },
        PanelEdge::Bottom => Edges {
            bottom: true,
            left: true,
            right: true,
            top: false,
        },
    };
    let scheme = match config.appearance {
        Appearance::Dark => Scheme::Dark,
        Appearance::Light | Appearance::Automatic => Scheme::Light,
    };
    let mut window = Window::open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Top,
            anchor,
            exclusive_zone: THICKNESS as i32,
            keyboard: KeyboardFocus::None,
            namespace: "panel".to_owned(),
        }),
        (THICKNESS, THICKNESS),
    )?;
    let locale = Locale::from_env();
    let mut panel = Panel {
        edge,
        palette: Palette::for_scheme(scheme),
        entries: applications(&data_dirs(), locale.as_ref(), &current_desktops()),
        icons: HashMap::new(),
        actions: Rc::new(RefCell::new(Vec::new())),
        ui: None,
        running: window.running_apps(),
        next_minute: next_minute(),
    };
    panel.rebuild();
    unbound_toolkit::run(&mut window, &mut panel)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("unbound-panel: {error}");
            ExitCode::FAILURE
        }
    }
}
