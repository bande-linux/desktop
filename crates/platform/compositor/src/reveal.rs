use std::time::Duration;

use bande_render::{VulkanRenderer, VulkanTexture};
use bande_spring::{Animated, Spring};
use smithay::backend::renderer::utils::{import_surface_tree, with_renderer_surface_state};
use smithay::backend::renderer::{ContextId, Renderer as _};
use smithay::desktop::layer_map_for_output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Clock, Logical, Monotonic, Rectangle, Size};
use smithay::wayland::compositor::{RectangleKind, SurfaceAttributes, with_states};
use smithay::wayland::shell::wlr_layer::Layer;

use crate::motion::{BAR_NAMESPACE, Look, Motion, SETTLED_ALPHA};
use crate::scene::{Closing, drawn_texture};
use crate::state::{Compositor, State, is_mapped};

const LOGIN_WINDOW_NAMESPACE: &str = "greeter";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Awaited {
    Shell,
    LoginWindow,
}

pub struct ShownFrame {
    pub output: String,
    pub texture: VulkanTexture,
    pub context: ContextId<VulkanTexture>,
}

#[derive(Default)]
pub(crate) enum Reveal {
    #[default]
    Shown,
    Holding {
        since: Duration,
        deadline: Duration,
        ready: Awaited,
        drawable: bool,
        over: Vec<ShownFrame>,
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

fn drawable(surface: &WlSurface, context: &ContextId<VulkanTexture>) -> bool {
    with_renderer_surface_state(surface, |state| state.texture(context.clone()).is_some())
        .unwrap_or(false)
}

impl State {
    fn login_window(&self) -> Option<Vec<(String, WlSurface)>> {
        let map = layer_map_for_output(&self.main_screen().output);
        map.layers()
            .find(|layer| {
                layer.namespace() == LOGIN_WINDOW_NAMESPACE && is_mapped(layer.wl_surface())
            })
            .map(|layer| vec![(layer.namespace().to_owned(), layer.wl_surface().clone())])
    }

    fn shell(&self) -> Option<Vec<(String, WlSurface)>> {
        let mut surfaces = Vec::new();
        for screen in self.screens.iter().filter(|screen| screen.mirror.is_none()) {
            let map = layer_map_for_output(&screen.output);
            let size = screen.rect().size;
            let wallpaper = map.layers_on(Layer::Background).find(|layer| {
                map.layer_geometry(layer)
                    .is_some_and(|geometry| geometry.size == size)
                    && opaque_over(layer.wl_surface(), size)
            })?;
            let bar = map.layers().find(|layer| {
                layer.namespace() == BAR_NAMESPACE && is_mapped(layer.wl_surface())
            })?;
            surfaces.extend(
                [wallpaper, bar]
                    .map(|layer| (layer.namespace().to_owned(), layer.wl_surface().clone())),
            );
        }
        Some(surfaces)
    }

    fn awaited(&self, awaited: Awaited) -> Option<Vec<(String, WlSurface)>> {
        match awaited {
            Awaited::Shell => self.shell(),
            Awaited::LoginWindow => self.login_window(),
        }
    }

    fn fade_out(&mut self, over: Vec<ShownFrame>) {
        let spring = match self.reduce_motion {
            true => Spring::FADE,
            false => Spring::STANDARD,
        };
        for frame in over {
            let Some(screen) = self.screen_named(&frame.output) else {
                continue;
            };
            let rect = self.screen_rect(screen).to_f64();
            let drawn = drawn_texture(
                frame.texture,
                frame.context,
                rect,
                self.render_scale(screen),
                self.decorations.scheme,
            );
            let motion = Motion::new(spring, Look::shown(rect), Look::hidden(rect));
            self.closing
                .push(Closing::covering(drawn, motion, frame.output));
        }
        self.reveal = Reveal::Shown;
        self.needs_redraw = true;
    }

    fn show_login_window(&mut self, now: Duration, since: Duration, over: Vec<ShownFrame>) {
        let waited = now.saturating_sub(since).as_millis();
        tracing::info!("the login window is shown {waited} ms after the compositor started");
        self.fade_out(over);
    }

    fn begin_reveal(&mut self, now: Duration, since: Duration, ready: bool, over: Vec<ShownFrame>) {
        let waited = now.saturating_sub(since).as_millis();
        match ready {
            true => tracing::info!("the desktop is shown {waited} ms after the session started"),
            false => tracing::warn!(
                "the desktop is shown after {waited} ms without waiting any longer for the shell"
            ),
        }
        if !over.is_empty() {
            self.fade_out(over);
            return;
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
        self.state.awaited = Some((ready, deadline));
        self.state.reveal = Reveal::Holding {
            since,
            deadline,
            ready,
            drawable: false,
            over: Vec::new(),
        };
    }

    pub fn import_awaited(&mut self, renderer: &mut VulkanRenderer) {
        let Reveal::Holding { ready, .. } = self.state.reveal else {
            return;
        };
        let Some(surfaces) = self.state.awaited(ready) else {
            return;
        };
        for (_, surface) in &surfaces {
            if let Err(error) = import_surface_tree(renderer, surface) {
                tracing::warn!("cannot draw what is awaited ({ready:?}) yet: {error}");
            }
        }
        let context = renderer.context_id();
        if surfaces
            .iter()
            .all(|(_, surface)| drawable(surface, &context))
            && let Reveal::Holding { drawable, .. } = &mut self.state.reveal
        {
            *drawable = true;
            self.state.needs_redraw = true;
        }
    }

    pub fn awaited_shown(&mut self, renderer: &VulkanRenderer, drew_everything: bool) -> bool {
        let Some((awaited, deadline)) = self.state.awaited else {
            return true;
        };
        let context = renderer.context_id();
        let undrawn: Option<Vec<String>> = self.state.awaited(awaited).map(|surfaces| {
            surfaces
                .into_iter()
                .filter(|(_, surface)| !drawable(surface, &context))
                .map(|(namespace, _)| namespace)
                .collect()
        });
        let shown = drew_everything && undrawn.as_ref().is_some_and(Vec::is_empty);
        let now = Duration::from(Clock::<Monotonic>::new().now());
        if !shown && now < deadline {
            return false;
        }
        match (shown, undrawn) {
            (true, _) => {}
            (false, None) => {
                tracing::warn!("going on after {awaited:?} never showed: not every surface mapped")
            }
            (false, Some(undrawn)) if undrawn.is_empty() => tracing::warn!(
                "going on after {awaited:?} never showed: an element could not be drawn"
            ),
            (false, Some(undrawn)) => tracing::warn!(
                "going on after {awaited:?} never showed: {} not drawn",
                undrawn.join(", ")
            ),
        }
        self.state.awaited = None;
        true
    }

    pub fn fade_in_over(&mut self, frames: Vec<ShownFrame>) {
        if let Reveal::Holding { over, .. } = &mut self.state.reveal {
            *over = frames;
        }
    }

    pub fn holds_frames(&self) -> bool {
        matches!(self.state.reveal, Reveal::Holding { .. })
    }

    /// Whether everything the held frames wait for can be drawn.
    pub fn awaited_drawable(&self) -> bool {
        matches!(self.state.reveal, Reveal::Holding { drawable: true, .. })
    }

    pub fn reveal_wake(&mut self, now: Duration) -> Option<Duration> {
        let Reveal::Holding {
            since,
            deadline,
            ready: awaited,
            drawable: ready,
            ..
        } = self.state.reveal
        else {
            return None;
        };
        if !ready && now < deadline {
            return Some(deadline);
        }
        let over = match &mut self.state.reveal {
            Reveal::Holding { over, .. } => std::mem::take(over),
            _ => Vec::new(),
        };
        match awaited {
            Awaited::Shell => self.state.begin_reveal(now, since, ready, over),
            Awaited::LoginWindow => self.state.show_login_window(now, since, over),
        }
        None
    }
}
