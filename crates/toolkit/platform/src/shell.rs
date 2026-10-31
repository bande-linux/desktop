use std::collections::HashMap;

use unbound_protocols::client::unbound_app_control_v1::{self, UnboundAppControlV1};
use unbound_protocols::client::unbound_app_v1::{self, Placement, UnboundAppV1};
use unbound_protocols::client::unbound_shell_v1::{self, SessionEnd, UnboundShellV1};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, event_created_child};
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
    identifier: String,
    title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppWindow {
    pub identifier: String,
    pub app_id: String,
    pub title: String,
    pub minimized: bool,
}

#[derive(Default)]
pub(crate) struct Shell {
    pub(crate) protocol: Option<UnboundShellV1>,
    pub(crate) app: Option<UnboundAppV1>,
    pub(crate) app_control_global: Option<(WlRegistry, u32)>,
    app_control: Option<UnboundAppControlV1>,
    toplevels: Vec<Toplevel>,
    processes: HashMap<String, u32>,
    minimized: Vec<String>,
    token: Option<String>,
    displays: Vec<String>,
}

impl Window {
    fn shell(&self) -> Result<&UnboundShellV1, WindowError> {
        self.state
            .shell
            .protocol
            .as_ref()
            .ok_or(WindowError::Missing("unbound_shell_v1"))
    }

    fn app(&self) -> Result<&UnboundAppV1, WindowError> {
        self.state
            .shell
            .app
            .as_ref()
            .ok_or(WindowError::Missing("unbound_app_v1"))
    }

    pub fn displays(&self) -> &[String] {
        &self.state.shell.displays
    }

    pub fn app_windows(&self, app_id: &str) -> Vec<AppWindow> {
        let shell = &self.state.shell;
        shell
            .toplevels
            .iter()
            .filter(|toplevel| toplevel.app_id == app_id)
            .map(|toplevel| AppWindow {
                identifier: toplevel.identifier.clone(),
                app_id: toplevel.app_id.clone(),
                title: toplevel.title.clone(),
                minimized: shell.minimized.contains(&toplevel.identifier),
            })
            .collect()
    }

    pub fn hide_app(&mut self, app_id: &str) -> Result<(), WindowError> {
        self.shell()?.hide_app(app_id.to_owned());
        self.flush()
    }

    pub fn show_app(&mut self, app_id: &str) -> Result<(), WindowError> {
        self.shell()?.show_app(app_id.to_owned());
        self.flush()
    }

    pub fn quit_app(&mut self, app_id: &str) -> Result<(), WindowError> {
        if let Ok(shell) = self.shell() {
            shell.quit_app(app_id.to_owned());
        } else {
            self.app_control()?.quit_app(app_id.to_owned());
        }
        self.flush()
    }

    pub fn watch_apps(&mut self) -> Result<(), WindowError> {
        if self.shell().is_ok() {
            return Ok(());
        }
        self.app_control()?;
        self.flush()
    }

    fn app_control(&mut self) -> Result<&UnboundAppControlV1, WindowError> {
        let shell = &mut self.state.shell;
        if shell.app_control.is_none() {
            let (registry, name) = shell
                .app_control_global
                .as_ref()
                .ok_or(WindowError::Missing("unbound_app_control_v1"))?;
            shell.app_control = Some(registry.bind(*name, 1, &self.queue.handle(), ()));
        }
        self.state
            .shell
            .app_control
            .as_ref()
            .ok_or(WindowError::Missing("unbound_app_control_v1"))
    }

    pub fn force_quit_app(&mut self, app_id: &str) -> Result<(), WindowError> {
        self.shell()?.force_quit_app(app_id.to_owned());
        self.flush()
    }

    pub fn activate_window(&mut self, identifier: &str) -> Result<(), WindowError> {
        self.shell()?.activate_window(identifier.to_owned());
        self.flush()
    }

    pub fn confirm_session_end(&mut self, ending: SessionEnd) -> Result<(), WindowError> {
        self.shell()?.confirm_session_end(ending);
        self.flush()
    }

    pub fn quit_all_apps(&mut self) -> Result<(), WindowError> {
        self.shell()?.quit_all_apps();
        self.flush()
    }

    pub fn show_launcher(&mut self) -> Result<(), WindowError> {
        self.shell()?.show_launcher();
        self.flush()
    }

