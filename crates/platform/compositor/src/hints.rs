use smithay::reexports::wayland_protocols::wp::content_type::v1::server::wp_content_type_v1;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::{get_parent, with_states};
use smithay::wayland::content_type::ContentTypeSurfaceCachedState;
use smithay::wayland::xdg_system_bell::XdgSystemBellHandler;
use smithay::{
    delegate_alpha_modifier, delegate_content_type, delegate_single_pixel_buffer,
    delegate_xdg_system_bell,
};

use crate::shell;
use crate::state::{Compositor, State};

delegate_alpha_modifier!(State);
delegate_content_type!(State);
delegate_single_pixel_buffer!(State);
delegate_xdg_system_bell!(State);

const WINDOW_BELL: &str = "bell-window-system";

pub(crate) fn wants_exact_timing(surface: &WlSurface) -> bool {
    with_states(surface, |states| {
        matches!(
            states
                .cached_state
                .get::<ContentTypeSurfaceCachedState>()
                .current()
                .content_type(),
            wp_content_type_v1::Type::Game | wp_content_type_v1::Type::Video
        )
    })
}

fn root_of(surface: &WlSurface) -> WlSurface {
    let mut root = surface.clone();
    while let Some(parent) = get_parent(&root) {
        root = parent;
    }
    root
}

impl Compositor {
    pub fn wants_variable_refresh(&self, output: &str) -> bool {
        let state = &self.state;
        let Some(screen) = state.screen_named(output) else {
            return false;
        };
        let workspaces = &state.screens[screen].workspaces;
        let active = workspaces.active_id();
        state.screens[screen].variable_refresh
            && workspaces.is_full_screen(active)
            && state.windows.iter().any(|window| {
                window.workspace == active
                    && state.screen_of(window) == screen
                    && wants_exact_timing(window.surface())
            })
    }
}

impl XdgSystemBellHandler for State {
    fn ring(&mut self, surface: Option<WlSurface>) {
        if let Some(session) = &self.assistive.session_bus {
            bande_sound::play(session, WINDOW_BELL);
        }
        let Some(root) = surface.as_ref().map(root_of) else {
            return;
        };
        if self.focused().as_ref() != Some(&root) {
            self.request_attention(shell::toplevel_names(&root).1);
        }
    }
}
