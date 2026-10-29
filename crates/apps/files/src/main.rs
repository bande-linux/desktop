use std::path::PathBuf;
use std::process::ExitCode;

use unbound_config::{Appearance, Config, config_path};
use unbound_files::{Browser, FilesView};
use unbound_toolkit::{App, Palette, Role, Scene, Scheme, TextSystem, Window, WindowEvent};

const APP_ID: &str = "unbound-files";
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
        if let WindowEvent::Input(input) = event
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

fn scheme(home: Option<&std::path::Path>) -> Scheme {
    let config_home = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let config = config_path(config_home.as_deref(), home)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|source| Config::parse(&source).ok())
        .unwrap_or_default();
    match config.appearance {
        Appearance::Dark => Scheme::Dark,
        Appearance::Light | Appearance::Automatic => Scheme::Light,
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let start = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| home.clone())
        .unwrap_or_else(|| PathBuf::from("/"));
    let view = FilesView::new(
        Browser::open(&start)?,
        Palette::for_scheme(scheme(home.as_deref())),
        TextSystem::new(),
    );
    let title = view.title();
    let mut window = Window::open(Role::toplevel(&title, APP_ID), WINDOW_SIZE)?;
    let mut files = Files { view, title };
    unbound_toolkit::run(&mut window, &mut files)?;
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("unbound-files: {error}");
            ExitCode::FAILURE
        }
    }
}
