use std::process::ExitCode;

use unbound_settings::{L10N, SettingsClient, SettingsView};
use unbound_toolkit::{App, Rect, Role, Scene, TextSystem, Window, WindowEvent};

const APP_ID: &str = "org.unbound.Settings";
const WINDOW_SIZE: (u32, u32) = (900, 600);

struct Settings {
    view: SettingsView,
    client: SettingsClient,
}

impl Settings {
    fn refresh(&mut self) {
        for (id, value) in self.client.read_all() {
            self.view.set_value(id, value);
        }
    }

    fn write_changes(&mut self) {
        let changes = self.view.take_changes();
        if changes.is_empty() {
            return;
        }
        let failed = changes
            .iter()
            .find_map(|(id, value)| self.client.set(*id, value).err());
        self.view.set_message(
            failed.map(|error| L10N.tr_args("not-changed", [("error", error.to_string().into())])),
        );
        self.refresh();
    }
}

impl App for Settings {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.view.scene(width, height)
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
            _ => return,
        }
        self.write_changes();
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
    let connection = zbus::blocking::Connection::session()?;
    let client = SettingsClient::connect(&connection)?;
    let mut window = Window::open(Role::toplevel(&L10N.tr("title"), APP_ID), WINDOW_SIZE)?;
    let view = SettingsView::new(window.appearance().palette(), TextSystem::new());
    let mut settings = Settings { view, client };
    settings.refresh();
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
