mod combined_client;
mod dialogs;
mod dock_client;
mod menu_bar_client;

use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use accesskit::TreeUpdate;
use unbound_bar::dock::surface_size;
use unbound_bar::rail::RailView;
use unbound_config::{Config, DockEdge, MenuBarPlacement, config_path};
use unbound_status::session::Ending;
use unbound_toolkit::{App, Scene, Surfaces, TextSystem, Window, WindowEvent};
use unbound_xdg::data_dirs;
use zbus::blocking::Connection;

use crate::combined_client::{CombinedClient, combined_role};
use crate::dock_client::{Dock, dock_role};
use crate::menu_bar_client::{MENU_BAR_SIZE, MenuBarClient, menu_bar_role};

const SECONDARY_DISPLAY: &str = "UNBOUND_SECONDARY_DISPLAY";

struct Bar {
    windows: Vec<Window>,
    parts: Vec<Box<dyn App>>,
}

impl Bar {
    fn part_of_source(&self, mut index: usize) -> Option<(usize, usize)> {
        for (part, app) in self.parts.iter().enumerate() {
            let count = app.sources().len();
            if index < count {
                return Some((part, index));
            }
            index -= count;
        }
        None
    }
}

impl Surfaces for Bar {
    fn windows(&self) -> &[Window] {
        &self.windows
    }

    fn windows_mut(&mut self) -> &mut [Window] {
        &mut self.windows
    }

    fn scene(&mut self, index: usize) -> Scene {
        self.parts[index].scene(&mut self.windows[index])
    }

    fn event(&mut self, index: usize, event: WindowEvent) {
        self.parts[index].event(&mut self.windows[index], event);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.parts.iter().flat_map(|part| part.sources()).collect()
    }

    fn source_ready(&mut self, index: usize) {
        if let Some((part, local)) = self.part_of_source(index) {
            self.parts[part].source_ready(&mut self.windows[part], local);
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.parts.iter().filter_map(|part| part.deadline()).min()
    }

    fn tick(&mut self, now: Instant) {
        for (part, window) in self.parts.iter_mut().zip(&mut self.windows) {
            if part.deadline().is_some_and(|deadline| deadline <= now) {
                part.tick(window, now);
            }
        }
    }

    fn animating(&self, index: usize) -> bool {
        self.parts[index].animating()
    }

    fn accessibility(&self, index: usize) -> Option<TreeUpdate> {
        self.parts[index].accessibility()
    }
}

fn read_config(config_home: Option<&PathBuf>, home: Option<&PathBuf>) -> Config {
    config_path(
        config_home.map(PathBuf::as_path),
        home.map(PathBuf::as_path),
    )
    .and_then(|path| std::fs::read_to_string(path).ok())
    .and_then(|source| Config::parse(&source).ok())
    .unwrap_or_default()
}

fn bus(connection: zbus::Result<Connection>, name: &str) -> Option<Connection> {
    connection
        .inspect_err(|error| tracing::warn!("no {name} bus: {error}"))
        .ok()
}

fn run_bar() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config_home = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let config = read_config(config_home.as_ref(), home.as_ref());
    let session = bus(Connection::session(), "session");
    let system = bus(Connection::system(), "system");
    let text = TextSystem::new();
    if config.bar.menu_bar == MenuBarPlacement::Combined {
        let dock_config = unbound_config::Dock {
            edge: DockEdge::Left,
            ..config.dock.clone()
        };
        let icon_size = dock_config.icon_size as f32;
        let (role, size) = combined_role(icon_size);
        let mut window = Window::open(role, size)?;
        window.share_fonts(&text);
        let dock = Dock::new(
            &window,
            &dock_config,
            text.sharing_fonts(),
            session.as_ref(),
            (config_home.as_deref(), home.as_deref()),
        )?;
        let view = RailView::new(
            window.appearance().palette(),
            text.sharing_fonts(),
            data_dirs(),
            icon_size,
        );
        let status = MenuBarClient::with_view(view, session.as_ref(), system.as_ref())?;
        let mut bar = Bar {
            windows: vec![window],
            parts: vec![Box::new(CombinedClient::new(dock, status))],
        };
        unbound_toolkit::run_surfaces(&mut bar)?;
        return Ok(());
    }
    let mut menu_bar = Window::open(menu_bar_role(), MENU_BAR_SIZE)?;
    menu_bar.share_fonts(&text);
    let client = MenuBarClient::new(
        &mut menu_bar,
        text.sharing_fonts(),
        session.as_ref(),
        system.as_ref(),
    )?;
    let mut bar = Bar {
        windows: vec![menu_bar],
        parts: vec![Box::new(client)],
    };
    if std::env::var_os(SECONDARY_DISPLAY).is_none() {
        let dock_config = &config.dock;
        let icon_size = dock_config.icon_size as f32;
        let mut window = bar.windows[0].open_beside(
            dock_role(dock_config.edge, icon_size),
            surface_size(dock_config.edge, icon_size, (1, 1)),
        )?;
        window.share_fonts(&text);
        let dock = Dock::new(
            &window,
            dock_config,
            text.sharing_fonts(),
            session.as_ref(),
            (config_home.as_deref(), home.as_deref()),
        )?;
        bar.windows.push(window);
        bar.parts.push(Box::new(dock));
    }
    unbound_toolkit::run_surfaces(&mut bar)?;
    Ok(())
}

fn main() -> ExitCode {
    unbound_log::init("unbound-bar");
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let result = match arguments
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--about"] => dialogs::run_about(),
        ["--about-app", app_id, version] => dialogs::run_about_app(app_id, version),
        ["--confirm", keyword] => match Ending::from_keyword(keyword) {
            Some(ending) => dialogs::run_confirmation(ending),
            None => Err(format!("unknown session ending {keyword}").into()),
        },
        _ => run_bar(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
