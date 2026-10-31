use std::cell::Cell;
use std::os::fd::BorrowedFd;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use crate::media::{self, Command, Playback, Player, Players};
use crate::{
    Anchor, Change, Changes, Choice, HUD_DURATION, HUD_SIZE, Hud, KeyboardLight, L10N, PANEL_WIDTH,
    SYSFS_LEDS, Status, chooser_menu, hud, panel, panel_items, place_beside, scene, slide_from,
    stepped,
};
use accesskit::TreeUpdate;
use unbound_audio::{Audio, Level, Meter, meter_level};
use unbound_bluetooth::Bluetooth;
use unbound_inbox::Inbox;
use unbound_network::NetworkManager;
use unbound_notify::client::NotificationCenter;
use unbound_power::backlight::{Backlight, SYSFS_BACKLIGHT};
use unbound_power::logind::own_session;
use unbound_status::captive::open_page;
use unbound_status::session::full_name;
use unbound_toolkit::ui::{Animated, Event, Key, Menu, MenuItem, Pointer, Scroll, Spring, Ui};
use unbound_toolkit::{
    App, BrightnessKey, Edges, Image, Item, KeyboardFocus, LayerKind, LayerOptions, MediaKey,
    MenuPlacement, Palette, Panel, Point, Rect, Role, Scene, SessionEnd, TextSystem, VolumeKey,
    Window, WindowEvent,
};
use unbound_xdg::{
    DesktopEntry, ICON_THEMES, applications, current_desktops, data_dirs, find_icon,
    path_from_file_uri, remember_running_apps, running_app, spawn_detached,
};
use zbus::blocking::Connection;

const JOIN: &str = "join:";
const SESSION: &str = "session:";
const ARTWORK_PIXELS: u32 = 128;
const METER_INTERVAL: Duration = Duration::from_millis(50);
const SLIDE: f32 = 8.0;

enum Mode {
    Hidden,
    Hud(Hud, Instant),
    Panel { ui: Box<Ui>, height: f32 },
}

pub struct ControlCenter {
    mode: Mode,
    shown: Animated,
    surface_shown: bool,
    reduce_motion: bool,
    palette: Palette,
    text: TextSystem,
    changes: Changes,
    pointer: Point,
    anchor: Option<Anchor>,
    audio: Option<Audio>,
    network: Option<NetworkManager>,
    bluetooth: Option<Bluetooth>,
    system: Option<Connection>,
    session: Option<Connection>,
    backlight: Option<Backlight>,
    keyboard_light: Option<KeyboardLight>,
    players: Players,
    player_updates: Inbox<Vec<Player>>,
    player_commands: Option<Sender<(String, Command)>>,
    artwork: Option<(String, Option<Image>)>,
    meter: Option<Meter>,
    input_level: Rc<Cell<f32>>,
}

fn report<E: std::fmt::Display>(what: &str, result: Result<(), E>) {
    if let Err(error) = result {
        tracing::warn!("cannot {what}: {error}");
    }
}

fn audio_key(key: VolumeKey) -> Option<unbound_audio::VolumeKey> {
    match key {
        VolumeKey::Mute => Some(unbound_audio::VolumeKey::Mute),
        VolumeKey::Down => Some(unbound_audio::VolumeKey::Down),
        VolumeKey::Up => Some(unbound_audio::VolumeKey::Up),
        _ => None,
    }
}

fn media_command(key: MediaKey) -> Option<Command> {
    match key {
        MediaKey::PlayPause => Some(Command::PlayPause),
        MediaKey::Pause => Some(Command::Pause),
        MediaKey::Next => Some(Command::Next),
        MediaKey::Previous => Some(Command::Previous),
        _ => None,
    }
}

fn app_icon(desktop_entry: &str) -> Option<Image> {
    let directories = data_dirs();
    let file = format!("{desktop_entry}.desktop");
    let entry = directories.iter().find_map(|directory| {
        let path = directory.join("applications").join(&file);
        let text = std::fs::read_to_string(&path).ok()?;
        DesktopEntry::parse_handler(&file, &path, &text, None, &current_desktops())
    })?;
    let path = find_icon(
        entry.icon.as_deref()?,
        ARTWORK_PIXELS,
        &ICON_THEMES,
        &directories,
    )?;
    Image::load(&path, ARTWORK_PIXELS)
}