    pub fn show_windows(&mut self, app_id: &str) -> Result<(), WindowError> {
        self.shell()?.show_windows(app_id.to_owned());
        self.flush()
    }

    pub fn select_input_source(&mut self, index: usize) -> Result<(), WindowError> {
        self.shell()?
            .select_input_source(u32::try_from(index).unwrap_or(u32::MAX));
        self.flush()
    }

    pub fn switch_workspace(&mut self, index: usize) -> Result<(), WindowError> {
        self.shell()?
            .switch_workspace(u32::try_from(index).unwrap_or(u32::MAX));
        self.flush()
    }

    pub fn set_panel_anchor(
        &mut self,
        panel: unbound_shell_v1::Panel,
        edge: unbound_shell_v1::Edge,
        rect: Rect,
    ) -> Result<(), WindowError> {
        self.shell()?.set_panel_anchor(
            &self.surface,
            panel,
            edge,
            rect.x.round() as i32,
            rect.y.round() as i32,
            rect.width.round() as i32,
            rect.height.round() as i32,
        );
        self.flush()
    }

    pub fn insert_text(&mut self, text: &str) -> Result<(), WindowError> {
        self.shell()?.insert_text(text.to_owned());
        self.flush()
    }

    pub fn show_emoji_picker(&mut self) -> Result<(), WindowError> {
        self.app()?.show_emoji_picker();
        self.flush()
    }

    pub fn set_scrim(&mut self, opacity: f32) -> Result<(), WindowError> {
        self.shell()?.set_scrim(&self.surface, f64::from(opacity));
        Ok(())
    }

    pub fn set_corner_radius(&mut self, radius: u32) -> Result<(), WindowError> {
        self.shell()?.set_corner_radius(&self.surface, radius);
        self.flush()
    }
    pub fn running_apps(&self) -> Vec<String> {
        let mut apps = Vec::new();
        for toplevel in &self.state.shell.toplevels {
            if !toplevel.app_id.is_empty() && !apps.contains(&toplevel.app_id) {
                apps.push(toplevel.app_id.clone());
            }
        }
        apps
    }

    pub fn app_pid(&self, app_id: &str) -> Option<u32> {
        self.state.shell.processes.get(app_id).copied()
    }

    pub fn activate_app(&mut self, app_id: &str) -> Result<(), WindowError> {
        self.shell()?.activate_app(app_id.to_owned());
        self.flush()
    }

    pub fn toggle_control_center(&mut self) -> Result<(), WindowError> {
        self.shell()?.toggle_control_center();
        self.flush()
    }

