use std::path::PathBuf;
use std::process::ExitCode;

use unbound_screenshot::{desktop_dir, save};
use unbound_toolkit::{
    App, Color, Edges, KeyboardFocus, LayerKind, LayerOptions, Role, Scene, Window, WindowError,
    WindowEvent,
};

struct Screenshots {
    home: PathBuf,
    config_home: PathBuf,
}

impl Screenshots {
    fn capture(&self, window: &mut Window) {
        let image = match window.capture_screen() {
            Ok(image) => image,
            Err(error) => {
                eprintln!("unbound-screenshot: cannot capture the screen: {error}");
                return;
            }
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
}

impl App for Screenshots {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        Scene {
            width,
            height,
            background: Color::TRANSPARENT,
            items: Vec::new(),
        }
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        if event == WindowEvent::CaptureScreen {
            self.capture(window);
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
            keyboard: KeyboardFocus::None,
            namespace: "screenshot".to_owned(),
        }),
        (0, 0),
    )?;
    window.hide()?;
    unbound_toolkit::run(&mut window, &mut Screenshots { home, config_home })
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
