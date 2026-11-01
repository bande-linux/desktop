use std::ffi::OsString;
use std::os::fd::BorrowedFd;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use bande_app::{Cx, Info, Keys, Kind, Menus, Program, View, WindowId};
use bande_app_menu::Entry;
use bande_settings::{
    Command, Displays, Feed, L10N, PaneId, PointingDevices, Services, SettingsView,
    SoftwareCommand, UDEV_DATA, UpdateNotice, announce_updates, feed, sidebar_width,
};
use bande_settings_panes::panes as panes_list;
use bande_toolkit::{
    Appearance, FrameRegions, MenuPlacement, Rect, Scene, TextSize, TextSystem, Window, WindowEvent,
};
use bande_xdg::spawn_detached;

const APP_ID: &str = "org.bande.Settings";
const WINDOW_SIZE: (u32, u32) = (900, 720);
const FEED: usize = 0;
const PANE_ACTION: &str = "pane:";
const FILES_APP_ID: &str = "org.bande.Files";
const FILES: &str = "bande-files";

static INFO: Info = Info {
    id: APP_ID,
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || L10N.tr("title"),
    keys: Keys::Standard,
    settings: None,
};

struct Pane(SettingsView);

impl Pane {
    fn after_input(&mut self, window: &mut Window) {
        let ui = self.0.ui();
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

impl View for Pane {
    fn scene(&mut self, window: &mut Window) -> Scene {
        window.set_frame_regions(frame_regions(window.appearance().text_size));
        self.0.set_full_screen(window.header_height() == 0);
        let (width, height) = window.size();
        self.0.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        match event {
            WindowEvent::Input(input) => {
                let handled = self.0.handle(&input);
                self.after_input(window);
                handled
            }
            WindowEvent::MenuAction(_) | WindowEvent::Pasted(_) | WindowEvent::MenuClosed => {
                let handled = View::event(self.0.ui(), window, event);
                self.0.settle();
                handled
            }
            WindowEvent::Accessibility(request) => self.0.perform(&request),
            WindowEvent::ToplevelChanged(state) => self.0.set_on_screen(!state.suspended),
            _ => false,
        }
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.0.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.0.set_window_active(active);
    }

    fn deadline(&self) -> Option<Instant> {
        self.0.deadline()
    }

    fn tick(&mut self, _window: &mut Window, now: Instant) -> bool {
        self.0.tick(now);
        true
    }

    fn animating(&self) -> bool {
        self.0.animating()
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.0.text_input_area()
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.0.accessibility())
    }
}

struct Settings {
    main: Option<WindowId>,
    parked: Option<Pane>,
    feed: Feed,
    displays: Option<Displays>,
    services: Services,
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

fn select_pane(view: &mut SettingsView, keyword: &str) {
    match PaneId::from_keyword(keyword) {
        Some(pane) => view.select_pane(pane),
        None => tracing::warn!("no settings pane {keyword:?}"),
    }
}

fn settings_view<'a>(
    cx: &'a mut Cx<Settings>,
    main: Option<WindowId>,
    parked: &'a mut Option<Pane>,
) -> Option<&'a mut SettingsView> {
    match main.and_then(|main| cx.window(main)) {
        Some((_, Pane(view))) => Some(view),
        None => parked.as_mut().map(|Pane(view)| view),
    }
}

fn open_main(
    cx: &mut Cx<Settings>,
    parked: Option<Pane>,
    arguments: &[String],
    displays: Option<&Displays>,
) -> Result<WindowId, bande_toolkit::WindowError> {
    cx.open(Kind::Main, &L10N.tr("title"), WINDOW_SIZE, |appearance| {
        if let Some(mut pane) = parked {
            pane.0.set_appearance(appearance);
            pane.0.set_on_screen(true);
            return pane;
        }
        let mut view = SettingsView::new(appearance.palette(), TextSystem::new());
        view.set_appearance(appearance);
        view.set_pointing_devices(PointingDevices::scan(Path::new(UDEV_DATA)));
        if bande_xdg::find_executable(bande_config::SCREEN_READER).is_none() {
            view.hide_screen_reader();
        }
        if let Some(keyword) = arguments.first() {
            select_pane(&mut view, keyword);
        }
        if let Some(displays) = displays {
            view.set_displays(displays.list());
        }
        Pane(view)
    })
}

impl Settings {
    fn send_commands(&mut self, cx: &mut Cx<Self>) {
        let Some((window, Pane(view))) = self.main.and_then(|main| cx.window(main)) else {
            return;
        };
        for command in view.take_commands() {
            match command {
                Command::Reveal(path) => reveal(window, &path),
                command => self.services.send(command),
            }
        }
        window.request_redraw();
    }
}

