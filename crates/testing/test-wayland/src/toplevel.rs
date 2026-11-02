use wayland_client::protocol::wl_shm::Format;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::xdg_popup::{self, XdgPopup};
use wayland_protocols::xdg::shell::client::xdg_positioner::{
    Anchor, ConstraintAdjustment, Gravity, XdgPositioner,
};
use wayland_protocols::xdg::shell::client::xdg_toplevel::{self, XdgToplevel};

use crate::{Compositor, Globals, ShmImage, TestClient};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopupRequest {
    pub parent: usize,
    pub anchor_rect: (i32, i32, i32, i32),
    pub size: (i32, i32),
    pub adjustment: ConstraintAdjustment,
    pub grab: bool,
    pub color: [u8; 4],
}

#[derive(Default)]
pub(crate) struct Toplevels {
    popups: Vec<(XdgPopup, bool)>,
    suspended: Vec<XdgToplevel>,
}

impl TestClient {
    pub fn open_popup(&mut self, server: &mut Compositor, request: &PopupRequest) -> usize {
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
        positioner.set_constraint_adjustment(request.adjustment);
        let popup =
            xdg_surface.get_popup(Some(&self.windows[request.parent].1), &positioner, &qh, ());
        positioner.destroy();
        if request.grab {
            let input = &self.globals.input;
            popup.grab(
                input.seat.as_ref().expect("use_seat first"),
                input.button_serial,
            );
        }
        surface.commit();
        self.globals.toplevels.popups.push((popup, false));
        self.exchange(server);
        let (width, height) = request.size;
        let pixels = request
            .color
            .repeat(usize::try_from(width * height).expect("buffer size"));
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
        self.exchange(server);
        self.globals.toplevels.popups.len() - 1
    }

    pub fn popup_dismissed(&self, index: usize) -> bool {
        self.globals.toplevels.popups[index].1
    }

    pub fn is_suspended(&self, index: usize) -> bool {
        self.globals
            .toplevels
            .suspended
            .contains(&self.windows[index].2)
    }
}

impl Dispatch<XdgToplevel, ()> for Globals {
    fn event(
        globals: &mut Self,
        toplevel: &XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure { states, .. } = event {
            let suspended = states
                .as_chunks::<4>()
                .0
                .iter()
                .any(|state| *state == (xdg_toplevel::State::Suspended as u32).to_ne_bytes());
            let known = &mut globals.toplevels.suspended;
            known.retain(|known| known != toplevel);
            if suspended {
                known.push(toplevel.clone());
            }
        }
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
        if let xdg_popup::Event::PopupDone = event
            && let Some(known) = globals
                .toplevels
                .popups
                .iter_mut()
                .find(|(known, _)| known == popup)
        {
            known.1 = true;
        }
    }
}

delegate_noop!(Globals: ignore XdgPositioner);
