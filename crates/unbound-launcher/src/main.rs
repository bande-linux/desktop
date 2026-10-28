use std::collections::HashMap;
use std::process::ExitCode;

use unbound_apps::{
    DesktopEntry, Locale, applications, current_desktops, data_dirs, find_icon, launch, load_icon,
};
use unbound_config::{Appearance, Config, config_path};
use unbound_launcher::{ICON_SIZE, Launcher, LauncherAction, LauncherApp};
use unbound_toolkit::{
    App, Edges, Image, KeyboardFocus, LayerKind, LayerOptions, Palette, Role, Scene, Scheme,
    TextSystem, Window, WindowError, WindowEvent,
};

struct LauncherShell {
    launcher: Launcher,
    entries: Vec<DesktopEntry>,
}

impl LauncherShell {
    fn close(&mut self, window: &mut Window) {
        if let Err(error) = window.hide() {
            eprintln!("unbound-launcher: cannot hide: {error}");
        }
        self.launcher.clear();
    }

    fn open(&mut self, window: &mut Window, app_id: &str) {
        let token = window.activation_token(app_id).ok();
        self.close(window);
        if let Some(entry) = self.entries.iter().find(|entry| entry.app_id() == app_id)
            && let Err(error) = launch(entry, token.as_deref())
        {
            eprintln!("unbound-launcher: cannot open {}: {error}", entry.name);
        }
    }
}

impl App for LauncherShell {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.launcher.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::ShowLauncher if window.is_visible() => self.close(window),
            WindowEvent::ShowLauncher => {
                if let Err(error) = window.show() {
                    eprintln!("unbound-launcher: cannot show: {error}");
                }
            }
            WindowEvent::Input(input) => {
                if self.launcher.handle(&input) {
                    window.request_redraw();
                }
                for action in self.launcher.take_actions() {
                    match action {
                        LauncherAction::Open(app_id) => self.open(window, &app_id),
                        LauncherAction::Close => self.close(window),
                    }
                }
            }
            _ => {}
        }
    }
}

fn icons(entries: &[DesktopEntry]) -> HashMap<String, Option<Image>> {
    let dirs = data_dirs();
    let mut icons = HashMap::new();
    for name in entries.iter().filter_map(|entry| entry.icon.as_deref()) {
        icons.entry(name.to_owned()).or_insert_with(|| {
            find_icon(name, ICON_SIZE, &[], &dirs).and_then(|path| load_icon(&path, ICON_SIZE))
        });
    }
    icons
}

fn run() -> Result<(), WindowError> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let config_home = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from);
    let config = config_path(config_home.as_deref(), home.as_deref())
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|source| Config::parse(&source).ok())
        .unwrap_or_default();
    let scheme = match config.appearance {
        Appearance::Dark => Scheme::Dark,
        Appearance::Light | Appearance::Automatic => Scheme::Light,
    };
    let entries = applications(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    );
    let icons = icons(&entries);
    let apps = entries
        .iter()
        .map(|entry| LauncherApp {
            app_id: entry.app_id().to_owned(),
            name: entry.name.clone(),
            generic_name: entry.generic_name.clone(),
            comment: entry.comment.clone(),
            keywords: entry.keywords.clone(),
            icon: entry
                .icon
                .as_ref()
                .and_then(|name| icons.get(name).cloned().flatten()),
        })
        .collect();
    let mut window = Window::open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Overlay,
            anchor: Edges::ALL,
            exclusive_zone: -1,
            keyboard: KeyboardFocus::Exclusive,
            namespace: "launcher".to_owned(),
        }),
        (0, 0),
    )?;
    window.hide()?;
    let mut shell = LauncherShell {
        launcher: Launcher::new(apps, Palette::for_scheme(scheme), TextSystem::new()),
        entries,
    };
    unbound_toolkit::run(&mut window, &mut shell)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("unbound-launcher: {error}");
            ExitCode::FAILURE
        }
    }
}
