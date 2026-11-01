mod cx;
mod instance;
mod menus;
mod runner;

use std::ffi::OsString;
use std::os::fd::BorrowedFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use accesskit::TreeUpdate;
use bande_app_menu::Entry;
use bande_toolkit::ui::Ui;
use bande_toolkit::{Appearance, Rect, Scene, Window, WindowEvent};

pub use cx::{Cx, WindowId};

pub type Error = Box<dyn std::error::Error>;

const VERSION_FLAG: &str = "--version";

pub struct Info {
    pub id: &'static str,
    pub package: &'static str,
    pub version: &'static str,
    pub name: fn() -> String,
    pub keys: Keys,
    pub settings: Option<Settings>,
}

pub struct Settings {
    pub title: fn() -> String,
    pub size: (u32, u32),
    pub view: fn(Appearance) -> Box<dyn View>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keys {
    Standard,
    Shifted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Main,
    Dialog,
    Utility,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Menus {
    pub menus: Vec<Entry>,
    pub window: Vec<Entry>,
}

pub trait View {
    fn scene(&mut self, window: &mut Window) -> Scene;

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool;

    fn set_appearance(&mut self, appearance: Appearance);

    fn set_active(&mut self, _active: bool) {}

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        Vec::new()
    }

    fn source_ready(&mut self, _window: &mut Window, _index: usize) -> bool {
        false
    }

    fn deadline(&self) -> Option<Instant> {
        None
    }

    fn tick(&mut self, _window: &mut Window, _now: Instant) -> bool {
        false
    }

    fn animating(&self) -> bool {
        false
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        None
    }

    fn text_input_area(&self) -> Option<Rect> {
        None
    }
}

impl View for Ui {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        Ui::scene(self, width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        window
            .drive(self, event)
            .inspect_err(|error| tracing::warn!("{error}"))
            .is_ok_and(|unhandled| unhandled.is_none())
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        Ui::set_appearance(self, appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.set_window_active(active);
    }

    fn animating(&self) -> bool {
        self.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(Ui::accessibility(self))
    }

    fn text_input_area(&self) -> Option<Rect> {
        Ui::text_input_area(self)
    }
}

pub trait Program: Sized + 'static {
    type View: View;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, Error>;

    fn menus(&self, _cx: &Cx<Self>) -> Menus {
        Menus::default()
    }

    fn command(&mut self, _cx: &mut Cx<Self>, _window: Option<WindowId>, _command: &str) {}

    fn open(&mut self, _cx: &mut Cx<Self>, _arguments: Vec<String>, _directory: &Path) {}

    fn reopen(&mut self, cx: &mut Cx<Self>);

    fn window_closed(&mut self, _cx: &mut Cx<Self>, _window: WindowId, _view: Self::View) {}

    fn changed(&mut self, _cx: &mut Cx<Self>, _window: WindowId) {}

    fn close_tab(&mut self, _cx: &mut Cx<Self>, _window: WindowId) -> bool {
        false
    }

    fn should_close(&mut self, _cx: &mut Cx<Self>, _window: WindowId) -> bool {
        true
    }

    fn should_quit(&mut self, cx: &mut Cx<Self>) -> bool {
        let windows: Vec<WindowId> = cx.windows().collect();
        windows
            .into_iter()
            .all(|window| self.should_close(cx, window))
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        Vec::new()
    }

    fn source_ready(&mut self, _cx: &mut Cx<Self>, _index: usize) {}

    fn deadline(&self) -> Option<Instant> {
        None
    }

    fn tick(&mut self, _cx: &mut Cx<Self>, _now: Instant) {}
}

pub fn run<P: Program>(info: &'static Info) -> ExitCode {
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    if arguments == [VERSION_FLAG] {
        println!("{} {}", info.package, info.version);
        return ExitCode::SUCCESS;
    }
    bande_log::init(info.package);
    let arguments = match arguments
        .into_iter()
        .map(OsString::into_string)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(arguments) => arguments,
        Err(argument) => {
            tracing::error!("the argument {} is not UTF-8", argument.display());
            return ExitCode::FAILURE;
        }
    };
    match runner::run::<P>(info, arguments, None) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

pub fn run_on<P: Program>(
    info: &'static Info,
    arguments: Vec<String>,
    display: UnixStream,
) -> Result<(), Error> {
    runner::run::<P>(info, arguments, Some(display))
}
