use std::path::Path;

use bande_config::Config;
use bande_status::hub::StatusHub;
use bande_toolkit::{App, TextSystem, Window};
use zbus::blocking::Connection;

use crate::bar_client::{BarClient, bar_role};
use crate::rail::TaskbarView;
use crate::status_client::StatusClient;
use crate::taskbar_client::Taskbar;

pub use crate::status_client::{Restart, status_hub};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Display {
    pub output: String,
    pub main: bool,
}

pub struct Services<'a> {
    pub text: &'a TextSystem,
    pub session: Option<&'a Connection>,
    pub status: &'a StatusHub,
    pub config_home: Option<&'a Path>,
    pub home: Option<&'a Path>,
}

pub struct Bar {
    pub surfaces: Vec<(Window, Box<dyn App>)>,
    pub restart: Restart,
}

pub fn open(
    beside: &Window,
    display: &Display,
    config: &Config,
    services: &Services<'_>,
) -> Result<Bar, Box<dyn std::error::Error>> {
    let text = services.text;
    let homes = (services.config_home, services.home);
    let open_window = |role, size| -> Result<Window, Box<dyn std::error::Error>> {
        let mut window = beside.open_beside_on(&display.output, role, size)?;
        window.share_fonts(text);
        Ok(window)
    };
    let bar = &config.bar;
    let icon_size = bar.icon_size as f32;
    let (role, size) = bar_role(bar.edge, icon_size);
    let window = open_window(role, size)?;
    let taskbar = Taskbar::new(
        &window,
        &display.output,
        bar,
        text.sharing_fonts(),
        services.session,
        homes,
    )?;
    let view = TaskbarView::new(
        window.appearance().palette(),
        text.sharing_fonts(),
        bar.edge,
        bande_xdg::data_dirs(),
        icon_size,
    );
    let status = StatusClient::new(view, display.clone(), services.status)?;
    let restart = status.restart();
    let surfaces: Vec<(Window, Box<dyn App>)> =
        vec![(window, Box::new(BarClient::new(taskbar, status)))];
    Ok(Bar { surfaces, restart })
}
