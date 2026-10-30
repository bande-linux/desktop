use std::os::fd::BorrowedFd;
use std::process::ExitCode;
use std::time::Instant;

use unbound_app_menu::{AppMenu, Entry, Standard, app_submenu, window_submenu};
use unbound_settings::{Displays, Feed, L10N, PaneId, Services, SettingsView, feed};
use unbound_settings_panes::panes;
use unbound_toolkit::{App, Rect, Role, Scene, TextSystem, Window, WindowEvent};

const APP_ID: &str = "org.unbound.Settings";
const WINDOW_SIZE: (u32, u32) = (900, 600);
const FEED: usize = 0;
const PANE_ACTION: &str = "pane:";

struct Settings {
    view: SettingsView,
    feed: Feed,
    displays: Option<Displays>,
    services: Services,
    app_menu: Option<AppMenu>,
}

fn app_menus() -> Vec<Entry> {
    let panes = panes()
        .into_iter()
        .map(|pane| Entry::item(&pane.name, &format!("{PANE_ACTION}{}", pane.id.keyword())))
        .collect();
    vec![
        app_submenu(&L10N.tr("title"), Vec::new()),
        Entry::submenu(&L10N.tr("menu-view"), panes),
        window_submenu(Vec::new()),
    ]
}

impl Settings {
    fn menu_action(&mut self, window: &mut Window, action: &str) {
        let result = match Standard::parse(action) {
            Some(Standard::Hide | Standard::Minimize) => window.minimize(),
            Some(Standard::Zoom) => window.toggle_zoom(),
            Some(Standard::Quit | Standard::CloseWindow) => {
                window.close();
                Ok(())
            }
            None => {
                if let Some(pane) = action
                    .strip_prefix(PANE_ACTION)
                    .and_then(PaneId::from_keyword)
                {
                    self.view.select_pane(pane);
                }
                Ok(())
            }
        };
        if let Err(error) = result {
            tracing::warn!("cannot {action}: {error}");
        }
    }

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

    fn deadline(&self) -> Option<Instant> {
        self.view.deadline()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        self.view.tick(now);
        self.send_commands();
        window.request_redraw();
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
            WindowEvent::Focus(true) => {
                if let Some(Err(error)) = self.app_menu.as_ref().map(AppMenu::claim) {
                    tracing::warn!("{error}");
                }
                return;
            }
            _ => return,
        }
        self.send_commands();
        window.request_redraw();
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        let mut sources = vec![self.feed.fd()];
        sources.extend(self.displays.as_ref().map(Displays::fd));
        sources.extend(self.app_menu.as_ref().map(AppMenu::fd));
        sources
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        let menu = 1 + usize::from(self.displays.is_some());
        if index == menu {
            let actions = self
                .app_menu
                .as_mut()
                .map(AppMenu::take_actions)
                .unwrap_or_default();
            for action in actions {
                self.menu_action(window, &action);
            }
        } else if index == FEED {
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
    let app_menu = AppMenu::export_on(session, APP_ID, app_menus())
        .inspect_err(|error| tracing::warn!("{error}"))
        .ok();
    let mut settings = Settings {
        view,
        feed,
        displays,
        services,
        app_menu,
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
