use std::ffi::OsStr;
use std::time::Duration;

use bande_spring::{Animated, Spring};
use smithay::desktop::layer_map_for_output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Rectangle, Size};
use smithay::wayland::compositor::{RectangleKind, SurfaceAttributes, with_states};
use smithay::wayland::shell::wlr_layer::Layer;

use crate::environment::notify_service_manager;
use crate::motion::{BAR_NAMESPACE, SETTLED_ALPHA};
use crate::state::{Compositor, State, is_mapped};

const LOGIN_WINDOW_NAMESPACE: &str = "greeter";
const LOGIN_READY_SOCKET: &str = "/run/bande/login-ready";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Awaited {
    Shell,
    LoginWindow,
}

#[derive(Default)]
pub(crate) enum Reveal {
    #[default]
    Shown,
    Holding {
        since: Duration,
        deadline: Duration,
        ready: Awaited,
    },
    Fading {
        opacity: Animated<Duration>,
        now: Duration,
    },
}

fn opaque_over(surface: &WlSurface, size: Size<i32, Logical>) -> bool {
    is_mapped(surface)
        && with_states(surface, |states| {
            let mut attributes = states.cached_state.get::<SurfaceAttributes>();
            attributes
                .current()
                .opaque_region
                .as_ref()
                .is_some_and(|region| {
                    region.rects.iter().any(|(kind, rect)| {
                        matches!(kind, RectangleKind::Add)
                            && rect.contains_rect(Rectangle::from_size(size))
                    })
                })
        })
}

impl State {
    fn login_window_ready(&self) -> bool {
        self.screens
            .iter()
            .filter(|screen| screen.mirror.is_none())
            .all(|screen| {
                let map = layer_map_for_output(&screen.output);
                map.layers().any(|layer| {
                    layer.namespace() == LOGIN_WINDOW_NAMESPACE && is_mapped(layer.wl_surface())
                })
            })
    }

    fn shell_ready(&self) -> bool {
        self.screens
            .iter()
            .filter(|screen| screen.mirror.is_none())
            .all(|screen| {
                let map = layer_map_for_output(&screen.output);
                let size = screen.rect().size;
                let wallpaper = map.layers_on(Layer::Background).any(|layer| {
                    map.layer_geometry(layer)
                        .is_some_and(|geometry| geometry.size == size)
                        && opaque_over(layer.wl_surface(), size)
                });
                let bar = map.layers().any(|layer| {
                    layer.namespace() == BAR_NAMESPACE && is_mapped(layer.wl_surface())
                });
                wallpaper && bar
            })
    }

    fn show_login_window(&mut self, now: Duration, since: Duration) {
        let waited = now.saturating_sub(since).as_millis();
        tracing::info!("the login window is shown {waited} ms after the compositor started");
        self.reveal = Reveal::Shown;
        self.needs_redraw = true;
    }

    fn begin_reveal(&mut self, now: Duration, since: Duration, ready: bool) {
        let waited = now.saturating_sub(since).as_millis();
        match ready {
            true => tracing::info!("the desktop is shown {waited} ms after the session started"),
            false => tracing::warn!(
                "the desktop is shown after {waited} ms without waiting any longer for the shell"
            ),
        }
        self.reveal = match self.reduce_motion {
            true => Reveal::Shown,
            false => {
                let mut opacity = Animated::new(0.0);
                opacity.animate_at(now, false, 1.0, Spring::STANDARD);
                Reveal::Fading { opacity, now }
            }
        };
        self.needs_redraw = true;
    }

    pub(crate) fn animate_reveal(&mut self, time: Duration) {
        let Reveal::Fading { opacity, now } = &mut self.reveal else {
            return;
        };
        *now = time;
        if opacity.is_at_rest(time, SETTLED_ALPHA) {
            self.reveal = Reveal::Shown;
        }
    }

    pub(crate) fn is_revealing(&self) -> bool {
        matches!(self.reveal, Reveal::Fading { .. })
    }

    pub(crate) fn shell_opacity(&self) -> f32 {
        match &self.reveal {
            Reveal::Shown => 1.0,
            Reveal::Holding { .. } => 0.0,
            Reveal::Fading { opacity, now } => {
                opacity.state_at(*now).position.clamp(0.0, 1.0) as f32
            }
        }
    }
}

impl Compositor {
    pub fn hold_frames_until(&mut self, ready: Awaited, since: Duration, deadline: Duration) {
        self.state.reveal = Reveal::Holding {
            since,
            deadline,
            ready,
        };
    }

    pub fn frame_presented(&mut self) {
        if !std::mem::take(&mut self.state.ready_unannounced) {
            return;
        }
        if let Err(error) = notify_service_manager(Some(OsStr::new(LOGIN_READY_SOCKET)), "READY=1")
        {
            tracing::debug!("nothing heard that the login window came up: {error}");
        }
    }

    pub fn holds_frames(&self) -> bool {
        matches!(self.state.reveal, Reveal::Holding { .. })
    }

    pub fn reveal_wake(&mut self, now: Duration) -> Option<Duration> {
        let Reveal::Holding {
            since,
            deadline,
            ready: awaited,
        } = self.state.reveal
        else {
            return None;
        };
        let ready = match awaited {
            Awaited::Shell => self.state.shell_ready(),
            Awaited::LoginWindow => self.state.login_window_ready(),
        };
        if !ready && now < deadline {
            return Some(deadline);
        }
        self.state.ready_unannounced = ready;
        match awaited {
            Awaited::Shell => self.state.begin_reveal(now, since, ready),
            Awaited::LoginWindow => self.state.show_login_window(now, since),
        }
        None
    }
}
