use std::os::fd::BorrowedFd;
use std::process::ExitCode;
use std::sync::mpsc::Sender;

use unbound_settings::{Command, Feed, L10N, SettingsView, feed, follow_config, worker};
use unbound_toolkit::{App, Rect, Role, Scene, TextSystem, Window, WindowEvent};

const APP_ID: &str = "org.unbound.Settings";
const WINDOW_SIZE: (u32, u32) = (900, 600);

struct Settings {
    view: SettingsView,
    feed: Feed,
    commands: Sender<Command>,
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
        for (id, value) in self.view.take_changes() {
            if self.commands.send(Command::Set(id, value)).is_err() {
                tracing::warn!("the settings worker has stopped");
            }
        }
        window.request_redraw();
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.feed.fd()]
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) {
        for update in self.feed.drain() {
            self.view.apply(update);
        }
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
    let session = zbus::blocking::Connection::session()?;
    let (feed, updates) = feed()?;
    follow_config(&session, updates.clone());
    let commands = worker(&session, updates);
    let mut window = Window::open(Role::toplevel(&L10N.tr("title"), APP_ID), WINDOW_SIZE)?;
    let view = SettingsView::new(window.appearance().palette(), TextSystem::new());
    let mut settings = Settings {
        view,
        feed,
        commands,
    };
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