fn artwork(player: &Player) -> Option<Image> {
    player
        .art_url
        .as_deref()
        .and_then(path_from_file_uri)
        .and_then(|path| Image::load(&path, ARTWORK_PIXELS))
        .or_else(|| app_icon(player.desktop_entry.as_deref()?))
}

fn artwork_key(player: &Player) -> String {
    format!(
        "{}\n{}",
        player.art_url.as_deref().unwrap_or_default(),
        player.desktop_entry.as_deref().unwrap_or_default()
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
        .collect()
}

fn moved(event: &Event, offset: Point) -> Event {
    let shift = |position: Point| Point::new(position.x - offset.x, position.y - offset.y);
    let pointer = |pointer: &Pointer| Pointer {
        position: shift(pointer.position),
        ..*pointer
    };
    match event {
        Event::PointerDown(found) => Event::PointerDown(pointer(found)),
        Event::PointerMove(found) => Event::PointerMove(pointer(found)),
        Event::PointerUp(found) => Event::PointerUp(pointer(found)),
        Event::PointerEnter(found) => Event::PointerEnter(pointer(found)),
        Event::Scroll(scroll) => Event::Scroll(Scroll {
            position: shift(scroll.position),
            ..*scroll
        }),
        other => other.clone(),
    }
}

fn contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.x
        && point.y >= rect.y
        && point.x < rect.x + rect.width
        && point.y < rect.y + rect.height
}

