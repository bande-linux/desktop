use std::process::ExitCode;

use unbound_settings::{Key, SettingsClient, SettingsView};
use unbound_toolkit::{App, Role, Scene, TextSystem, Window, WindowEvent};

const TITLE: &str = "System Settings";
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
        let input = match event {
            WindowEvent::Input(input) => input,
            WindowEvent::AppearanceChanged(appearance) => {
                self.view.set_palette(appearance.palette());
                return;
            }
            _ => return,
        };
        if !self.view.handle(&input) {
            return;
        }
        for (key, value) in self.view.take_changes() {
            let failed = self.client.set(key, &value).err();
            self.view
                .set_message(failed.map(|error| format!("Not changed: {error}")));
            self.refresh(key);
        }
        window.request_redraw();
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let connection = zbus::blocking::Connection::session()?;
    let client = SettingsClient::connect(&connection)?;
    let mut window = Window::open(Role::toplevel(TITLE, APP_ID), WINDOW_SIZE)?;
    let view = SettingsView::new(window.appearance().palette(), TextSystem::new());
    let mut settings = Settings { view, client };
    for key in KEYS {
        settings.refresh(key);
    }
    unbound_toolkit::run(&mut window, &mut settings)?;
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("unbound-settings: {error}");
            ExitCode::FAILURE
        }
    }
}
