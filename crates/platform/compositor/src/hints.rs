use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::get_parent;
use smithay::wayland::xdg_system_bell::XdgSystemBellHandler;
use smithay::{
    delegate_alpha_modifier, delegate_content_type, delegate_single_pixel_buffer,
    delegate_xdg_system_bell,
};

use crate::shell;
use crate::state::State;

delegate_alpha_modifier!(State);
delegate_content_type!(State);
delegate_single_pixel_buffer!(State);
delegate_xdg_system_bell!(State);

fn root_of(surface: &WlSurface) -> WlSurface {
    let mut root = surface.clone();
    while let Some(parent) = get_parent(&root) {
        root = parent;
    }
    root
}

impl XdgSystemBellHandler for State {
    fn ring(&mut self, surface: Option<WlSurface>) {
        let Some(root) = surface.as_ref().map(root_of) else {
            return;
        };
        if self.focused().as_ref() != Some(&root) {
            self.request_attention(shell::toplevel_names(&root).1);
        }
    }
}
