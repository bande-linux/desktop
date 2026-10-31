use std::path::Path;
use std::rc::Rc;

use bande_config::Config;
use bande_toolkit::{App, TextSystem, Window};
use zbus::blocking::Connection;

use crate::combined_client::{CombinedClient, combined_role};
use crate::dock::surface_size;
use crate::dock_client::{Dock, dock_role};
use crate::menu_bar_client::{MENU_BAR_SIZE, MenuBarClient, menu_bar_role};
use crate::menu_search_client::{SearchCard, search_role};
use crate::rail::RailView;

pub use crate::menu_bar_client::Restart;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Display {
    pub output: String,
    pub main: bool,
}

pub type NotificationCenter = Rc<dyn Fn()>;

pub struct Services<'a> {
    pub text: &'a TextSystem,
    pub session: Option<&'a Connection>,
    pub system: Option<&'a Connection>,
    pub config_home: Option<&'a Path>,
    pub home: Option<&'a Path>,
    pub notification_center: NotificationCenter,
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
    let buses = (services.session, services.system);
    let place = (display.clone(), services.notification_center.clone());
    let open_window = |role, size| -> Result<Window, Box<dyn std::error::Error>> {
        let mut window = beside.open_beside_on(&display.output, role, size)?;
        window.share_fonts(text);
        Ok(window)
    };
    let bar = &config.bar;
    let layout = (bar.menu_bar, bar.edge);
    let icon_size = bar.icon_size as f32;
    let mut surfaces: Vec<(Window, Box<dyn App>)> = Vec::new();
    let (handoff, restart) = match bar.menu_bar.combines(bar.edge) {
        true => {
            let (role, size) = combined_role(icon_size);
            let window = open_window(role, size)?;
            let dock = Dock::new(
                &window,
                &display.output,
                bar,
                text.sharing_fonts(),
                services.session,
                homes,
            )?;
            let view = RailView::new(
                window.appearance().palette(),
                text.sharing_fonts(),
                bande_xdg::data_dirs(),
                icon_size,
            );
            let status = MenuBarClient::with_view(view, place, buses, layout)?;
            let shared = (status.handoff(), status.restart());
            surfaces.push((window, Box::new(CombinedClient::new(dock, status))));
            shared
        }
        false => {
            let mut window = open_window(menu_bar_role(), MENU_BAR_SIZE)?;
            let client =
                MenuBarClient::new(&mut window, text.sharing_fonts(), place, buses, layout)?;
            let shared = (client.handoff(), client.restart());
            surfaces.push((window, Box::new(client)));
            if display.main {
                let window = open_window(
                    dock_role(bar.edge, icon_size),
                    surface_size(bar.edge, icon_size, (1, 1)),
                )?;
                let dock = Dock::new(
                    &window,
                    &display.output,
                    bar,
                    text.sharing_fonts(),
                    services.session,
                    homes,
                )?;
                surfaces.push((window, Box::new(dock)));
            }
            shared
        }
    };
    if display.main {
        let mut window = open_window(search_role(), (0, 0))?;
        let card = SearchCard::new(&mut window, text.sharing_fonts(), handoff);
        surfaces.push((window, Box::new(card)));
    }
    Ok(Bar { surfaces, restart })
}
