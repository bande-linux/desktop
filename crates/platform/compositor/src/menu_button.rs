use std::sync::{Mutex, PoisonError};

use bande_graphics::menu_button_area;
use bande_protocols::server::bande_shell_v1::BandeShellV1;
use bande_protocols::server::gtk_shell1::{self, GtkShell1};
use bande_protocols::server::gtk_surface1::{self, GtkSurface1};
use bande_protocols::server::org_kde_kwin_appmenu::{self, OrgKdeKwinAppmenu};
use bande_protocols::server::org_kde_kwin_appmenu_manager::{self, OrgKdeKwinAppmenuManager};
use smithay::desktop::layer_map_for_output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Logical, Rectangle, SERIAL_COUNTER, Serial};
use smithay::wayland::compositor::with_states;

use crate::motion::BAR_NAMESPACE;
use crate::shell::toplevel_names;
use crate::state::{MENU_BAR_NAMESPACE, State};

const MENU_BUTTON_SINCE: u32 = 4;
pub(crate) const SHOW_APP_MENU_SINCE: u32 = 3;
pub(crate) const ACTIVE_MENUS_SINCE: u32 = 4;
const APPMENU_VERSION: u32 = 2;
const GTK_SHELL_VERSION: u32 = 1;

#[derive(Default)]
struct MenuAddress(Mutex<Option<String>>);

pub(crate) fn menu_address(surface: &WlSurface) -> Option<String> {
    with_states(surface, |states| {
        states.data_map.get::<MenuAddress>().and_then(|address| {
            address
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        })
    })
}

fn set_menu_address(surface: &WlSurface, address: Option<String>) {
    with_states(surface, |states| {
        states
            .data_map
            .insert_if_missing_threadsafe(MenuAddress::default);
        if let Some(stored) = states.data_map.get::<MenuAddress>() {
            *stored.0.lock().unwrap_or_else(PoisonError::into_inner) = address;
        }
    });
}

pub(crate) fn create_globals(handle: &DisplayHandle) {
    handle.create_global::<State, OrgKdeKwinAppmenuManager, ()>(APPMENU_VERSION, ());
    handle.create_global::<State, GtkShell1, ()>(GTK_SHELL_VERSION, ());
}

fn gtk_menu_address(
    menubar_path: Option<String>,
    application_object_path: Option<String>,
    window_object_path: Option<String>,
    unique_bus_name: Option<String>,
) -> Option<String> {
    let bus = unique_bus_name.filter(|bus| bus.starts_with(':'))?;
    let menubar = menubar_path.filter(|path| path.starts_with('/'))?;
    let object = |path: Option<String>| path.filter(|path| path.starts_with('/'));
    let application = object(application_object_path).unwrap_or_default();
    let window = object(window_object_path).unwrap_or_default();
    Some(format!("gtk {bus} {menubar} {application} {window}"))
}

pub(crate) struct MenuRelay {
    window: WlSurface,
    serial: Serial,
    popup: Option<WlSurface>,
}

impl State {
    pub(crate) fn menu_button(&self, index: usize) -> Option<Rectangle<i32, Logical>> {
        let window = &self.windows[index];
        let bar = window.title_bar_height();
        if bar > 0 {
            return menu_address(window.surface()).map(|_| {
                let area = menu_button_area(bar as f32);
                Rectangle::new(
                    (area.x.round() as i32, area.y.round() as i32).into(),
                    (area.width.round() as i32, area.height.round() as i32).into(),
                )
            });
        }
        window.menu_button
    }

    fn shell_bar(&self, screen: usize) -> Option<WlSurface> {
        let map = layer_map_for_output(&self.screens[screen].output);
        [BAR_NAMESPACE, MENU_BAR_NAMESPACE]
            .into_iter()
            .find_map(|namespace| {
                map.layers()
                    .find(|layer| layer.namespace() == namespace)
                    .map(|layer| layer.wl_surface().clone())
            })
    }

    fn shells_of(&self, surface: &WlSurface) -> Vec<BandeShellV1> {
        self.shell_resources
            .iter()
            .filter(|shell| {
                shell.client() == surface.client() && shell.version() >= SHOW_APP_MENU_SINCE
            })
            .cloned()
            .collect()
    }

