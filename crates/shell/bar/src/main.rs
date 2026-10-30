mod combined_client;
mod dialogs;
mod dock_client;
mod menu_bar_client;
mod menu_search_client;

use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use accesskit::TreeUpdate;
use unbound_bar::dock::surface_size;
use unbound_bar::rail::RailView;
use unbound_config::{Config, config_path};
use unbound_status::session::Ending;
use unbound_toolkit::{App, Role, Scene, Surfaces, TextSystem, Window, WindowError, WindowEvent};
use unbound_xdg::data_dirs;
use zbus::blocking::Connection;

use crate::combined_client::{CombinedClient, combined_role};
use crate::dock_client::{Dock, dock_role};
use crate::menu_bar_client::{MENU_BAR_SIZE, MenuBarClient, Restart, menu_bar_role};
use crate::menu_search_client::{SearchCard, search_role};

const SECONDARY_DISPLAY: &str = "UNBOUND_SECONDARY_DISPLAY";

struct Context {
    text: TextSystem,
    session: Option<Connection>,
    system: Option<Connection>,
    config_home: Option<PathBuf>,
    home: Option<PathBuf>,
}

struct Bar {
    context: Context,
    windows: Vec<Window>,
    parts: Vec<Box<dyn App>>,
    restart: Restart,
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

    fn rebuild(&mut self) {
        let Some(first) = self.windows.first() else {
            return;
        };
        let config = read_config(&self.context);
        match build(&self.context, &config, |role, size| {
            first.open_beside(role, size)
        }) {
            Ok((windows, parts, restart)) => {
                self.parts = parts;
                self.windows = windows;
                self.restart = restart;
            }
            Err(error) => {
                tracing::warn!("cannot rearrange the bar: {error}");
                self.restart.set(false);
            }
        }
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
        if self.restart.get() {
            self.rebuild();
        }
    }

    fn animating(&self, index: usize) -> bool {
        self.parts[index].animating()
    }

    fn accessibility(&self, index: usize) -> Option<TreeUpdate> {
        self.parts[index].accessibility()
    }
}

fn read_config(context: &Context) -> Config {
    config_path(context.config_home.as_deref(), context.home.as_deref())
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|source| Config::load(&source).ok())
        .map(|(config, _)| config)
        .unwrap_or_default()
}

fn bus(connection: zbus::Result<Connection>, name: &str) -> Option<Connection> {
    connection
        .inspect_err(|error| tracing::warn!("no {name} bus: {error}"))
        .ok()
}

type Built = (Vec<Window>, Vec<Box<dyn App>>, Restart);

fn build(
    context: &Context,
    config: &Config,
    open: impl FnOnce(Role, (u32, u32)) -> Result<Window, WindowError>,
) -> Result<Built, Box<dyn std::error::Error>> {
    let text = &context.text;
    let session = context.session.as_ref();
    let system = context.system.as_ref();
    let homes = (context.config_home.as_deref(), context.home.as_deref());
    let bar = &config.bar;
    let layout = (bar.menu_bar, bar.edge);
    let icon_size = bar.icon_size as f32;
    let mut windows = Vec::new();
    let mut parts: Vec<Box<dyn App>> = Vec::new();
    let status = match bar.menu_bar.combines(bar.edge) {
        true => {
            let (role, size) = combined_role(icon_size);
            let mut window = open(role, size)?;
            window.share_fonts(text);
            let dock = Dock::new(&window, bar, text.sharing_fonts(), session, homes)?;
            let view = RailView::new(
                window.appearance().palette(),
                text.sharing_fonts(),
                data_dirs(),
                icon_size,
            );
            let status = MenuBarClient::with_view(view, session, system, layout)?;
            let shared = (status.handoff(), status.restart());
            windows.push(window);
            parts.push(Box::new(CombinedClient::new(dock, status)));
            shared
        }
        false => {
            let mut window = open(menu_bar_role(), MENU_BAR_SIZE)?;
            window.share_fonts(text);
            let client =
                MenuBarClient::new(&mut window, text.sharing_fonts(), session, system, layout)?;
            let shared = (client.handoff(), client.restart());
            windows.push(window);
            parts.push(Box::new(client));
            if std::env::var_os(SECONDARY_DISPLAY).is_none() {
                let mut window = windows[0].open_beside(
                    dock_role(bar.edge, icon_size),
                    surface_size(bar.edge, icon_size, (1, 1)),
                )?;
                window.share_fonts(text);
                let dock = Dock::new(&window, bar, text.sharing_fonts(), session, homes)?;
                windows.push(window);
                parts.push(Box::new(dock));
            }
            shared
        }
    };
    let (handoff, restart) = status;
    let mut window = windows[0].open_beside(search_role(), (0, 0))?;
    window.share_fonts(text);
    let card = SearchCard::new(&mut window, text.sharing_fonts(), handoff);
    windows.push(window);
    parts.push(Box::new(card));
    Ok((windows, parts, restart))
}

fn run_bar() -> Result<(), Box<dyn std::error::Error>> {
    let context = Context {
        text: TextSystem::new(),
        session: bus(Connection::session(), "session"),
        system: bus(Connection::system(), "system"),
        config_home: std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        home: std::env::var_os("HOME").map(PathBuf::from),
    };
    let config = read_config(&context);
    let (windows, parts, restart) = build(&context, &config, Window::open)?;
    let mut bar = Bar {
        context,
        windows,
        parts,
        restart,
    };
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
