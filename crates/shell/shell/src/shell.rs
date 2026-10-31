use std::cell::RefCell;
use std::error::Error;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use accesskit::TreeUpdate;
use unbound_bar::pairing::PairingDialogs;
use unbound_bar::surfaces::{self, Display, Restart, Services, status_hub};
use unbound_bluetooth::register_agent;
use unbound_config::{Config, config_path};
use unbound_controlcenter::ControlCenter;
use unbound_desktop_surface::DesktopSurface;
use unbound_launcher::LauncherShell;
use unbound_notifications::Notifications;
use unbound_status::captive::{follow_captive_portal, open_page};
use unbound_status::hub::StatusHub;
use unbound_toolkit::{App, Rect, Scene, Surfaces, TextSystem, Window, WindowEvent};
use unbound_xdg::{AppCatalog, find_app, launch, reopen_file, take_apps_to_reopen};
use zbus::blocking::Connection;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Place {
    Notifications,
    ControlCenter,
    Launcher,
    Desktop(Display),
    Bar(Display),
}

type Opened = Vec<(Window, Box<dyn App>)>;

struct Context {
    text: TextSystem,
    session: Option<Connection>,
    system: Option<Connection>,
    status: StatusHub,
    catalog: AppCatalog,
    config_home: Option<PathBuf>,
    home: Option<PathBuf>,
    notification_center: Rc<RefCell<Box<dyn Fn()>>>,
}

impl Context {
    fn config(&self) -> Config {
        config_path(self.config_home.as_deref(), self.home.as_deref())
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|source| Config::load(&source).ok())
            .map(|(config, _)| config)
            .unwrap_or_default()
    }

    fn services(&self) -> Services<'_> {
        let toggle = self.notification_center.clone();
        Services {
            text: &self.text,
            session: self.session.as_ref(),
            status: &self.status,
            catalog: &self.catalog,
            config_home: self.config_home.as_deref(),
            home: self.home.as_deref(),
            notification_center: Rc::new(move || (toggle.borrow())()),
        }
    }

    fn notifications(&self, window: &mut Window) -> std::io::Result<Box<dyn App>> {
        window.share_fonts(&self.text);
        let part = Notifications::new(
            window,
            self.text.sharing_fonts(),
            self.session.as_ref(),
            &self.catalog,
        )?;
        *self.notification_center.borrow_mut() = Box::new(part.toggle());
        Ok(Box::new(part))
    }

    fn open(
        &self,
        beside: &Window,
        place: &Place,
        config: &Config,
    ) -> Result<(Opened, Option<Restart>), Box<dyn Error>> {
        match place {
            Place::Notifications => {
                let mut window = beside.open_beside(unbound_notifications::role(), (0, 0))?;
                let part = self.notifications(&mut window)?;
                Ok((vec![(window, part)], None))
            }
            Place::ControlCenter => {
                let mut window = beside.open_beside(unbound_controlcenter::role(), (0, 0))?;
                window.share_fonts(&self.text);
                let part = ControlCenter::new(
                    &mut window,
                    self.text.sharing_fonts(),
                    self.session.as_ref(),
                    self.system.as_ref(),
                    self.status.audio(),
                    &self.catalog,
                )?;
                Ok((vec![(window, Box::new(part))], None))
            }
            Place::Launcher => {
                let mut window = beside.open_beside(unbound_launcher::role(), (0, 0))?;
                window.share_fonts(&self.text);
                let part = LauncherShell::new(
                    &mut window,
                    self.text.sharing_fonts(),
                    self.session.as_ref(),
                    &self.catalog,
                )?;
                Ok((vec![(window, Box::new(part))], None))
            }
            Place::Desktop(display) => {
                let mut window = beside.open_beside_on(
                    &display.output,
                    unbound_desktop_surface::role(),
                    (0, 0),
                )?;
                window.share_fonts(&self.text);
                let part = DesktopSurface::new(
                    &window,
                    display.main,
                    config,
                    self.text.sharing_fonts(),
                    (self.session.as_ref(), &self.catalog),
                )?;
                Ok((vec![(window, Box::new(part))], None))
            }
            Place::Bar(display) => {
                let bar = surfaces::open(beside, display, config, &self.services())?;
                Ok((bar.surfaces, Some(bar.restart)))
            }
        }
    }
}

struct Shell {
    context: Context,
    windows: Vec<Window>,
    parts: Vec<(Place, Box<dyn App>)>,
    restarts: Vec<(Display, Restart)>,
    reconcile: bool,
}

impl Shell {
    fn part_of_source(&self, mut index: usize) -> Option<(usize, usize)> {
        for (part, (_, app)) in self.parts.iter().enumerate() {
            let count = app.sources().len();
            if index < count {
                return Some((part, index));
            }
            index -= count;
        }
        None
    }

    fn wanted(&self) -> Vec<Place> {
        let displays = self
            .windows
            .first()
            .map(|window| window.displays().to_vec())
            .unwrap_or_default();
        let mut wanted = vec![Place::Notifications];
        for (index, output) in displays.into_iter().enumerate() {
            let display = Display {
                output,
                main: index == 0,
            };
            wanted.push(Place::Desktop(display.clone()));
            wanted.push(Place::Bar(display));
        }
        wanted.extend([Place::ControlCenter, Place::Launcher]);
        wanted
    }

    fn remove(&mut self, unwanted: impl Fn(&Place) -> bool) {
        let mut index = 0;
        while index < self.parts.len() {
            if unwanted(&self.parts[index].0) {
                self.parts.remove(index);
                self.windows.remove(index);
            } else {
                index += 1;
            }
        }
        let parts = &self.parts;
        self.restarts.retain(|(display, _)| {
            let place = Place::Bar(display.clone());
            parts.iter().any(|(known, _)| *known == place)
        });
    }

