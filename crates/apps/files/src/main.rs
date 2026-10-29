use std::path::PathBuf;
use std::process::ExitCode;

use unbound_files::{Browser, FilesView};
use unbound_toolkit::{App, Appearance, Role, Scene, TextSystem, Window, WindowEvent};

const APP_ID: &str = "org.unbound.Files";
const WINDOW_SIZE: (u32, u32) = (920, 560);

struct Files {
    view: FilesView,
    title: String,
}

impl App for Files {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.view.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        if let WindowEvent::AppearanceChanged(appearance) = event {
            self.view.set_palette(appearance.palette());
        } else if let WindowEvent::Input(input) = event
            && self.view.handle(&input)
        {
            let title = self.view.title();
            if title != self.title {
                window.set_title(&title);
                self.title = title;
            }
            window.request_redraw();
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let start = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .or(home)
        .unwrap_or_else(|| PathBuf::from("/"));
    let view = FilesView::new(
        Browser::open(&start)?,
        Appearance::default().palette(),
        TextSystem::new(),
    );
    let title = view.title();
    let mut window = Window::open(Role::toplevel(&title, APP_ID), WINDOW_SIZE)?;
    let mut files = Files { view, title };
    unbound_toolkit::run(&mut window, &mut files)?;
    Ok(())
}

fn main() -> ExitCode {
    unbound_log::init("unbound-files");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
