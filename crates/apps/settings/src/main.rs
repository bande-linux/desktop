use std::os::fd::BorrowedFd;
use std::process::ExitCode;

use unbound_settings::{Displays, Feed, L10N, PaneId, Services, SettingsView, feed};
use unbound_toolkit::{App, Rect, Role, Scene, TextSystem, Window, WindowEvent};

const APP_ID: &str = "org.unbound.Settings";
const WINDOW_SIZE: (u32, u32) = (900, 600);
const FEED: usize = 0;

struct Settings {
    view: SettingsView,
    feed: Feed,
    displays: Option<Displays>,
    services: Services,
}

impl Settings {
    fn send_commands(&mut self) {
        for command in self.view.take_commands() {
            self.services.send(command);
        }
    }
}

impl App for Settings {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.view.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::Input(input) => {
                self.view.handle(&input);
            }
            WindowEvent::Accessibility(request) => {
                self.view.perform(&request);
            }
            WindowEvent::AppearanceChanged(appearance) => {
                self.view.set_palette(appearance.palette());
            }
            _ => return,
        }
        self.send_commands();
        window.request_redraw();
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        let mut sources = vec![self.feed.fd()];
        sources.extend(self.displays.as_ref().map(Displays::fd));
        sources
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        if index == FEED {
            for update in self.feed.drain() {
                self.view.apply(update);
            }
        } else if let Some(displays) = self.displays.as_mut().and_then(Displays::dispatch) {
            self.view.set_displays(displays);
        }
        self.send_commands();
        window.request_redraw();
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.view.text_input_area()
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.view.accessibility())
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let session = zbus::blocking::Connection::session()?;
    let system = zbus::blocking::Connection::system()
        .inspect_err(|error| tracing::warn!("no system bus: {error}"))
        .ok();
    let (feed, updates) = feed()?;
    let services = Services::start(&session, system.as_ref(), &updates);
    let mut window = Window::open(Role::toplevel(&L10N.tr("title"), APP_ID), WINDOW_SIZE)?;
    let mut view = SettingsView::new(window.appearance().palette(), TextSystem::new());
    let requested = std::env::args().nth(1);
    match requested.as_deref().map(PaneId::from_keyword) {
        Some(Some(pane)) => view.select_pane(pane),
        Some(None) => tracing::warn!("no settings pane {requested:?}"),
        None => {}
    }
    let displays = Displays::connect();
    if let Some(displays) = &displays {
        view.set_displays(displays.list());
    }
    let mut settings = Settings {
        view,
        feed,
        displays,
        services,
    };
    unbound_toolkit::run(&mut window, &mut settings)?;
    Ok(())
}

fn main() -> ExitCode {
    unbound_log::init("unbound-settings");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