    pub(crate) fn show_menus_of(&mut self, surface: &WlSurface) -> bool {
        let Some(index) = self.window_index(surface) else {
            return false;
        };
        let Some(button) = self.menu_button(index) else {
            return false;
        };
        let Some(geometry) = self.windows[index].geometry() else {
            return false;
        };
        let screen = self.screen_of(&self.windows[index]);
        let Some(bar) = self.shell_bar(screen) else {
            return false;
        };
        let Some(origin) = self.layer_origin(&bar) else {
            return false;
        };
        let shells = self.shells_of(&bar);
        if shells.is_empty() {
            return false;
        }
        let at = geometry.loc + button.loc - origin;
        let (_, app_id) = toplevel_names(surface);
        let menu = menu_address(surface).unwrap_or_default();
        let serial = SERIAL_COUNTER.next_serial();
        for shell in shells {
            shell.show_app_menu(
                &bar,
                app_id.clone(),
                menu.clone(),
                at.x,
                at.y,
                button.size.w,
                button.size.h,
                serial.into(),
            );
        }
        self.menu_relay = Some(MenuRelay {
            window: surface.clone(),
            serial,
            popup: None,
        });
        true
    }

    pub(crate) fn relays_menus(&self, serial: Serial) -> bool {
        self.menu_relay
            .as_ref()
            .is_some_and(|relay| relay.serial == serial && relay.popup.is_none())
    }

    pub(crate) fn menus_opened(&mut self, popup: &WlSurface) {
        let Some(relay) = &mut self.menu_relay else {
            return;
        };
        relay.popup = Some(popup.clone());
        let window = relay.window.clone();
        self.relayed_menu_owner = Some(window.clone());
        self.set_menus_open(&window, true);
    }

    pub(crate) fn popup_gone(&mut self, popup: &WlSurface) {
        if self
            .menu_relay
            .as_ref()
            .is_some_and(|relay| relay.popup.as_ref() == Some(popup))
            && let Some(relay) = self.menu_relay.take()
        {
            self.set_menus_open(&relay.window, false);
        }
    }

    pub(crate) fn set_menus_open(&mut self, surface: &WlSurface, open: bool) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let window = &mut self.windows[index];
        if window.frame.menus_open != open {
            window.frame.menus_open = open;
            self.needs_redraw = true;
        }
        if window.title_bar_height() > 0 {
            return;
        }
        let toplevel = window.toplevel.xdg_toplevel().clone();
        for app in self
            .app_resources
            .iter()
            .filter(|app| app.client() == toplevel.client() && app.version() >= MENU_BUTTON_SINCE)
        {
            app.menus_open(&toplevel, u32::from(open));
        }
    }

    fn menu_address_changed(&mut self, surface: &WlSurface, address: Option<String>) {
        set_menu_address(surface, address);
        self.needs_redraw = true;
        let key = self.key_window();
        self.announce_active_app(key.as_ref());
    }

    pub(crate) fn request_menus(&mut self, surface: &WlSurface) {
        let key = self.key_window();
        if key.as_ref() != Some(surface) || !self.show_menus_of(surface) {
            self.set_menus_open(surface, false);
        }
    }
}

impl GlobalDispatch<OrgKdeKwinAppmenuManager, ()> for State {
    fn bind(
        _state: &mut Self,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<OrgKdeKwinAppmenuManager>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<OrgKdeKwinAppmenuManager, ()> for State {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _manager: &OrgKdeKwinAppmenuManager,
        request: org_kde_kwin_appmenu_manager::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let org_kde_kwin_appmenu_manager::Request::Create { id, surface } = request {
            data_init.init(id, surface);
        }
    }
}

impl Dispatch<OrgKdeKwinAppmenu, WlSurface> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        _menu: &OrgKdeKwinAppmenu,
        request: org_kde_kwin_appmenu::Request,
        surface: &WlSurface,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        if !surface.is_alive() {
            return;
        }
        let address = match request {
            org_kde_kwin_appmenu::Request::SetAddress {
                service_name,
                object_path,
            } if !service_name.is_empty() && object_path.starts_with('/') => {
                Some(format!("{service_name}{object_path}"))
            }
            _ => None,
        };
        state.menu_address_changed(surface, address);
    }
}

impl GlobalDispatch<GtkShell1, ()> for State {
    fn bind(
        _state: &mut Self,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<GtkShell1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let shell = data_init.init(resource, ());
        shell.capabilities(gtk_shell1::Capability::GlobalMenuBar as u32);
    }
}

impl Dispatch<GtkShell1, ()> for State {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _shell: &GtkShell1,
        request: gtk_shell1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let gtk_shell1::Request::GetGtkSurface {
            gtk_surface,
            surface,
        } = request
        {
            data_init.init(gtk_surface, surface);
        }
    }
}

impl Dispatch<GtkSurface1, WlSurface> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        _gtk_surface: &GtkSurface1,
        request: gtk_surface1::Request,
        surface: &WlSurface,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        if !surface.is_alive() {
            return;
        }
        if let gtk_surface1::Request::SetDbusProperties {
            menubar_path,
            window_object_path,
            application_object_path,
            unique_bus_name,
            ..
        } = request
            && let Some(address) = gtk_menu_address(
                menubar_path,
                application_object_path,
                window_object_path,
                unique_bus_name,
            )
        {
            state.menu_address_changed(surface, Some(address));
        }
    }
}
