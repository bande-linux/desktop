use wayland_client::protocol::wl_shm::Format;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::xdg::shell::client::xdg_popup::{self, XdgPopup};
use wayland_protocols::xdg::shell::client::xdg_positioner::{
    Anchor, ConstraintAdjustment, Gravity, XdgPositioner,
};
use wayland_protocols::xdg::shell::client::xdg_surface::XdgSurface;
use wayland_protocols::xdg::shell::client::xdg_toplevel::{ResizeEdge, State, XdgToplevel};

use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1;

use crate::{Globals, Headless, ShmImage, TestClient};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Configured {
    pub width: i32,
    pub height: i32,
    pub states: Vec<State>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopupRequest {
    pub parent: usize,
    pub anchor_rect: (i32, i32, i32, i32),
    pub size: (i32, i32),
    pub flip_x: bool,
    pub grab: bool,
    pub color: [u8; 4],
}

struct Popup {
    surface: WlSurface,
    popup: XdgPopup,
    geometry: Option<(i32, i32, i32, i32)>,
    done: bool,
}

#[derive(Default)]
pub(crate) struct Toplevels {
    configured: Vec<(XdgToplevel, Configured)>,
    popups: Vec<Popup>,
}

impl Toplevels {
    pub(crate) fn configured(
        &mut self,
        toplevel: &XdgToplevel,
        width: i32,
        height: i32,
        states: &[u8],
    ) {
        let states = states
            .as_chunks::<4>()
            .0
            .iter()
            .filter_map(|bytes| State::try_from(u32::from_ne_bytes(*bytes)).ok())
            .collect();
        let configured = Configured {
            width,
            height,
            states,
        };
        match self
            .configured
            .iter_mut()
            .find(|(known, _)| known == toplevel)
        {
            Some((_, last)) => *last = configured,
            None => self.configured.push((toplevel.clone(), configured)),
        }
    }
}

impl TestClient {
    fn toplevel(&self, index: usize) -> &XdgToplevel {
        &self.windows[index].2
    }

    pub fn configured(&self, index: usize) -> Option<Configured> {
        let toplevel = self.toplevel(index);
        self.globals
            .toplevels
            .configured
            .iter()
            .find(|(known, _)| known == toplevel)
            .map(|(_, configured)| configured.clone())
    }

    pub fn fill_configured_size(&mut self, server: &mut Headless, index: usize, color: [u8; 4]) {
        let configured = self.configured(index).expect("a configure for the window");
        let (width, height) = (configured.width, configured.height);
        assert!(width > 0 && height > 0, "the compositor chose a size");
        let surface = self.windows[index].0.clone();
        self.attach_solid(&surface, width, height, color);
        self.exchange(server);
    }

    fn attach_solid(&self, surface: &WlSurface, width: i32, height: i32, color: [u8; 4]) {
        let pixels = color.repeat(usize::try_from(width * height).expect("buffer size"));
        let buffer = self.shm_buffer(&ShmImage {
            width,
            height,
            stride: width * 4,
            format: Format::Argb8888,
            bytes: &pixels,
        });
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, width, height);
        surface.commit();
    }

    fn seat_and_serial(&self) -> (&wayland_client::protocol::wl_seat::WlSeat, u32) {
        let input = &self.globals.input;
        (
            input.seat.as_ref().expect("use_seat first"),
            input.button_serial,
        )
    }

    pub fn request_move(&mut self, server: &mut Headless, index: usize) {
        let (seat, serial) = self.seat_and_serial();
        self.toplevel(index)._move(seat, serial);
        self.exchange(server);
    }

    pub fn request_resize(&mut self, server: &mut Headless, index: usize, edges: ResizeEdge) {
        let (seat, serial) = self.seat_and_serial();
        self.toplevel(index).resize(seat, serial, edges);
        self.exchange(server);
    }

    pub fn set_min_size(&mut self, server: &mut Headless, index: usize, width: i32, height: i32) {
        self.toplevel(index).set_min_size(width, height);
        self.windows[index].0.commit();
        self.exchange(server);
    }

    pub fn set_maximized(&mut self, server: &mut Headless, index: usize, maximized: bool) {
        let toplevel = self.toplevel(index);
        if maximized {
            toplevel.set_maximized();
        } else {
            toplevel.unset_maximized();
        }
        self.exchange(server);
    }

    pub fn set_fullscreen(&mut self, server: &mut Headless, index: usize, fullscreen: bool) {
        let toplevel = self.toplevel(index);
        if fullscreen {
            toplevel.set_fullscreen(None);
        } else {
            toplevel.unset_fullscreen();
        }
        self.exchange(server);
    }

    pub fn set_minimized(&mut self, server: &mut Headless, index: usize) {
        self.toplevel(index).set_minimized();
        self.exchange(server);
    }

    pub fn open_popup(&mut self, server: &mut Headless, request: &PopupRequest) -> usize {
        let parent = self.windows[request.parent].1.clone();
        self.open_popup_on(server, request, Some(&parent), None)
    }

    pub fn open_layer_popup(&mut self, server: &mut Headless, request: &PopupRequest) -> usize {
        let layer = self.globals.shell.layers[request.parent].1.clone();
        self.open_popup_on(server, request, None, Some(&layer))
    }

    fn open_popup_on(
        &mut self,
        server: &mut Headless,
        request: &PopupRequest,
        parent: Option<&XdgSurface>,
        layer: Option<&ZwlrLayerSurfaceV1>,
    ) -> usize {
        let qh = self.queue.handle();
        let compositor = self.globals.compositor.as_ref().expect("wl_compositor");
        let wm_base = self.globals.wm_base.as_ref().expect("xdg_wm_base");
        let surface = compositor.create_surface(&qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
        let positioner = wm_base.create_positioner(&qh, ());
        let (x, y, width, height) = request.anchor_rect;
        positioner.set_anchor_rect(x, y, width, height);
        positioner.set_size(request.size.0, request.size.1);
        positioner.set_anchor(Anchor::BottomRight);
        positioner.set_gravity(Gravity::BottomRight);
        if request.flip_x {
            positioner.set_constraint_adjustment(ConstraintAdjustment::FlipX);
        }
        let popup = xdg_surface.get_popup(parent, &positioner, &qh, ());
        if let Some(layer) = layer {
            layer.get_popup(&popup);
        }
        positioner.destroy();
        if request.grab {
            let (seat, serial) = self.seat_and_serial();
            popup.grab(seat, serial);
        }
        surface.commit();
        self.globals.toplevels.popups.push(Popup {
            surface: surface.clone(),
            popup,
            geometry: None,
            done: false,
        });
        self.exchange(server);
        self.attach_solid(&surface, request.size.0, request.size.1, request.color);
        self.exchange(server);
        self.globals.toplevels.popups.len() - 1
    }

    pub fn destroy_popup(&mut self, server: &mut Headless, index: usize) {
        let popup = &self.globals.toplevels.popups[index];
        popup.popup.destroy();
        popup.surface.destroy();
        self.exchange(server);
    }

    pub fn popup_geometry(&self, index: usize) -> Option<(i32, i32, i32, i32)> {
        self.globals.toplevels.popups[index].geometry
    }

    pub fn popup_dismissed(&self, index: usize) -> bool {
        self.globals.toplevels.popups[index].done
    }

    pub fn pointer_on_popup(&self, index: usize) -> bool {
        self.globals
            .input
            .pointer_focus
            .as_ref()
            .is_some_and(|(surface, _, _)| *surface == self.globals.toplevels.popups[index].surface)
    }
}

impl Dispatch<XdgPopup, ()> for Globals {
    fn event(
        globals: &mut Self,
        popup: &XdgPopup,
        event: xdg_popup::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(known) = globals
            .toplevels
            .popups
            .iter_mut()
            .find(|known| &known.popup == popup)
        else {
            return;
        };
        match event {
            xdg_popup::Event::Configure {
                x,
                y,
                width,
                height,
            } => known.geometry = Some((x, y, width, height)),
            xdg_popup::Event::PopupDone => known.done = true,
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(Globals: ignore XdgPositioner);
