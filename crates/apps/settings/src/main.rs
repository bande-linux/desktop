use std::os::fd::BorrowedFd;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use bande_app_menu::{AppMenu, Entry, Standard, app_submenu, window_submenu};
use bande_settings::{
    Command, Displays, Feed, L10N, PaneId, PointingDevices, Services, SettingsView,
    SoftwareCommand, UDEV_DATA, announce_updates, clicked, feed, sidebar_width,
};
use bande_settings_panes::panes as panes_list;
use bande_toolkit::ui::shortcut_pressed;
use bande_toolkit::{
    App, FrameRegions, MenuPlacement, Rect, Role, Scene, TextSize, TextSystem, Window, WindowEvent,
};
use bande_xdg::spawn_detached;

const APP_ID: &str = "org.bande.Settings";
const WINDOW_SIZE: (u32, u32) = (900, 720);
const FEED: usize = 0;
const PANE_ACTION: &str = "pane:";
const FILES_APP_ID: &str = "org.bande.Files";
const FILES: &str = "bande-files";

struct Settings {
    view: SettingsView,
    feed: Feed,
    displays: Option<Displays>,
    services: Services,
    app_menu: Option<AppMenu>,
}

impl Settings {
    fn after_input(&mut self, window: &mut Window) {
        let ui = self.view.ui();
        let menu = ui.take_menu_request();
        let clipboard = ui.take_clipboard_request();
        window.set_cursor(ui.cursor());
        if let Some((menu, at)) = menu
            && let Err(error) = window.show_menu(menu, MenuPlacement::AtPoint(at))
        {
            tracing::warn!("cannot show the menu: {error}");
        }
        if let Some(request) = clipboard
            && let Err(error) = window.serve_clipboard(request)
        {
            tracing::warn!("cannot use the clipboard: {error}");
        }
    }
}

fn reveal(window: &mut Window, path: &Path) {
    let token = window.activation_token(FILES_APP_ID).ok();
    let command = [FILES.to_owned(), path.to_string_lossy().into_owned()];
    if let Err(error) = spawn_detached(FILES_APP_ID, &command, None, token.as_deref()) {
        tracing::warn!("cannot show {} in Files: {error}", path.display());
    }
}

fn frame_regions(text: TextSize) -> FrameRegions {
    FrameRegions {
        sidebar: sidebar_width(text),
        toolbar: 0.0,
    }
}

fn app_menus() -> Vec<Entry> {
    let mut panes = Vec::new();
    let mut group = None;
    for pane in panes_list() {
        let Some(own) = pane.id.group() else {
            continue;
        };
        if group.is_some_and(|group| group != own) {
            panes.push(Entry::separator());
        }
        group = Some(own);
        panes.push(Entry::item(
            &pane.name,
            &format!("{PANE_ACTION}{}", pane.id.keyword()),
        ));
    }
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

    fn send_commands(&mut self, window: &mut Window) {
        for command in self.view.take_commands() {
            match command {
                Command::Reveal(path) => reveal(window, &path),
                command => self.services.send(command),
            }
        }
    }
}

impl App for Settings {
    fn scene(&mut self, window: &mut Window) -> Scene {
        self.view.set_full_screen(window.header_height() == 0);
        let (width, height) = window.size();
        self.view.scene(width, height)
    }

    fn deadline(&self) -> Option<Instant> {
        self.view.deadline()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        self.view.tick(now);
        self.send_commands(window);
        window.request_redraw();
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::Input(input) => {
                match Standard::for_shortcut(|shortcut| shortcut_pressed(&input, shortcut)) {
                    Some(standard) => self.menu_action(window, standard.action()),
                    None => {
                        self.view.handle(&input);
                    }
                }
                self.after_input(window);
            }
            WindowEvent::MenuAction(_) | WindowEvent::Pasted(_) | WindowEvent::MenuClosed => {
                if let Err(error) = window.drive(self.view.ui(), event) {
                    tracing::warn!("{error}");
                }
                self.view.settle();
            }
            WindowEvent::Accessibility(request) => {
                self.view.perform(&request);
            }
            WindowEvent::AppearanceChanged(appearance) => {
                self.view.set_appearance(appearance);
                window.set_frame_regions(frame_regions(appearance.text_size));
            }
            WindowEvent::Focus(focused) => {
                self.view.set_window_active(focused);
                if focused && let Some(Err(error)) = self.app_menu.as_ref().map(AppMenu::claim) {
                    tracing::warn!("{error}");
                }
            }
            _ => return,
        }
        self.send_commands(window);
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
        self.send_commands(window);
        window.request_redraw();
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.view.text_input_area()
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.view.accessibility())
    }
}

const CHECK_UPDATES: &str = "--check-updates";
const OPEN_ON_CLICK: &str = "--open-on-click";

fn check_updates() -> Result<(), Box<dyn std::error::Error>> {
    let session = zbus::blocking::Connection::session()?;
    let system = zbus::blocking::Connection::system()?;
    if let Some(id) = announce_updates(&session, &system)? {
        let program = std::env::current_exe()?.to_string_lossy().into_owned();
        let waiter = [
            "systemd-run",
            "--user",
            "--scope",
            "--quiet",
            "--collect",
            "--",
            &program,
            OPEN_ON_CLICK,
            &id.to_string(),
        ]
        .map(str::to_owned);
        spawn_detached("", &waiter, None, None)?;
    }
    Ok(())
}

fn open_on_click(id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let session = zbus::blocking::Connection::session()?;
    if clicked(&session, id.parse()?)? {
        let program = std::env::current_exe()?.to_string_lossy().into_owned();
        let pane = PaneId::SoftwareUpdate.keyword().to_owned();
        spawn_detached(APP_ID, &[program, pane], None, None)?;
    }
    Ok(())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let session = zbus::blocking::Connection::session()?;
    let system = zbus::blocking::Connection::system()
        .inspect_err(|error| tracing::warn!("no system bus: {error}"))
        .ok();
    let (feed, updates) = feed()?;
    let services = Services::start(&session, system.as_ref(), &updates);
    services.send(Command::Software(SoftwareCommand::Open));
    let mut window = Window::open(Role::toplevel(&L10N.tr("title"), APP_ID), WINDOW_SIZE)?;
    window.set_frame_regions(frame_regions(window.appearance().text_size));
    let mut view = SettingsView::new(window.appearance().palette(), TextSystem::new());
    view.set_pointing_devices(PointingDevices::scan(Path::new(UDEV_DATA)));
    if bande_xdg::find_executable(bande_config::SCREEN_READER).is_none() {
        view.hide_screen_reader();
    }
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
    bande_toolkit::run(&mut window, &mut settings)?;
    Ok(())
}

fn main() -> ExitCode {
    bande_log::init("bande-settings");
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match arguments.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [CHECK_UPDATES] => check_updates(),
        [OPEN_ON_CLICK, id] => open_on_click(id),
        _ => run(),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