    fn open(&mut self, place: Place, config: &Config) -> Result<(), Box<dyn Error>> {
        let Some(beside) = self.windows.first() else {
            return Ok(());
        };
        let (opened, restart) = self.context.open(beside, &place, config)?;
        self.remove(|known| *known == place);
        if let (Place::Bar(display), Some(restart)) = (&place, restart) {
            self.restarts.push((display.clone(), restart));
        }
        for (window, app) in opened {
            self.windows.push(window);
            self.parts.push((place.clone(), app));
        }
        Ok(())
    }

    fn reconcile(&mut self) {
        self.reconcile = false;
        let wanted = self.wanted();
        self.remove(|place| !wanted.contains(place));
        let mut config = None;
        for place in wanted {
            if self.parts.iter().any(|(known, _)| *known == place) {
                continue;
            }
            let config = config.get_or_insert_with(|| self.context.config());
            if let Err(error) = self.open(place.clone(), config) {
                tracing::warn!("cannot open {place:?}: {error}");
            }
        }
    }

    fn rearrange_bar(&mut self, display: Display) {
        let config = self.context.config();
        if let Err(error) = self.open(Place::Bar(display.clone()), &config) {
            tracing::warn!("cannot rearrange the bar: {error}");
            for (known, restart) in &self.restarts {
                if *known == display {
                    restart.set(false);
                }
            }
        }
    }
}

impl Surfaces for Shell {
    fn windows(&self) -> &[Window] {
        &self.windows
    }

    fn windows_mut(&mut self) -> &mut [Window] {
        &mut self.windows
    }

    fn scene(&mut self, index: usize) -> Scene {
        self.parts[index].1.scene(&mut self.windows[index])
    }

    fn event(&mut self, index: usize, event: WindowEvent) {
        if matches!(event, WindowEvent::Displays(_)) {
            self.reconcile = true;
        }
        self.parts[index].1.event(&mut self.windows[index], event);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.parts
            .iter()
            .flat_map(|(_, app)| app.sources())
            .collect()
    }

    fn source_ready(&mut self, index: usize) {
        if let Some((part, local)) = self.part_of_source(index) {
            self.parts[part]
                .1
                .source_ready(&mut self.windows[part], local);
        }
    }

    fn deadline(&self) -> Option<Instant> {
        let now = self.reconcile.then(Instant::now);
        self.parts
            .iter()
            .filter_map(|(_, app)| app.deadline())
            .chain(now)
            .min()
    }

    fn tick(&mut self, now: Instant) {
        for ((_, app), window) in self.parts.iter_mut().zip(&mut self.windows) {
            if app.deadline().is_some_and(|deadline| deadline <= now) {
                app.tick(window, now);
            }
        }
        let rearranged: Vec<Display> = self
            .restarts
            .iter()
            .filter(|(_, restart)| restart.get())
            .map(|(display, _)| display.clone())
            .collect();
        for display in rearranged {
            self.rearrange_bar(display);
        }
        if self.reconcile {
            self.reconcile();
        }
    }

    fn animating(&self, index: usize) -> bool {
        self.parts[index].1.animating()
    }

    fn accessibility(&self, index: usize) -> Option<TreeUpdate> {
        self.parts[index].1.accessibility()
    }

    fn text_input_area(&self, index: usize) -> Option<Rect> {
        self.parts[index].1.text_input_area()
    }

    fn closed(&mut self) -> bool {
        let closed: Vec<Place> = self
            .windows
            .iter()
            .zip(&self.parts)
            .filter(|(window, _)| window.is_closed())
            .map(|(_, (place, _))| place.clone())
            .collect();
        if !closed.is_empty() {
            self.remove(|place| closed.contains(place));
            self.reconcile = true;
        }
        false
    }
}

fn bus(connection: zbus::Result<Connection>, name: &str) -> Option<Connection> {
    connection
        .inspect_err(|error| tracing::warn!("no {name} bus: {error}"))
        .ok()
}

fn reopen_apps(wanted: bool, catalog: &AppCatalog) {
    let Some(file) = reopen_file() else {
        return;
    };
    let apps = take_apps_to_reopen(&file);
    if !wanted || apps.is_empty() {
        return;
    }
    for app in apps {
        if let Some(entry) = find_app(&catalog.entries(), &app)
            && let Err(error) = launch(entry, None)
        {
            tracing::warn!("cannot reopen {app}: {error}");
        }
    }
}

pub fn run() -> Result<(), Box<dyn Error>> {
    let session = bus(Connection::session(), "session");
    let system = bus(Connection::system(), "system");
    let status = status_hub(session.as_ref(), system.as_ref())?;
    let context = Context {
        text: TextSystem::new(),
        session,
        system,
        status,
        catalog: AppCatalog::load(),
        config_home: std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        home: std::env::var_os("HOME").map(PathBuf::from),
        notification_center: Rc::new(RefCell::new(Box::new(|| {}))),
    };
    reopen_apps(context.config().reopen_apps, &context.catalog);
    if let Some(system) = &context.system {
        if let Err(error) = register_agent(system, Arc::new(PairingDialogs::default())) {
            tracing::warn!("Bluetooth devices can pair only from Settings: {error}");
        }
        if let Some(session) = &context.session {
            follow_captive_portal(system, session, open_page);
        }
    }
    let mut root = Window::open(unbound_notifications::role(), (0, 0))?;
    let notifications = context.notifications(&mut root)?;
    let mut shell = Shell {
        context,
        windows: vec![root],
        parts: vec![(Place::Notifications, notifications)],
        restarts: Vec::new(),
        reconcile: false,
    };
    shell.reconcile();
    unbound_toolkit::run_surfaces(&mut shell)?;
    Ok(())
}
