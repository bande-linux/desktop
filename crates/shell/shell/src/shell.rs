use std::cell::RefCell;
use std::error::Error;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use accesskit::TreeUpdate;
use unbound_bar::dialogs::DIALOGS;
use unbound_bar::low_memory::{LOW_MEMORY_SIZE, LowMemory};
use unbound_bar::pairing::PairingDialogs;
use unbound_bar::surfaces::{self, Display, Restart, Services};
use unbound_bluetooth::register_agent;
use unbound_config::{Config, config_path};
use unbound_controlcenter::ControlCenter;
use unbound_desktop_surface::DesktopSurface;
use unbound_launcher::LauncherShell;
use unbound_notifications::Notifications;
use unbound_status::captive::{follow_captive_portal, open_page};
use unbound_status::memory::{
    CGROUP_ROOT, OWN_CGROUP, PressureTrigger, SYSTEM_PRESSURE, Streak, app_slice,
};
use unbound_toolkit::{App, Rect, Role, Scene, Surfaces, TextSystem, Window, WindowEvent};
use unbound_xdg::{
    Locale, applications, current_desktops, data_dirs, find_app, follow_oom_kills, launch,
    reopen_file, take_apps_to_reopen,
};
use zbus::blocking::Connection;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Place {
    Notifications,
    ControlCenter,
    Launcher,
    Desktop(Display),
    Bar(Display),
    LowMemory,
}

type Opened = Vec<(Window, Box<dyn App>)>;

struct Context {
    text: TextSystem,
    session: Option<Connection>,
    system: Option<Connection>,
    config_home: Option<PathBuf>,
    home: Option<PathBuf>,
    notification_center: Rc<RefCell<Box<dyn Fn()>>>,
    app_slice: Option<PathBuf>,
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
            system: self.system.as_ref(),
            config_home: self.config_home.as_deref(),
            home: self.home.as_deref(),
            notification_center: Rc::new(move || (toggle.borrow())()),
        }
    }

    fn notifications(&self, window: &mut Window) -> std::io::Result<Box<dyn App>> {
        window.share_fonts(&self.text);
        let part = Notifications::new(window, self.text.sharing_fonts(), self.session.as_ref())?;
        *self.notification_center.borrow_mut() = Box::new(part.toggle());
        Ok(Box::new(part))
    }

    fn open(
        &self,
        beside: &Window,
        place: &Place,
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
                    &self.config(),
                    self.text.sharing_fonts(),
                )?;
                Ok((vec![(window, Box::new(part))], None))
            }
            Place::Bar(display) => {
                let bar = surfaces::open(beside, display, &self.config(), &self.services())?;
                Ok((bar.surfaces, Some(bar.restart)))
            }
            Place::LowMemory => {
                let title = unbound_bar::L10N.tr("low-memory-window");
                let mut window =
                    beside.open_beside(Role::toplevel(&title, DIALOGS), LOW_MEMORY_SIZE)?;
                window.share_fonts(&self.text);
                if let Err(error) = window.set_keep_on_top(true) {
                    tracing::warn!("the low memory window may hide behind others: {error}");
                }
                let part = LowMemory::new(
                    &window,
                    self.text.sharing_fonts(),
                    self.session.clone(),
                    self.app_slice.clone(),
                );
                Ok((vec![(window, Box::new(part))], None))
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
    pressure: Option<PressureTrigger>,
    streak: Streak,
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
        if self.streak.alerting() {
            wanted.push(Place::LowMemory);
        }
        wanted
    }

    fn memory_stalled(&mut self) {
        if self.streak.stalled(Instant::now()) {
            tracing::warn!("memory is running low");
            self.reconcile = true;
        }
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

    fn open(&mut self, place: Place) -> Result<(), Box<dyn Error>> {
        let Some(beside) = self.windows.first() else {
            return Ok(());
        };
        let (opened, restart) = self.context.open(beside, &place)?;
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
        for place in wanted {
            if self.parts.iter().any(|(known, _)| *known == place) {
                continue;
            }
            if let Err(error) = self.open(place.clone()) {
                tracing::warn!("cannot open {place:?}: {error}");
            }
        }
    }

    fn rearrange_bar(&mut self, display: Display) {
        if let Err(error) = self.open(Place::Bar(display.clone())) {
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
            .chain(self.pressure.as_ref().map(PressureTrigger::fd))
            .collect()
    }

    fn source_ready(&mut self, index: usize) {
        match self.part_of_source(index) {
            Some((part, local)) => self.parts[part]
                .1
                .source_ready(&mut self.windows[part], local),
            None => self.memory_stalled(),
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
        if closed.contains(&Place::LowMemory) {
            self.streak.dismissed();
        }
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

fn reopen_apps(wanted: bool) {
    let Some(file) = reopen_file() else {
        return;
    };
    let apps = take_apps_to_reopen(&file);
    if !wanted || apps.is_empty() {
        return;
    }
    let entries = applications(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    );
    for app in apps {
        if let Some(entry) = find_app(&entries, &app)
            && let Err(error) = launch(entry, None)
        {
            tracing::warn!("cannot reopen {app}: {error}");
        }
    }
}

fn pressure_trigger() -> Option<PressureTrigger> {
    PressureTrigger::open(Path::new(SYSTEM_PRESSURE))
        .inspect_err(|error| tracing::warn!("no warning when memory runs low: {error}"))
        .ok()
}

fn own_app_slice() -> Option<PathBuf> {
    let cgroup = std::fs::read_to_string(OWN_CGROUP).ok()?;
    app_slice(Path::new(CGROUP_ROOT), &cgroup)
}

pub fn run() -> Result<(), Box<dyn Error>> {
    let context = Context {
        text: TextSystem::new(),
        session: bus(Connection::session(), "session"),
        system: bus(Connection::system(), "system"),
        config_home: std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        home: std::env::var_os("HOME").map(PathBuf::from),
        notification_center: Rc::new(RefCell::new(Box::new(|| {}))),
        app_slice: own_app_slice(),
    };
    reopen_apps(context.config().reopen_apps);
    if let Some(session) = &context.session {
        follow_oom_kills(session.clone());
    }
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
        pressure: pressure_trigger(),
        streak: Streak::default(),
    };
    shell.reconcile();
    unbound_toolkit::run_surfaces(&mut shell)?;
    Ok(())
}
