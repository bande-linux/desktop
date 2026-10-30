use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use accesskit::TreeUpdate;
use unbound_audio::{Audio, Level};
use unbound_bluetooth::Bluetooth;
use unbound_controlcenter::{
    Change, Changes, EDGE, HUD_DURATION, HUD_SIZE, Hud, KeyboardLight, PANEL_WIDTH, SYSFS_LEDS,
    Status, blur_rects, card, chooser_menu, hud, material, panel, scene, stepped,
};
use unbound_network::NetworkManager;
use unbound_notify::client::NotificationCenter;
use unbound_power::backlight::{Backlight, SYSFS_BACKLIGHT};
use unbound_power::logind::own_session;
use unbound_toolkit::ui::{Animated, Event, Key, Pointer, Scroll, Spring, Ui};
use unbound_toolkit::{
    App, BrightnessKey, Edges, Item, KeyboardFocus, LayerKind, LayerOptions, MenuPlacement,
    Palette, Point, Rect, Role, Scene, TextSystem, VolumeKey, Window, WindowError, WindowEvent,
};
use unbound_xdg::spawn_detached;
use zbus::blocking::Connection;

const JOIN: &str = "join:";
const OUTPUT: &str = "output:";
const SLIDE: f32 = 8.0;

enum Mode {
    Hidden,
    Hud(Hud, Instant),
    Panel { ui: Box<Ui>, height: f32 },
}

