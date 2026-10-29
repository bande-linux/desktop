use std::process::ExitCode;
use std::time::Instant;

use unbound_audio::{Audio, Level};
use unbound_bluetooth::Bluetooth;
use unbound_controlcenter::{
    Change, Changes, HUD_DURATION, HUD_SIZE, PANEL_SIZE, Status, card, panel, volume_hud,
};
use unbound_network::NetworkManager;
use unbound_toolkit::ui::Ui;
use unbound_toolkit::{
    App, Color, Edges, KeyboardFocus, LayerKind, LayerOptions, Palette, Role, Scene, TextSystem,
    VolumeKey, Window, WindowError, WindowEvent,
};
use zbus::blocking::{Connection, Proxy};

const NOTIFICATIONS: &str = "org.freedesktop.Notifications";
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";
const NOTIFICATION_CENTER: &str = "org.unbound.NotificationCenter";

enum Mode {
    Hidden,
    Volume(Instant),
    Panel(Box<Ui>),
}

struct ControlCenter {
    mode: Mode,
    level: Level,
    palette: Palette,
    hud_text: TextSystem,
    changes: Changes,
    audio: Option<Audio>,
    network: Option<NetworkManager>,
    bluetooth: Option<Bluetooth>,
    session: Option<Connection>,
}

fn report<E: std::fmt::Display>(what: &str, result: Result<(), E>) {
    if let Err(error) = result {
        eprintln!("unbound-controlcenter: cannot {what}: {error}");
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

impl ControlCenter {
    fn notification_center(&self) -> Option<Proxy<'_>> {
        let session = self.session.as_ref()?;
        Proxy::new(
            session,
            NOTIFICATIONS,
            NOTIFICATIONS_PATH,
            NOTIFICATION_CENTER,
        )
        .ok()
    }

    fn status(&self) -> Status {
        let wifi = self.network.as_ref().and_then(|network| {
            network
                .has_wifi()
                .ok()
                .filter(|has| *has)
                .and_then(|_| network.wifi_enabled().ok())
        });
        let bluetooth = self.bluetooth.as_ref().and_then(|bluetooth| {
            bluetooth
                .available()
                .ok()
                .filter(|available| *available)
                .and_then(|_| bluetooth.powered().ok())
        });
        let do_not_disturb = self
            .notification_center()
            .and_then(|center| center.get_property::<bool>("DoNotDisturb").ok())
            .unwrap_or(false);
        let volume = self.audio.as_ref().and_then(|audio| {
            audio
                .snapshot()
                .default_output()
                .map(|output| output.level.volume)
        });
        Status {
            wifi,
            bluetooth,
            do_not_disturb,
            volume,
        }
    }

    fn apply(&mut self, change: Change) {
        match change {
            Change::Wifi(on) => {
                if let Some(network) = &self.network {
                    report("switch Wi-Fi", network.set_wifi_enabled(on));
                }
            }
            Change::Bluetooth(on) => {
                if let Some(bluetooth) = &self.bluetooth {
                    report("switch Bluetooth", bluetooth.set_powered(on));
                }
            }
            Change::DoNotDisturb(on) => {
                if let Some(center) = self.notification_center() {
                    report(
                        "switch Do Not Disturb",
                        center.set_property("DoNotDisturb", on),
                    );
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
                    self.level = level;
                }
            }
        }
    }

    fn show(&mut self, window: &mut Window, size: (u32, u32)) {
        report("resize", window.set_layer_size(size));
        report("show", window.show());
        window.request_redraw();
    }

    fn hide(&mut self, window: &mut Window) {
        self.mode = Mode::Hidden;
        report("hide", window.hide());
    }

    fn toggle_panel(&mut self, window: &mut Window) {
        if matches!(self.mode, Mode::Panel(_)) {
            self.hide(window);
            return;
        }
        let ui = panel(
            self.status(),
            self.palette,
            TextSystem::new(),
            &self.changes,
        );
        self.mode = Mode::Panel(Box::new(ui));
        self.show(window, PANEL_SIZE);
    }

    fn volume_key(&mut self, window: &mut Window, key: VolumeKey, fine: bool) {
        let (Some(audio), Some(key)) = (&self.audio, audio_key(key)) else {
            return;
        };
        match audio.press(key, fine) {
            Ok(level) => self.level = level,
            Err(error) => {
                eprintln!("unbound-controlcenter: cannot change the volume: {error}");
                return;
            }
        }
        if matches!(self.mode, Mode::Panel(_)) {
            return;
        }
        self.mode = Mode::Volume(Instant::now() + HUD_DURATION);
        self.show(window, HUD_SIZE);
    }
}

impl App for ControlCenter {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        match &mut self.mode {
            Mode::Panel(ui) => {
                let mut scene = ui.scene(width, height);
                scene.background = Color::TRANSPARENT;
                scene.items.insert(0, card(&self.palette, (width, height)));
                scene
            }
            Mode::Volume(_) => volume_hud(self.level, &self.palette, &mut self.hud_text),
            Mode::Hidden => Scene {
                width,
                height,
                background: Color::TRANSPARENT,
                items: Vec::new(),
            },
        }
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::ToggleControlCenter => self.toggle_panel(window),
            WindowEvent::VolumeKey { key, fine } => self.volume_key(window, key, fine),
            WindowEvent::Input(input) => {
                if let Mode::Panel(ui) = &mut self.mode
                    && ui.handle(&input)
                {
                    window.request_redraw();
                }
                let changes = std::mem::take(&mut *self.changes.borrow_mut());
                for change in changes {
                    self.apply(change);
                }
            }
            WindowEvent::AppearanceChanged(appearance) => {
                self.palette = appearance.palette();
                if let Mode::Panel(ui) = &mut self.mode {
                    ui.context().palette = self.palette;
                }
            }
            _ => {}
        }
    }

    fn deadline(&self) -> Option<Instant> {
        match self.mode {
            Mode::Volume(until) => Some(until),
            Mode::Hidden | Mode::Panel(_) => None,
        }
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        if matches!(self.mode, Mode::Volume(until) if now >= until) {
            self.hide(window);
        }
    }
}

fn run() -> Result<(), WindowError> {
    let audio = Audio::connect()
        .inspect_err(|error| eprintln!("unbound-controlcenter: no sound: {error}"))
        .ok();
    let system = Connection::system()
        .inspect_err(|error| eprintln!("unbound-controlcenter: no system bus: {error}"))
        .ok();
    let session = Connection::session()
        .inspect_err(|error| eprintln!("unbound-controlcenter: no session bus: {error}"))
        .ok();
    let mut window = Window::open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Overlay,
            anchor: Edges {
                top: true,
                right: true,
                left: false,
                bottom: false,
            },
            exclusive_zone: 0,
            keyboard: KeyboardFocus::None,
            namespace: "control-center".to_owned(),
        }),
        HUD_SIZE,
    )?;
    window.hide()?;
    let mut center = ControlCenter {
        mode: Mode::Hidden,
        level: Level {
            volume: 0.0,
            muted: false,
        },
        palette: window.appearance().palette(),
        hud_text: TextSystem::new(),
        changes: Changes::default(),
        audio,
        network: system.as_ref().map(NetworkManager::new),
        bluetooth: system.as_ref().map(Bluetooth::new),
        session,
    };
    unbound_toolkit::run(&mut window, &mut center)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("unbound-controlcenter: {error}");
            ExitCode::FAILURE
        }
    }
}
