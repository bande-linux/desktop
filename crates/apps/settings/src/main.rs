use std::process::ExitCode;

use unbound_config::Appearance;
use unbound_settings::{Key, L10N, SettingsClient, SettingsView, Value};
use unbound_toolkit::{App, Palette, Role, Scene, Scheme, TextSystem, Window, WindowEvent};

const APP_ID: &str = "org.unbound.Settings";
const WINDOW_SIZE: (u32, u32) = (900, 540);
const KEYS: [Key; 5] = [
    Key::Appearance,
    Key::DockPosition,
    Key::DimAfter,
    Key::LockAfter,
    Key::SleepAfter,
];

struct Settings {
    view: SettingsView,
    client: SettingsClient,
}

impl Settings {
    fn refresh(&mut self, key: Key) {
        if let Ok(value) = self.client.get(key) {
            self.view.set_value(key, value);
        }
    }
}

impl App for Settings {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.view.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let WindowEvent::Input(input) = event else {
            return;
        };
        if !self.view.handle(&input) {
            return;
        }
        for (key, value) in self.view.take_changes() {
            let failed = self.client.set(key, &value).err();
            self.view.set_message(
                failed.map(|error| {
                    L10N.tr_args("not-changed", [("error", error.to_string().into())])
                }),
            );
            self.refresh(key);
        }
        window.request_redraw();
    }
}

fn scheme(appearance: Option<&Value>) -> Scheme {
    match appearance {
        Some(Value::Keyword(keyword)) if keyword == Appearance::Dark.keyword() => Scheme::Dark,
        _ => Scheme::Light,
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let connection = zbus::blocking::Connection::session()?;
    let client = SettingsClient::connect(&connection)?;
    let appearance = client.get(Key::Appearance).ok();
    let view = SettingsView::new(
        Palette::for_scheme(scheme(appearance.as_ref())),
        TextSystem::new(),
    );
    let mut settings = Settings { view, client };
    for key in KEYS {
        settings.refresh(key);
    }
    let mut window = Window::open(Role::toplevel(&L10N.tr("title"), APP_ID), WINDOW_SIZE)?;
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
