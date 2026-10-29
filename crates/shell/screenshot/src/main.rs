use std::path::PathBuf;
use std::process::ExitCode;

use unbound_screenshot::area::{AreaOutcome, AreaSelection, Region, crop};
use unbound_screenshot::{desktop_dir, save};
use unbound_toolkit::{
    App, Edges, KeyboardFocus, LayerKind, LayerOptions, Role, Scene, TextSystem, Window,
    WindowError, WindowEvent,
};

struct Screenshots {
    home: PathBuf,
    config_home: PathBuf,
    text: TextSystem,
    selection: Option<AreaSelection>,
}

impl Screenshots {
    fn capture(&self, window: &mut Window, region: Option<Region>) {
        let image = match window.capture_screen() {
            Ok(image) => image,
            Err(error) => {
                eprintln!("unbound-screenshot: cannot capture the screen: {error}");
                return;
            }
        };
        let image = match region {
            Some(region) => match crop(&image, region) {
                Some(cropped) => cropped,
                None => {
                    eprintln!("unbound-screenshot: the chosen area is not on the screen");
                    return;
                }
            },
            None => image,
        };
        let user_dirs = std::fs::read_to_string(self.config_home.join("user-dirs.dirs")).ok();
        let dir = desktop_dir(&self.home, user_dirs.as_deref());
        let taken = jiff::Zoned::now().datetime();
        match save(image, &dir, taken) {
            Ok(path) => eprintln!("unbound-screenshot: saved {}", path.display()),
            Err(error) => eprintln!(
                "unbound-screenshot: cannot save to {}: {error}",
                dir.display()
            ),
        }
    }

    fn end_selection(&mut self, window: &mut Window) {
        self.selection = None;
        if let Err(error) = window.hide() {
            eprintln!("unbound-screenshot: cannot hide the selection: {error}");
        }
    }
}

impl App for Screenshots {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        match &self.selection {
            Some(selection) => selection.scene(width, height, &mut self.text),
            None => AreaSelection::default().scene(width, height, &mut self.text),
        }
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::CaptureScreen if self.selection.is_none() => self.capture(window, None),
            WindowEvent::CaptureArea if self.selection.is_none() => {
                self.selection = Some(AreaSelection::default());
                if let Err(error) = window.show() {
                    eprintln!("unbound-screenshot: cannot show the selection: {error}");
                }
            }
            WindowEvent::Input(input) => {
                let Some(selection) = self.selection.as_mut() else {
                    return;
                };
                match selection.handle(&input) {
                    AreaOutcome::Selecting => window.request_redraw(),
                    AreaOutcome::Cancelled => self.end_selection(window),
                    AreaOutcome::Chosen(region) => {
                        self.end_selection(window);
                        self.capture(window, Some(region));
                    }
                }
            }
            _ => {}
        }
    }
}

fn run() -> Result<(), WindowError> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| WindowError::Io(std::io::Error::other("HOME is not set")))?;
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let mut window = Window::open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Overlay,
            anchor: Edges::ALL,
            exclusive_zone: -1,
            keyboard: KeyboardFocus::Exclusive,
            namespace: "screenshot".to_owned(),
        }),
        (0, 0),
    )?;
    window.hide()?;
    let mut screenshots = Screenshots {
        home,
        config_home,
        text: TextSystem::new(),
        selection: None,
    };
    unbound_toolkit::run(&mut window, &mut screenshots)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("unbound-screenshot: {error}");
            ExitCode::FAILURE
        }
    }
}