    pub fn set_icon_rect(&mut self, app_id: &str, rect: Rect) -> Result<(), WindowError> {
        self.shell()?.set_icon_rect(
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

impl Window {
    pub(crate) fn can_place(&self) -> bool {
        self.app().is_ok()
    }

    pub(crate) fn place(&mut self, placement: Placement) -> Result<(), WindowError> {
        self.app()?.place_window(self.toplevel()?, placement);
        self.flush()
    }

    pub(crate) fn can_keep_on_top(&self) -> bool {
        self.app().is_ok()
    }

    pub fn set_keep_on_top(&mut self, keep: bool) -> Result<(), WindowError> {
        self.app()?
            .set_keep_on_top(self.toplevel()?, u32::from(keep));
        self.flush()
    }
}

impl Dispatch<UnboundAppControlV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &UnboundAppControlV1,
        event: unbound_app_control_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let unbound_app_control_v1::Event::AppResponding {
            pid,
            app_id,
            responding,
        } = event
        {
            state.events.push(WindowEvent::AppResponding {
                pid,
                app_id,
                responding: responding != 0,
            });
        }
    }
}

impl Dispatch<UnboundAppV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &UnboundAppV1,
        event: unbound_app_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let unbound_app_v1::Event::QuitRequested = event {
            state.events.push(WindowEvent::QuitRequested);
        }
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
            unbound_shell_v1::Event::ScrimPressed { surface } => {
                if state.own_surface.as_ref() == Some(&surface) {
                    state.events.push(WindowEvent::ScrimPressed);
                }
            }
            unbound_shell_v1::Event::OpenLauncher {
                mode: WEnum::Value(mode),
                text_field,
            } => state.events.push(WindowEvent::OpenLauncher {
                mode,
                text_field: text_field != 0,
            }),
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
            unbound_shell_v1::Event::BrightnessKey {
                key: WEnum::Value(key),
                fine,
            } => state.events.push(WindowEvent::BrightnessKey {
                key,
                fine: fine != 0,
            }),
            unbound_shell_v1::Event::MediaKey {
                key: WEnum::Value(key),
            } => state.events.push(WindowEvent::MediaKey(key)),
            unbound_shell_v1::Event::TogglePanel {
                panel: WEnum::Value(panel),
            } => state.events.push(WindowEvent::TogglePanel(panel)),
            unbound_shell_v1::Event::AppResponding {
                pid,
                app_id,
                responding,
            } => state.events.push(WindowEvent::AppResponding {
                pid,
                app_id,
                responding: responding != 0,
            }),
            unbound_shell_v1::Event::WindowMinimized {
                identifier,
                minimized,
            } => {
                let known = &mut state.shell.minimized;
                known.retain(|known| *known != identifier);
                if minimized != 0 {
                    known.push(identifier);
                }
                state.events.push(WindowEvent::AppsChanged);
            }
            unbound_shell_v1::Event::AttentionRequested { app_id } => {
                state.events.push(WindowEvent::AttentionRequested(app_id));
            }
            unbound_shell_v1::Event::FocusShell { namespace } => {
                state.events.push(WindowEvent::FocusShell(namespace));
            }
            unbound_shell_v1::Event::ActiveOutput { name } => {
                state.events.push(WindowEvent::ActiveOutput(name));
            }
            unbound_shell_v1::Event::Workspaces { count, active } => {
                state.events.push(WindowEvent::Workspaces {
                    count: count as usize,
                    active: active as usize,
                });
            }
            unbound_shell_v1::Event::PanelAnchor {
                panel: WEnum::Value(panel),
                output,
                edge: WEnum::Value(edge),
                x,
                y,
                width,
                height,
            } => state.events.push(WindowEvent::PanelAnchor {
                panel,
                output,
                edge,
                rect: Rect::new(x as f32, y as f32, width as f32, height as f32),
            }),
            unbound_shell_v1::Event::FullScreen {
                output,
                full_screen,
            } => state.events.push(WindowEvent::FullScreen {
                output,
                full_screen: full_screen != 0,
            }),
            unbound_shell_v1::Event::InputSources { sources, active } => {
                state.events.push(WindowEvent::InputSources {
                    sources: sources
                        .split(',')
                        .filter(|source| !source.is_empty())
                        .map(str::to_owned)
                        .collect(),
                    active: active as usize,
                });
            }
            unbound_shell_v1::Event::Displays { names } => {
                state.shell.displays = names
                    .split(',')
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned)
                    .collect();
                state
                    .events
                    .push(WindowEvent::Displays(state.shell.displays.clone()));
            }
            unbound_shell_v1::Event::AppsRunning { app_id, count } => {
                state
                    .events
                    .push(WindowEvent::AppsRunning { app_id, count });
            }
            unbound_shell_v1::Event::AppProcess { app_id, pid } => {
                let Ok(pid) = u32::try_from(pid) else {
                    return;
                };
                if !app_id.is_empty() && state.shell.processes.insert(app_id, pid) != Some(pid) {
                    state.events.push(WindowEvent::AppsChanged);
                }
            }
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
                identifier: String::new(),
                title: String::new(),
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
            ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } => {
                toplevels[index].identifier = identifier;
            }
            ext_foreign_toplevel_handle_v1::Event::Title { title } => {
                toplevels[index].title = title;
            }
            ext_foreign_toplevel_handle_v1::Event::Done => {
                let toplevel = &mut toplevels[index];
                if toplevel.app_id != toplevel.pending_app_id {
                    toplevel.app_id = toplevel.pending_app_id.clone();
                    state.events.push(WindowEvent::AppsChanged);
                }
            }
            ext_foreign_toplevel_handle_v1::Event::Closed => {
                let closed = toplevels.remove(index);
                state
                    .shell
                    .minimized
                    .retain(|identifier| *identifier != closed.identifier);
                closed.handle.destroy();
                let shell = &mut state.shell;
                shell.processes.retain(|app_id, _| {
                    shell
                        .toplevels
                        .iter()
                        .any(|toplevel| toplevel.app_id == *app_id)
                });

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