struct ControlCenter {
    mode: Mode,
    shown: Animated,
    surface_shown: bool,
    reduce_motion: bool,
    palette: Palette,
    hud_text: TextSystem,
    changes: Changes,
    pointer: Point,
    audio: Option<Audio>,
    network: Option<NetworkManager>,
    bluetooth: Option<Bluetooth>,
    system: Option<Connection>,
    session: Option<Connection>,
    backlight: Option<Backlight>,
    keyboard_light: Option<KeyboardLight>,
    blur: Option<Vec<Rect>>,
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
        Status {
            wifi,
            network,
            bluetooth,
            do_not_disturb,
            volume: output.map(|output| output.level.volume),
            output: output
                .filter(|_| {
                    snapshot
                        .as_ref()
                        .is_some_and(|found| found.outputs.len() > 1)
                })
                .map(|output| output.description.clone()),
            brightness: self.brightness(),
            keyboard_brightness: self.keyboard_light.as_ref().and_then(KeyboardLight::level),
        }
    }

    fn panel_rect(&self, window: &Window, height: f32) -> Rect {
        let (width, _) = window.size();
        Rect::new(width as f32 - EDGE - PANEL_WIDTH, EDGE, PANEL_WIDTH, height)
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

    fn choose_network(&self, window: &mut Window) {
        let Some(network) = &self.network else {
            return;
        };
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

    fn choose_output(&self, window: &mut Window) {
        let Some(audio) = &self.audio else {
            return;
        };
        let snapshot = audio.snapshot();
        let choices: Vec<_> = snapshot
            .outputs
            .iter()
            .map(|output| {
                (
                    output.description.clone(),
                    format!("{OUTPUT}{}", output.name),
                    snapshot.default_output.as_deref() == Some(output.name.as_str()),
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
            let token = window.activation_token("org.unbound.Settings").ok();
            let command = ["unbound-settings".to_owned(), "wifi".to_owned()];
            if let Err(error) = spawn_detached(&command, None, token.as_deref()) {
                tracing::warn!("cannot open Wi-Fi settings: {error}");
            }
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
            Change::ChooseNetwork => self.choose_network(window),
            Change::ChooseOutput => self.choose_output(window),
        }
    }

    fn rebuild(&mut self) {
        if !matches!(self.mode, Mode::Panel { .. }) {
            return;
        }
        let (fresh, fresh_height) = panel(
            &self.status(),
            self.palette,
            TextSystem::new(),
            &self.changes,
        );
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

    fn dismiss(&mut self, window: &mut Window) {
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
        let (ui, height) = panel(
            &self.status(),
            self.palette,
            TextSystem::new(),
            &self.changes,
        );
        self.mode = Mode::Panel {
            ui: Box::new(ui),
            height,
        };
        self.shown.jump_to(0.0);
        self.reveal(window, true);
    }

    fn volume_key(&mut self, window: &mut Window, key: VolumeKey, fine: bool) {
        let (Some(audio), Some(key)) = (&self.audio, audio_key(key)) else {
            return;
        };
        match audio.press(key, fine) {
            Ok(level) => self.show_hud(window, Hud::Volume(level)),
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
        let mut items = Vec::new();
        let palette = material(self.palette, self.blur.is_some());
        let mut shapes = Vec::new();
        match &mut self.mode {
            Mode::Panel {
                ui,
                height: panel_height,
            } => {
                let rect = Rect::new(
                    width as f32 - EDGE - PANEL_WIDTH,
                    EDGE,
                    PANEL_WIDTH,
                    *panel_height,
                );
                card(&palette, rect, &mut items);
                shapes.push(rect);
                let content = ui.scene(PANEL_WIDTH as u32, *panel_height as u32);
                items.push(Item::Translate {
                    offset: Point::new(rect.x, rect.y),
                    items: content.items,
                });
            }
            Mode::Hud(shown_hud, _) => {
                let origin = Point::new(width as f32 - EDGE - HUD_SIZE.0, EDGE);
                shapes.push(Rect::new(origin.x, origin.y, HUD_SIZE.0, HUD_SIZE.1));
                items.push(Item::Translate {
                    offset: origin,
                    items: hud(*shown_hud, &palette, &mut self.hud_text),
                });
            }
            Mode::Hidden => {}
        }
        let lift = -SLIDE * (1.0 - shown);
        let rects: Vec<Rect> = shapes
            .into_iter()
            .filter(|_| shown > 0.5)
            .flat_map(|rect| blur_rects(Rect::new(rect.x, rect.y + lift, rect.width, rect.height)))
            .collect();
        if self.blur.as_ref().is_some_and(|known| *known != rects) {
            match window.set_background_blur(&rects) {
                Ok(()) => self.blur = Some(rects),
                Err(error) => {
                    tracing::info!("Control Center is not blurred: {error}");
                    self.blur = None;
                }
            }
        }
        scene(
            width,
            height,
            vec![Item::Opacity {
                alpha: shown.clamp(0.0, 1.0),
                items: vec![Item::Translate {
                    offset: Point::new(0.0, lift),
                    items,
                }],
            }],
        )
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::ToggleControlCenter => self.toggle_panel(window),
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
                } else if let Some(name) = action.strip_prefix(OUTPUT)
                    && let Some(audio) = &self.audio
                    && let Some(device) = audio.snapshot().device(name).cloned()
                {
                    report("choose the output", audio.set_default(&device));
                }
                self.rebuild();
                window.request_redraw();
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
        match self.mode {
            Mode::Hud(_, until) => Some(until),
            Mode::Hidden if self.surface_shown && !self.shown.value_at(Instant::now()).1 => {
                Some(Instant::now())
            }
            Mode::Hidden | Mode::Panel { .. } => None,
        }
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        match self.mode {
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

fn run() -> Result<(), WindowError> {
    let audio = Audio::connect()
        .inspect_err(|error| tracing::warn!("no sound: {error}"))
        .ok();
    let system = Connection::system()
        .inspect_err(|error| tracing::warn!("no system bus: {error}"))
        .ok();
    let session = Connection::session()
        .inspect_err(|error| tracing::warn!("no session bus: {error}"))
        .ok();
    let mut window = Window::open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Overlay,
            anchor: Edges::ALL,
            exclusive_zone: 0,
            keyboard: KeyboardFocus::None,
            namespace: "control-center".to_owned(),
        }),
        (0, 0),
    )?;
    window.set_input_region(Some(&[]));
    window.hide()?;
    let appearance = window.appearance();
    let mut center = ControlCenter {
        mode: Mode::Hidden,
        shown: Animated::new(0.0),
        surface_shown: false,
        reduce_motion: appearance.reduce_motion,
        palette: appearance.palette(),
        hud_text: TextSystem::new(),
        changes: Changes::default(),
        pointer: Point::new(0.0, 0.0),
        audio,
        network: system.as_ref().map(NetworkManager::new),
        bluetooth: system.as_ref().map(Bluetooth::new),
        backlight: Backlight::find(Path::new(SYSFS_BACKLIGHT)),
        keyboard_light: KeyboardLight::find(Path::new(SYSFS_LEDS)),
        blur: Some(vec![Rect::default()]),
        system,
        session,
    };
    unbound_toolkit::run(&mut window, &mut center)
}

fn main() -> ExitCode {
    unbound_log::init("unbound-controlcenter");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
