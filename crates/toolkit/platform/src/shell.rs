use unbound_protocols::client::unbound_shell_v1::{self, UnboundShellV1};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, event_created_child};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::{
    self, ExtForeignToplevelHandleV1,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::{
    self, ExtForeignToplevelListV1,
};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_token_v1::{
    self, XdgActivationTokenV1,
};

use unbound_graphics::Rect;

use crate::window::{State, Window, WindowError, WindowEvent};

struct Toplevel {
    handle: ExtForeignToplevelHandleV1,
    app_id: String,
    pending_app_id: String,
}

#[derive(Default)]
pub(crate) struct Shell {
    pub(crate) protocol: Option<UnboundShellV1>,
    toplevels: Vec<Toplevel>,
    token: Option<String>,
}

impl Window {
    pub fn running_apps(&self) -> Vec<String> {
        let mut apps = Vec::new();
        for toplevel in &self.state.shell.toplevels {
            if !toplevel.app_id.is_empty() && !apps.contains(&toplevel.app_id) {
                apps.push(toplevel.app_id.clone());
            }
        }
        apps
    }

    pub fn activate_app(&mut self, app_id: &str) -> Result<(), WindowError> {
        let protocol = self
            .state
            .shell
            .protocol
            .as_ref()
            .ok_or(WindowError::Missing("unbound_shell_v1"))?;
        protocol.activate_app(app_id.to_owned());
        self.flush()
    }

    pub fn toggle_control_center(&mut self) -> Result<(), WindowError> {
        let protocol = self
            .state
            .shell
            .protocol
            .as_ref()
            .filter(|protocol| {
                protocol.version() >= unbound_shell_v1::REQ_TOGGLE_CONTROL_CENTER_SINCE
            })
            .ok_or(WindowError::Missing("unbound_shell_v1 version 4"))?;
        protocol.toggle_control_center();
        self.flush()
    }

    pub fn set_icon_rect(&mut self, app_id: &str, rect: Rect) -> Result<(), WindowError> {
        let protocol = self
            .state
            .shell
            .protocol
            .as_ref()
            .filter(|protocol| protocol.version() >= unbound_shell_v1::REQ_SET_ICON_RECT_SINCE)
            .ok_or(WindowError::Missing("unbound_shell_v1 version 5"))?;
        protocol.set_icon_rect(
            app_id.to_owned(),
            &self.surface,
            rect.x.round() as i32,
            rect.y.round() as i32,
            rect.width.round() as i32,
            rect.height.round() as i32,
        );
        Ok(())
    }

    pub fn activation_token(&mut self, app_id: &str) -> Result<String, WindowError> {
        let activation = self
            .state
            .activation
            .as_ref()
            .ok_or(WindowError::Missing("xdg_activation_v1"))?;
        let token = activation.get_activation_token(&self.queue.handle(), ());
        if let (Some(serial), Some(seat)) = (self.state.input.serial, &self.state.seat) {
            token.set_serial(serial, seat);
        }
        token.set_app_id(app_id.to_owned());
        token.set_surface(&self.surface);
        token.commit();
        self.state.shell.token = None;
        while self.state.shell.token.is_none() {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        token.destroy();
        self.state
            .shell
            .token
            .take()
            .ok_or(WindowError::Missing("an activation token"))
    }
}

impl Dispatch<UnboundShellV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &UnboundShellV1,
        event: unbound_shell_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            unbound_shell_v1::Event::ShowLauncher => state.events.push(WindowEvent::ShowLauncher),
            unbound_shell_v1::Event::CaptureScreen => state.events.push(WindowEvent::CaptureScreen),
            unbound_shell_v1::Event::CaptureArea => state.events.push(WindowEvent::CaptureArea),
            unbound_shell_v1::Event::ActiveApp { app_id } => {
                state.events.push(WindowEvent::ActiveApp(app_id));
            }
            unbound_shell_v1::Event::VolumeKey {
                key: WEnum::Value(key),
                fine,
            } => state.events.push(WindowEvent::VolumeKey {
                key,
                fine: fine != 0,
            }),
            unbound_shell_v1::Event::ToggleControlCenter => {
                state.events.push(WindowEvent::ToggleControlCenter);
            }
            unbound_shell_v1::Event::AppResponding {
                pid,
                app_id,
                responding,
            } => state.events.push(WindowEvent::AppResponding {
                pid,
                app_id,
                responding: responding != 0,
            }),
            _ => {}
        }
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            state.shell.toplevels.push(Toplevel {
                handle: toplevel,
                app_id: String::new(),
                pending_app_id: String::new(),
            });
        }
    }

    event_created_child!(State, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let toplevels = &mut state.shell.toplevels;
        let Some(index) = toplevels
            .iter()
            .position(|toplevel| &toplevel.handle == handle)
        else {
            return;
        };
        match event {
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                toplevels[index].pending_app_id = app_id;
            }
            ext_foreign_toplevel_handle_v1::Event::Done => {
                let toplevel = &mut toplevels[index];
                if toplevel.app_id != toplevel.pending_app_id {
                    toplevel.app_id = toplevel.pending_app_id.clone();
                    state.events.push(WindowEvent::AppsChanged);
                }
            }
            ext_foreign_toplevel_handle_v1::Event::Closed => {
                toplevels.remove(index).handle.destroy();
                state.events.push(WindowEvent::AppsChanged);
            }
            _ => {}
        }
    }
}

impl Dispatch<XdgActivationTokenV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token } = event {
            state.shell.token = Some(token);
        }
    }
}