impl Program for Settings {
    type View = Pane;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let session = zbus::blocking::Connection::session()?;
        let system = zbus::blocking::Connection::system()
            .inspect_err(|error| tracing::warn!("no system bus: {error}"))
            .ok();
        let (feed, updates) = feed()?;
        let services = Services::start(&session, system.as_ref(), &updates);
        services.send(Command::Software(SoftwareCommand::Open));
        let displays = Displays::connect();
        let main = open_main(cx, None, &arguments, displays.as_ref())?;
        Ok(Self {
            main: Some(main),
            parked: None,
            feed,
            displays,
            services,
        })
    }

    fn menus(&self, _cx: &Cx<Self>) -> Menus {
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
        Menus {
            menus: vec![Entry::submenu(&L10N.tr("menu-view"), panes)],
            window: Vec::new(),
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, _window: Option<WindowId>, command: &str) {
        if let Some(keyword) = command.strip_prefix(PANE_ACTION) {
            if let Some(view) = settings_view(cx, self.main, &mut self.parked) {
                select_pane(view, keyword);
            }
            if self.main.is_none() {
                self.reopen(cx);
            }
        }
        self.send_commands(cx);
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, _directory: &Path) {
        if let (Some(keyword), Some(view)) = (
            arguments.first(),
            settings_view(cx, self.main, &mut self.parked),
        ) {
            select_pane(view, keyword);
        }
        if self.main.is_none() {
            self.reopen(cx);
        }
        self.send_commands(cx);
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        if self.main.is_some() {
            return;
        }
        match open_main(cx, self.parked.take(), &[], self.displays.as_ref()) {
            Ok(main) => self.main = Some(main),
            Err(error) => tracing::warn!("cannot open the settings: {error}"),
        }
        self.send_commands(cx);
    }

    fn window_closed(&mut self, _cx: &mut Cx<Self>, window: WindowId, mut view: Pane) {
        if self.main == Some(window) {
            self.main = None;
            view.0.set_on_screen(false);
            for command in view.0.take_commands() {
                if !matches!(command, Command::Reveal(_)) {
                    self.services.send(command);
                }
            }
            self.parked = Some(view);
        }
    }

    fn changed(&mut self, cx: &mut Cx<Self>, _window: WindowId) {
        self.send_commands(cx);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        let mut sources = vec![self.feed.fd()];
        sources.extend(self.displays.as_ref().map(Displays::fd));
        sources
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, index: usize) {
        let updates = match index {
            FEED => self.feed.drain(),
            _ => Vec::new(),
        };
        let displays = match index {
            FEED => None,
            _ => self.displays.as_mut().and_then(Displays::dispatch),
        };
        if let Some(view) = settings_view(cx, self.main, &mut self.parked) {
            for update in updates {
                view.apply(update);
            }
            if let Some(displays) = displays {
                view.set_displays(displays);
            }
        }
        self.send_commands(cx);
    }
}

const CHECK_UPDATES: &str = "--check-updates";
const ANNOUNCE_UPDATES: &str = "--announce-updates";

fn check_updates() -> Result<(), Box<dyn std::error::Error>> {
    let program = std::env::current_exe()?.to_string_lossy().into_owned();
    let announcer = [
        "systemd-run",
        "--user",
        "--scope",
        "--quiet",
        "--collect",
        "--",
        &program,
        ANNOUNCE_UPDATES,
    ]
    .map(str::to_owned);
    spawn_detached("", &announcer, None, None)?;
    Ok(())
}

fn announce_and_open_on_click() -> Result<(), Box<dyn std::error::Error>> {
    let session = zbus::blocking::Connection::session()?;
    let system = zbus::blocking::Connection::system()?;
    let notice = UpdateNotice::listen(&session)?;
    let Some(id) = announce_updates(&session, &system)? else {
        return Ok(());
    };
    if notice.clicked(id)? {
        let program = std::env::current_exe()?.to_string_lossy().into_owned();
        let pane = PaneId::SoftwareUpdate.keyword().to_owned();
        spawn_detached(APP_ID, &[program, pane], None, None)?;
    }
    Ok(())
}

fn finish(outcome: Result<(), Box<dyn std::error::Error>>) -> ExitCode {
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    match arguments.as_slice() {
        [flag] if flag == CHECK_UPDATES => {
            bande_log::init(INFO.package);
            finish(check_updates())
        }
        [flag] if flag == ANNOUNCE_UPDATES => {
            bande_log::init(INFO.package);
            finish(announce_and_open_on_click())
        }
        _ => bande_app::run::<Settings>(&INFO),
    }
}
