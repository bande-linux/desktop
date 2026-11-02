use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_data_device_manager::WlDataDeviceManager;
use wayland_client::protocol::wl_region::WlRegion;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_shm::WlShm;
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1;
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;
use wayland_protocols::xdg::shell::client::xdg_surface::{self, XdgSurface};
use wayland_protocols::xdg::shell::client::xdg_toplevel::{self, XdgToplevel};
use wayland_protocols::xdg::shell::client::xdg_wm_base::{self, XdgWmBase};
use wayland_protocols::xdg::toplevel_tag::v1::client::xdg_toplevel_tag_manager_v1::XdgToplevelTagManagerV1;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::ZwlrLayerShellV1;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    self, ZwlrLayerSurfaceV1,
};

use super::{State, ToplevelState, WindowEvent};
use crate::shell::{ACTIVE_MENUS_SINCE, MENU_BUTTON_SINCE};

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::GlobalRemove { name } = event {
            state.outputs.retain(|output| *output != name);
            return;
        }
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, version.min(5), handle, ()));
                }
                "wl_shm" => state.shm = Some(registry.bind(name, 1, handle, ())),
                "xdg_wm_base" => {
                    state.wm_base = Some(registry.bind(name, version.min(6), handle, ()));
                }
                "zwlr_layer_shell_v1" => {
                    state.layer_shell = Some(registry.bind(name, version.min(5), handle, ()));
                }
                "xdg_activation_v1" => {
                    state.activation = Some(registry.bind(name, 1, handle, ()));
                }
                "xdg_toplevel_tag_manager_v1" => {
                    state.toplevel_tag = Some(registry.bind(name, 1, handle, ()));
                }
                "xdg_toplevel_drag_manager_v1" => {
                    state.toplevel_drag = Some(registry.bind(name, 1, handle, ()));
                }
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind(name, version.min(9), handle, ()));
                }
                "wl_data_device_manager" => {
                    state.data_device_manager =
                        Some(registry.bind(name, version.min(3), handle, ()));
                }
                "zwp_primary_selection_device_manager_v1" => {
                    state.clipboard.primary.manager = Some(registry.bind(name, 1, handle, ()));
                }
                "wp_cursor_shape_manager_v1" => {
                    state.cursor_shape_manager = Some(registry.bind(name, 1, handle, ()));
                }
                "ext_session_lock_manager_v1" => {
                    state.lock.manager = Some(registry.bind(name, 1, handle, ()));
                }
                "wl_output" => {
                    state.outputs.push(name);
                    if state.lock.output.is_none() {
                        state.lock.output = Some(registry.bind(name, 1, handle, ()));
                    }
                    if version >= 4 && state.lock.wanted_name.is_some() {
                        let _: wayland_client::protocol::wl_output::WlOutput =
                            registry.bind(name, 4, handle, crate::lock::NamedOutput);
                    }
                }
                "bande_shell_v1" => {
                    state.shell.protocol =
                        Some(registry.bind(name, version.min(ACTIVE_MENUS_SINCE), handle, ()));
                }
                "bande_app_v1" => {
                    state.shell.app =
                        Some(registry.bind(name, version.min(MENU_BUTTON_SINCE), handle, ()));
                }
                "bande_app_control_v1" => {
                    state.shell.app_control_global = Some((registry.clone(), name));
                }
                "zwp_text_input_manager_v3" => {
                    state.text_input.manager = Some(registry.bind(name, 1, handle, ()));
                }
                "wp_fractional_scale_manager_v1" => {
                    state.scaling.fractional_manager = Some(registry.bind(name, 1, handle, ()));
                }
                "wp_presentation" => {
                    state.presentation.manager = Some(registry.bind(name, 1, handle, ()));
                }
                "wp_viewporter" => {
                    state.scaling.viewporter = Some(registry.bind(name, 1, handle, ()));
                }
                "ext_foreign_toplevel_list_v1" => {
                    let _: ExtForeignToplevelListV1 = registry.bind(name, 1, handle, ());
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<XdgWmBase, ()> for State {
    fn event(
        _: &mut Self,
        wm_base: &XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, ()> for State {
    fn event(
        state: &mut Self,
        xdg_surface: &XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            state.configured = true;
            state.configure_unapplied = true;
        }
    }
}

impl Dispatch<XdgToplevel, ()> for State {
    fn event(
        state: &mut Self,
        _: &XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure {
                width,
                height,
                states,
            } => {
                if width > 0 && height > 0 {
                    state.suggested_size = Some((width.unsigned_abs(), height.unsigned_abs()));
                }
                let states = states
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter_map(|bytes| {
                        xdg_toplevel::State::try_from(u32::from_ne_bytes(*bytes)).ok()
                    })
                    .collect::<Vec<_>>();
                let previous = state.toplevel;
                state.toplevel = ToplevelState {
                    maximized: states.contains(&xdg_toplevel::State::Maximized),
                    fullscreen: states.contains(&xdg_toplevel::State::Fullscreen),
                    activated: states.contains(&xdg_toplevel::State::Activated),
                    suspended: states.contains(&xdg_toplevel::State::Suspended),
                    tiled: states.iter().any(|state| {
                        matches!(
                            state,
                            xdg_toplevel::State::TiledLeft
                                | xdg_toplevel::State::TiledRight
                                | xdg_toplevel::State::TiledTop
                                | xdg_toplevel::State::TiledBottom
                        )
                    }),
                };
                if state.toplevel != previous {
                    state
                        .events
                        .push(WindowEvent::ToplevelChanged(state.toplevel));
                }
            }
            xdg_toplevel::Event::Close => state.events.push(WindowEvent::CloseRequested),
            _ => {}
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        layer_surface: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer_surface.ack_configure(serial);
                state.configure(width, height);
            }
            zwlr_layer_surface_v1::Event::Closed => state.closed = true,
            _ => {}
        }
    }
}

impl Dispatch<WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.frame_pending = false;
        }
    }
}

impl Dispatch<WlBuffer, Arc<AtomicBool>> for State {
    fn event(
        _: &mut Self,
        _: &WlBuffer,
        event: wl_buffer::Event,
        busy: &Arc<AtomicBool>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            busy.store(false, Ordering::Release);
        }
    }
}

delegate_noop!(State: ignore WlCompositor);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: ignore WlShmPool);
delegate_noop!(State: ignore WlRegion);
delegate_noop!(State: ignore WlSurface);
delegate_noop!(State: ignore ZwlrLayerShellV1);
delegate_noop!(State: ignore XdgActivationV1);
delegate_noop!(State: ignore XdgToplevelTagManagerV1);
delegate_noop!(State: ignore wayland_protocols::xdg::toplevel_drag::v1::client::xdg_toplevel_drag_manager_v1::XdgToplevelDragManagerV1);
delegate_noop!(State: ignore wayland_protocols::xdg::toplevel_drag::v1::client::xdg_toplevel_drag_v1::XdgToplevelDragV1);
delegate_noop!(State: ignore WlDataDeviceManager);