impl ControlCenter {
    fn notification_center(&self) -> Option<NotificationCenter<'_>> {
        NotificationCenter::new(self.session.as_ref()?).ok()
    }

    fn brightness(&self) -> Option<f32> {
        let backlight = self.backlight.as_ref()?;
        let maximum = backlight
            .max_brightness()
            .ok()
            .filter(|maximum| *maximum > 0)?;
        Some(backlight.brightness().ok()? as f32 / maximum as f32)
    }

    fn status(&self) -> Status {
        let wifi = self.network.as_ref().and_then(|network| {
            network
                .has_wifi()
                .ok()
                .filter(|has| *has)
                .and_then(|_| network.wifi_enabled().ok())
        });
        let network = self
            .network
            .as_ref()
            .filter(|_| wifi == Some(true))
            .and_then(|network| network.networks().ok())
            .and_then(|networks| networks.into_iter().find(|found| found.current))
            .map(|current| current.name());
        let sign_in = self
            .network
            .as_ref()
            .is_some_and(|network| matches!(network.sign_in_page(), Ok(Some(_))));
        let bluetooth = self.bluetooth.as_ref().and_then(|bluetooth| {
            bluetooth
                .available()
                .ok()
                .filter(|available| *available)
                .and_then(|_| bluetooth.powered().ok())
        });
        let do_not_disturb = self
            .notification_center()
            .and_then(|center| center.do_not_disturb().ok())
            .unwrap_or(false);
        let snapshot = self.audio.as_ref().map(Audio::snapshot);
        let output = snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.default_output());
        let outputs = snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .outputs
                    .iter()
                    .map(|output| Choice {
                        key: output.name.clone(),
                        title: output.description.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let player = self.players.current().cloned();
        let input = snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.default_input());
        Status {
            wifi,
            network,
            sign_in,
            bluetooth,
            do_not_disturb,
            volume: output.map(|output| output.level.volume),
            outputs,
            output: output.map(|output| output.name.clone()),
            brightness: self.brightness(),
            keyboard_brightness: self.keyboard_light.as_ref().and_then(KeyboardLight::level),
            inputs: snapshot
                .as_ref()
                .map(|snapshot| {
                    snapshot
                        .inputs
                        .iter()
                        .map(|input| Choice {
                            key: input.name.clone(),
                            title: input.description.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            input: input.map(|input| input.name.clone()),
            input_muted: input.map(|input| input.level.muted),
            input_level: Rc::clone(&self.input_level),
            artwork: self
                .artwork
                .as_ref()
                .filter(|(key, _)| {
                    player
                        .as_ref()
                        .is_some_and(|player| artwork_key(player) == *key)
                })
                .and_then(|(_, image)| image.clone()),
            players: self
                .players
                .all()
                .map(|player| Choice {
                    key: player.bus_name.clone(),
                    title: player.identity.clone(),
                })
                .collect(),
            player,
            person: Some(full_name()),
            icon_dirs: data_dirs(),
        }
    }

    fn panel_rect(&self, window: &Window, height: f32) -> Rect {
        let (width, surface_height) = window.size();
        place_beside(
            self.anchor,
            (width as f32, surface_height as f32),
            (PANEL_WIDTH, height),
        )
    }

    fn show_menu(&self, window: &mut Window, choices: &[(String, String, bool)]) {
        if choices.is_empty() {
            return;
        }
        report(
            "open the menu",
            window.show_menu(chooser_menu(choices), MenuPlacement::AtPoint(self.pointer)),
        );
    }

    fn choose_network(&mut self, window: &mut Window) {
        let Some(network) = &self.network else {
            return;
        };
        if let Ok(Some(page)) = network.sign_in_page() {
            open_page(&page);
            self.dismiss(window);
            return;
        }
        let _ = network.scan();
        let choices: Vec<_> = network
            .networks()
            .unwrap_or_default()
            .into_iter()
            .map(|found| {
                (
                    found.name(),
                    format!("{JOIN}{}", hex(&found.ssid)),
                    found.current,
                )
            })
            .collect();
        self.show_menu(window, &choices);
    }

    fn join(&mut self, window: &mut Window, ssid: &[u8]) {
        let Some(network) = &self.network else {
            return;
        };
        let Some(found) = network
            .networks()
            .unwrap_or_default()
            .into_iter()
            .find(|candidate| candidate.ssid == ssid)
        else {
            return;
        };
        if found.security.needs_password() && !found.known {
            self.launch(
                window,
                "org.unbound.Settings",
                &["unbound-settings", "wifi"],
            );
            self.dismiss(window);
            return;
        }
        report("join the network", network.join(&found, None).map(|_| ()));
    }

    fn apply(&mut self, window: &mut Window, change: Change) {
        match change {
            Change::Wifi(on) => {
                if let Some(network) = &self.network {
                    report("switch Wi-Fi", network.set_wifi_enabled(on));
                }
                self.rebuild();
            }
            Change::Bluetooth(on) => {
                if let Some(bluetooth) = &self.bluetooth {
                    report("switch Bluetooth", bluetooth.set_powered(on));
                }
            }
            Change::DoNotDisturb(on) => {
                if let Some(center) = self.notification_center() {
                    report("switch Do Not Disturb", center.set_do_not_disturb(on));
                }
            }
            Change::Volume(volume) => {
                let Some(audio) = &self.audio else {
                    return;
                };
                if let Some(output) = audio.snapshot().default_output() {
                    let level = Level {
                        volume,
                        muted: false,
                    };
                    report("set the volume", audio.set_level(output, level));
                }
            }
            Change::Brightness(level) => {
                if let (Some(system), Some(backlight)) = (&self.system, &self.backlight)
                    && let Ok(maximum) = backlight.max_brightness()
                {
                    let value = ((level * maximum as f32).round() as u32).clamp(1, maximum);
                    report(
                        "set the brightness",
                        own_session(system).and_then(|session| {
                            session.set_brightness("backlight", &backlight.name, value)
                        }),
                    );
                }
            }
            Change::KeyboardBrightness(level) => self.set_keyboard_light(level),
            Change::InputMuted(muted) => {
                if let Some(audio) = &self.audio
                    && let Some(input) = audio.snapshot().default_input()
                {
                    report("mute the microphone", audio.set_muted(input, muted));
                }
            }
            Change::Input(name) => {
                if let Some(audio) = &self.audio
                    && let Some(device) = audio.snapshot().device(&name).cloned()
                {
                    report("choose the input", audio.set_default(&device));
                }
                self.listen();
            }
            Change::ChooseNetwork => self.choose_network(window),
            Change::Output(name) => {
                if let Some(audio) = &self.audio
                    && let Some(device) = audio.snapshot().device(&name).cloned()
                {
                    report("choose the output", audio.set_default(&device));
                }
            }
            Change::Media(command) => self.command_player(command),
            Change::RaisePlayer => self.raise_player(window),
            Change::Lock => {
                if let Some(system) = &self.system {
                    report(
                        "lock the screen",
                        own_session(system).and_then(|session| session.lock()),
                    );
                }
                self.dismiss(window);
            }
            Change::Power => {
                let user = [("user", full_name().into())];
                let items = vec![
                    MenuItem::new(&L10N.tr("sleep"), &format!("{SESSION}sleep")),
                    MenuItem::separator(),
                    MenuItem::new(&L10N.tr("restart"), &format!("{SESSION}restart")),
                    MenuItem::new(&L10N.tr("shut-down"), &format!("{SESSION}shut-down")),
                    MenuItem::separator(),
                    MenuItem::new(&L10N.tr_args("log-out", user), &format!("{SESSION}log-out")),
                ];
                report(
                    "open the power menu",
                    window.show_menu(Menu::new(items), MenuPlacement::AtPoint(self.pointer)),
                );
            }
            Change::Settings => {
                self.launch(window, "org.unbound.Settings", &["unbound-settings"]);
                self.dismiss(window);
            }
            Change::Player(bus_name) => {
                self.players.promote(&bus_name);
                self.load_artwork();
                self.rebuild();
            }
        }
    }

    fn command_player(&self, command: Command) {
        if let (Some(player), Some(commands)) = (self.players.current(), &self.player_commands) {
            let _ = commands.send((player.bus_name.clone(), command));
        }
    }

    fn launch(&self, window: &mut Window, app_id: &str, command: &[&str]) {
        let token = window.activation_token(app_id).ok();
        let command: Vec<String> = command.iter().map(|part| (*part).to_owned()).collect();
        if let Err(error) = spawn_detached(app_id, &command, None, token.as_deref()) {
            tracing::warn!("cannot start {}: {error}", command.join(" "));
        }
    }

    fn end_session(&mut self, window: &mut Window, keyword: &str) {
        if keyword == "sleep" {
            if let Some(system) = &self.system {
                report("sleep", unbound_status::devices::sleep(system));
            }
        } else {
            let ending = match keyword {
                "restart" => SessionEnd::Restart,
                "shut-down" => SessionEnd::ShutDown,
                _ => SessionEnd::LogOut,
            };
            remember_running_apps(&window.running_apps());
            if let Err(error) = window.confirm_session_end(ending) {
                tracing::warn!("cannot ask to end the session: {error}");
            }
        }
        self.dismiss(window);
    }

    fn raise_player(&mut self, window: &mut Window) {
        let Some(player) = self.players.current() else {
            return;
        };
        let entries = applications(&data_dirs(), None, &current_desktops());
        let windows = window.running_apps();
        let running = player
            .desktop_entry
            .as_deref()
            .and_then(|entry| running_app(&entries, &windows, entry))
            .map(str::to_owned);
        match running {
            Some(app_id) => report("bring the player forward", window.activate_app(&app_id)),
            None if player.can_raise => self.command_player(Command::Raise),
            None => return,
        }
        self.dismiss(window);
    }

    fn load_artwork(&mut self) {
        let Some(player) = self.players.current() else {
            self.artwork = None;
            return;
        };
        let key = artwork_key(player);
        if self.artwork.as_ref().is_none_or(|(known, _)| *known != key) {
            self.artwork = Some((key, artwork(player)));
        }
    }

    fn playing(&self) -> Option<&Player> {
        self.players
            .current()
            .filter(|player| player.playback == Playback::Playing && player.length.is_some())
    }

    fn fresh_panel(&self) -> (Ui, f32) {
        let (mut ui, height) = panel(
            &self.status(),
            self.palette,
            self.text.sharing_fonts(),
            &self.changes,
        );
        ui.context().reduce_motion = self.reduce_motion;
        (ui, height)
    }

    fn rebuild(&mut self) {
        if !matches!(self.mode, Mode::Panel { .. }) {
            return;
        }
        let (fresh, fresh_height) = self.fresh_panel();
        if let Mode::Panel { ui, height } = &mut self.mode {
            **ui = fresh;
            *height = fresh_height;
        }
    }

    fn reveal(&mut self, window: &mut Window, keyboard: bool) {
        report("show", window.show());
        self.surface_shown = true;
        if keyboard {
            window.set_input_region(None);
        } else {
            window.set_input_region(Some(&[]));
        }
        report(
            "change keyboard focus",
            window.set_keyboard_focus(if keyboard {
                KeyboardFocus::Exclusive
            } else {
                KeyboardFocus::None
            }),
        );
        self.shown
            .animate_at(Instant::now(), self.reduce_motion, 1.0, Spring::QUICK);
        window.request_redraw();
    }

    fn listen(&mut self) {
        self.input_level.set(0.0);
        self.meter = None;
        let Some(audio) = &self.audio else {
            return;
        };
        self.meter = audio.snapshot().default_input().and_then(|input| {
            audio
                .meter(input)
                .inspect_err(|error| tracing::warn!("no input level: {error}"))
                .ok()
        });
    }

    fn dismiss(&mut self, window: &mut Window) {
        self.meter = None;
        self.mode = Mode::Hidden;
        self.shown
            .animate_at(Instant::now(), self.reduce_motion, 0.0, Spring::QUICK);
        window.set_input_region(Some(&[]));
        report(
            "release the keyboard",
            window.set_keyboard_focus(KeyboardFocus::None),
        );
        window.request_redraw();
    }

    fn toggle_panel(&mut self, window: &mut Window) {
        if matches!(self.mode, Mode::Panel { .. }) {
            self.dismiss(window);
            return;
        }
        let (ui, height) = self.fresh_panel();
        self.mode = Mode::Panel {
            ui: Box::new(ui),
            height,
        };
        self.listen();
        self.shown.jump_to(0.0);
        self.reveal(window, true);
    }

    fn volume_key(&mut self, window: &mut Window, key: VolumeKey, fine: bool) {
        let (Some(audio), Some(key)) = (&self.audio, audio_key(key)) else {
            return;
        };
        match audio.press(key, fine) {
            Ok(level) => {
                if !level.muted
                    && let Some(session) = &self.session
                {
                    unbound_sound::play(session, "audio-volume-change");
                }
                self.show_hud(window, Hud::Volume(level));
            }
            Err(error) => tracing::warn!("cannot change the volume: {error}"),
        }
    }

    fn set_keyboard_light(&self, level: f32) {
        if let (Some(system), Some(light)) = (&self.system, &self.keyboard_light)
            && let Some(maximum) = light.maximum()
        {
            let value = ((level * maximum as f32).round() as u32).min(maximum);
            report(
                "set the keyboard brightness",
                own_session(system)
                    .and_then(|session| session.set_brightness("leds", &light.name, value)),
            );
        }
    }

    fn brightness_key(&mut self, window: &mut Window, key: BrightnessKey, fine: bool) {
        let up = matches!(key, BrightnessKey::DisplayUp | BrightnessKey::KeyboardUp);
        let hud = match key {
            BrightnessKey::DisplayDown | BrightnessKey::DisplayUp => {
                let Some(level) = self.brightness() else {
                    return;
                };
                let level = stepped(level, up, fine);
                self.apply(window, Change::Brightness(level));
                Hud::Display(level)
            }
            BrightnessKey::KeyboardDown | BrightnessKey::KeyboardUp => {
                let Some(level) = self.keyboard_light.as_ref().and_then(KeyboardLight::level)
                else {
                    return;
                };
                let level = stepped(level, up, fine);
                self.set_keyboard_light(level);
                Hud::Keyboard(level)
            }
            _ => return,
        };
        self.show_hud(window, hud);
    }

    fn show_hud(&mut self, window: &mut Window, hud: Hud) {
        if let Mode::Panel { .. } = self.mode {
            self.rebuild();
            window.request_redraw();
            return;
        }
        let fresh = matches!(self.mode, Mode::Hidden);
        self.mode = Mode::Hud(hud, Instant::now() + HUD_DURATION);
        if fresh {
            self.shown.jump_to(0.0);
        }
        self.reveal(window, false);
    }

    fn input(&mut self, window: &mut Window, input: &Event) {
        if let Event::PointerMove(pointer) | Event::PointerDown(pointer) = input {
            self.pointer = pointer.position;
        }
        let Mode::Panel { height, .. } = &self.mode else {
            return;
        };
        let rect = self.panel_rect(window, *height);
        match input {
            Event::Key {
                key: Key::Escape, ..
            } => {
                self.dismiss(window);
                return;
            }
            Event::PointerDown(pointer) if !contains(rect, pointer.position) => {
                self.dismiss(window);
                return;
            }
            _ => {}
        }
        if let Mode::Panel { ui, .. } = &mut self.mode
            && ui.handle(&moved(input, Point::new(rect.x, rect.y)))
        {
            window.request_redraw();
        }
        self.run_changes(window);
    }

    fn run_changes(&mut self, window: &mut Window) {
        if let Mode::Panel { ui, height } = &mut self.mode
            && let Some((menu, point)) = ui.take_menu_request()
        {
            let origin = place_beside(
                self.anchor,
                (window.size().0 as f32, window.size().1 as f32),
                (PANEL_WIDTH, *height),
            );
            report(
                "open the menu",
                window.show_menu(
                    menu,
                    MenuPlacement::AtPoint(Point::new(origin.x + point.x, origin.y + point.y)),
                ),
            );
        }
        let changes = std::mem::take(&mut *self.changes.borrow_mut());
        for change in changes {
            self.apply(window, change);
        }
    }
}

impl App for ControlCenter {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let now = Instant::now();
        let (shown, _) = self.shown.value_at(now);
        let palette = self.palette;
        let items = match &mut self.mode {
            Mode::Panel {
                ui,
                height: panel_height,
            } => {
                let rect = place_beside(
                    self.anchor,
                    (width as f32, height as f32),
                    (PANEL_WIDTH, *panel_height),
                );
                panel_items(ui, &palette, rect)
            }
            Mode::Hud(shown_hud, _) => {
                let placed = place_beside(self.anchor, (width as f32, height as f32), HUD_SIZE);
                vec![Item::Translate {
                    offset: Point::new(placed.x, placed.y),
                    items: hud(*shown_hud, &palette, &mut self.text),
                }]
            }
            Mode::Hidden => Vec::new(),
        };
        let lift = slide_from(self.anchor, SLIDE * (1.0 - shown));
        scene(
            width,
            height,
            vec![Item::Opacity {
                alpha: shown.clamp(0.0, 1.0),
                items: vec![Item::Translate {
                    offset: lift,
                    items,
                }],
            }],
        )
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::TogglePanel(Panel::ControlCenter) => self.toggle_panel(window),
            WindowEvent::Focus(false) if matches!(self.mode, Mode::Panel { .. }) => {
                self.dismiss(window);
            }
            WindowEvent::PanelAnchor {
                panel: Panel::ControlCenter,
                edge,
                rect,
                ..
            } => {
                self.anchor = Some(Anchor { edge, item: rect });
                window.request_redraw();
            }
            WindowEvent::VolumeKey { key, fine } => self.volume_key(window, key, fine),
            WindowEvent::BrightnessKey { key, fine } => self.brightness_key(window, key, fine),
            WindowEvent::Input(input) => self.input(window, &input),
            WindowEvent::Accessibility(request) => {
                if let Mode::Panel { ui, .. } = &mut self.mode
                    && ui.perform(&request)
                {
                    window.request_redraw();
                }
                self.run_changes(window);
            }
            WindowEvent::MenuAction(action) => {
                if let Some(ssid) = action.strip_prefix(JOIN).and_then(unhex) {
                    self.join(window, &ssid);
                } else if let Some(keyword) = action.strip_prefix(SESSION) {
                    self.end_session(window, keyword);
                } else if let Mode::Panel { ui, .. } = &mut self.mode {
                    ui.menu_action(&action);
                    self.run_changes(window);
                }
                self.rebuild();
                window.request_redraw();
            }
            WindowEvent::MediaKey(key) => {
                if let Some(command) = media_command(key) {
                    self.command_player(command);
                }
            }
            WindowEvent::AppearanceChanged(appearance) => {
                self.palette = appearance.palette();
                self.reduce_motion = appearance.reduce_motion;
                if let Mode::Panel { ui, .. } = &mut self.mode {
                    ui.set_appearance(appearance);
                }
            }
            _ => {}
        }
    }

    fn animating(&self) -> bool {
        let now = Instant::now();
        self.shown.value_at(now).1
            || matches!(&self.mode, Mode::Panel { ui, .. } if ui.is_animating())
    }

    fn deadline(&self) -> Option<Instant> {
        let now = Instant::now();
        match self.mode {
            Mode::Hud(_, until) => Some(until),
            Mode::Hidden if self.surface_shown && !self.shown.value_at(now).1 => Some(now),
            Mode::Panel { .. } => {
                let clock = self.playing().and_then(|player| {
                    let position = player.position_at(now)?;
                    let to_next_second = Duration::from_secs(1)
                        .saturating_sub(Duration::from_nanos(u64::from(position.subsec_nanos())));
                    Some(now + to_next_second.div_f64(player.rate.max(0.1)))
                });
                let meter = self.meter.as_ref().map(|_| now + METER_INTERVAL);
                clock.into_iter().chain(meter).min()
            }
            Mode::Hidden => None,
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.player_updates.fd()]
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) {
        let Some(latest) = self.player_updates.drain().pop() else {
            return;
        };
        self.players.update(latest);
        self.load_artwork();
        if matches!(self.mode, Mode::Panel { .. }) {
            self.rebuild();
            window.request_redraw();
        }
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        match self.mode {
            Mode::Panel { .. } => {
                if let Some(meter) = &self.meter {
                    let level = meter_level(meter.take_peak(), self.input_level.get());
                    self.input_level.set(level);
                }
                window.request_redraw();
            }
            Mode::Hud(_, until) if now >= until => self.dismiss(window),
            Mode::Hidden if self.surface_shown && !self.shown.value_at(now).1 => {
                self.surface_shown = false;
                report("hide", window.hide());
            }
            _ => {}
        }
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        match &self.mode {
            Mode::Panel { ui, .. } => Some(ui.accessibility()),
            _ => None,
        }
    }
}

pub fn role() -> Role {
    Role::Layer(LayerOptions {
        layer: LayerKind::Overlay,
        anchor: Edges::ALL,
        exclusive_zone: 0,
        keyboard: KeyboardFocus::None,
        namespace: "control-center".to_owned(),
    })
}

impl ControlCenter {
    pub fn new(
        window: &mut Window,
        text: TextSystem,
        session: Option<&Connection>,
        system: Option<&Connection>,
    ) -> std::io::Result<Self> {
        let audio = Audio::connect()
            .inspect_err(|error| tracing::warn!("no sound: {error}"))
            .ok();
        let player_updates = Inbox::new()?;
        let player_commands = session.map(|connection| {
            let (commands, received) = mpsc::channel();
            let (watching, outbox) = (connection.clone(), player_updates.outbox());
            std::thread::spawn(move || {
                if let Err(error) = media::watch(&watching, &outbox) {
                    tracing::warn!("cannot follow media players: {error}");
                }
            });
            let commanding = connection.clone();
            std::thread::spawn(move || media::run_commands(&commanding, &received));
            commands
        });
        window.set_input_region(Some(&[]));
        report("hide", window.hide());
        let appearance = window.appearance();
        Ok(Self {
            mode: Mode::Hidden,
            shown: Animated::new(0.0),
            surface_shown: false,
            reduce_motion: appearance.reduce_motion,
            palette: appearance.palette(),
            text,
            changes: Changes::default(),
            pointer: Point::new(0.0, 0.0),
            anchor: None,
            audio,
            network: system.map(NetworkManager::new),
            bluetooth: system.map(Bluetooth::new),
            backlight: Backlight::find(Path::new(SYSFS_BACKLIGHT)),
            keyboard_light: KeyboardLight::find(Path::new(SYSFS_LEDS)),
            system: system.cloned(),
            session: session.cloned(),
            players: Players::default(),
            player_updates,
            player_commands,
            artwork: None,
            meter: None,
            input_level: Rc::default(),
        })
    }
}
