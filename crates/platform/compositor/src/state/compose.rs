use std::sync::Mutex;

use super::{Compositor, OutputElement, State, is_mapped};
use crate::element::ScreenElement;
use smithay::backend::renderer::Renderer as _;
use smithay::backend::renderer::element::Kind as ElementKind;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::surface::render_elements_from_surface_tree;
use smithay::desktop::{LayerMap, PopupManager, WindowSurfaceType, layer_map_for_output};
use smithay::input::pointer::{CursorIcon, CursorImageAttributes, CursorImageStatus};

use bande_render::VulkanRenderer;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Physical, Point};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::wlr_layer::Layer;

use crate::scene::{
    Closing, Drawn, Shading, preview_elements, scrim, surface_blurs, tie_blurs_to_what_lies_below,
    window_elements,
};
use crate::window::Window;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Band {
    Front,
    Leaving,
    OnTop,
    Rest,
}

struct Layers<'m> {
    map: &'m LayerMap,
    origin: Point<i32, Logical>,
    screen: usize,
}

impl Layers<'_> {
    fn push(&self, placed: &mut Vec<Placed<'_>>, layer: Layer) {
        for mapped in self.map.layers_on(layer).rev() {
            let Some(geometry) = self.map.layer_geometry(mapped) else {
                continue;
            };
            let surface = mapped.wl_surface();
            let at = self.origin + geometry.loc;
            placed.extend(popup_surfaces(surface, at));
            placed.push(Placed::Surface(surface.clone(), at));
            if crate::effect::scrim(surface).is_some() && is_mapped(surface) {
                placed.push(Placed::Scrim(surface.clone(), self.screen));
            }
        }
    }
}

fn push_windows<'a>(
    placed: &mut Vec<Placed<'a>>,
    visible: &[(Band, usize, &'a Window)],
    band: Band,
) {
    for (_, index, window) in visible.iter().filter(|(of, _, _)| *of == band) {
        placed.extend(popup_surfaces(window.surface(), window.content_location()));
        placed.push(Placed::Window(*index, window));
    }
}

fn popup_surfaces<'a>(
    root: &WlSurface,
    geometry_origin: Point<i32, Logical>,
) -> impl Iterator<Item = Placed<'a>> + use<'a> {
    PopupManager::popups_for_surface(root).map(move |(popup, offset)| {
        let own = popup.geometry().loc;
        Placed::Surface(
            popup.wl_surface().clone(),
            (
                geometry_origin.x + offset.x - own.x,
                geometry_origin.y + offset.y - own.y,
            )
                .into(),
        )
    })
}

#[derive(Clone)]
pub(crate) enum Placed<'a> {
    Surface(WlSurface, Point<i32, Logical>),
    Window(usize, &'a Window),
    Closing(&'a Closing),
    Overview(usize),
    Backdrop(usize),
    Scrim(WlSurface, usize),
}

type Composed<E = ScreenElement> = (
    Vec<E>,
    Vec<(usize, Option<Drawn>)>,
    Option<(Vec<crate::scene::Piece>, crate::scene::Area)>,
);

fn relocated(elements: Vec<OutputElement>, by: Point<i32, Physical>) -> Vec<ScreenElement> {
    elements
        .into_iter()
        .map(|element| ScreenElement::new(element, by))
        .collect()
}

impl Compositor {
    pub fn render_elements_on(
        &mut self,
        renderer: &mut VulkanRenderer,
        output: &str,
    ) -> Vec<ScreenElement> {
        match self.state.screen_named(output) {
            Some(screen) => self.render_screen(renderer, screen),
            None => Vec::new(),
        }
    }

    fn render_screen(
        &mut self,
        renderer: &mut VulkanRenderer,
        screen: usize,
    ) -> Vec<ScreenElement> {
        self.state.paint_title_bars(renderer);
        self.state.prepare_switcher(screen);
        self.state.prepare_window_menu(screen);
        self.state.prepare_force_quit(screen);
        let (elements, drawn, icon) = self.compose(renderer, screen, true);
        if self.state.pointer_screen() == screen {
            self.state.remember_drag_icon(icon);
        }
        for index in 0..self.state.windows.len() {
            if self.state.screen_of(&self.state.windows[index]) == screen {
                self.state.windows[index].drawn = None;
            }
        }
        for (index, drawn) in drawn {
            if self.state.screen_of(&self.state.windows[index]) == screen {
                self.state.windows[index].drawn = drawn;
            }
        }
        elements
    }

    pub(crate) fn scene_elements(
        &self,
        renderer: &mut VulkanRenderer,
        screen: usize,
        with_cursor: bool,
    ) -> Vec<ScreenElement> {
        self.compose(renderer, screen, with_cursor).0
    }

    fn compose(&self, renderer: &mut VulkanRenderer, screen: usize, with_cursor: bool) -> Composed {
        if let Some(source) = self.state.screens[screen]
            .mirror
            .as_deref()
            .and_then(|source| self.state.screen_named(source))
        {
            return self.compose_mirror(renderer, screen, source, with_cursor);
        }
        let scale = self.state.render_scale(screen);
        let (elements, drawn, icon) = self.compose_at(renderer, screen, scale, with_cursor);
        let origin = self.state.view_origin_in_pixels(screen, scale);
        (relocated(elements, Point::default() - origin), drawn, icon)
    }

    fn compose_mirror(
        &self,
        renderer: &mut VulkanRenderer,
        screen: usize,
        source: usize,
        with_cursor: bool,
    ) -> Composed {
        let mirror = &self.state.screens[screen];
        let scale = mirror.scale();
        let frame = mirror.settings.transform.transform_size(mirror.mode_size());
        let shown = self
            .state
            .screen_rect(source)
            .size
            .to_f64()
            .to_physical(scale)
            .to_i32_round::<i32>();
        let inset =
            Point::<i32, Physical>::from(((frame.w - shown.w) / 2, (frame.h - shown.h) / 2));
        let (elements, _, _) = self.compose_at(renderer, source, scale, with_cursor);
        let origin = self.state.screens[source].origin_in_pixels(scale);
        (relocated(elements, inset - origin), Vec::new(), None)
    }

    fn compose_at(
        &self,
        renderer: &mut VulkanRenderer,
        screen: usize,
        scale: f64,
        with_cursor: bool,
    ) -> Composed<OutputElement> {
        let mut drawn = Vec::new();
        let pointer_here = self.state.pointer_screen() == screen;
        let (dragged, icon) = match with_cursor {
            true => self.state.drag_icon_elements(renderer, scale),
            false => (Vec::new(), None),
        };
        let mut elements: Vec<OutputElement> = match with_cursor && pointer_here {
            true => self.cursor_elements(renderer, scale),
            false => Vec::new(),
        };
        elements.extend(dragged);
        elements.extend(self.state.alert_elements(renderer, screen, scale));
        elements.extend(self.state.flash_elements(screen, scale));
        elements.extend(self.state.source_hud_element(renderer, screen, scale));
        elements.extend(self.state.force_quit_elements(renderer, screen, scale));
        elements.extend(self.state.switcher_elements(renderer, screen, scale));
        elements.extend(self.state.assist_elements(renderer, screen, scale));
        elements.extend(self.state.window_menu_elements(renderer, screen, scale));
        let key = self.state.key_window();
        let shell_opacity = self.state.shell_opacity();
        let wallpapers: Vec<WlSurface> = match shell_opacity < 1.0 {
            true => self
                .state
                .placed_layers(screen, Layer::Background)
                .into_iter()
                .map(|(surface, _)| surface)
                .collect(),
            false => Vec::new(),
        };
        for placed in self.state.scene(true, screen) {
            match placed {
                Placed::Surface(surface, location) => {
                    let alpha = match wallpapers.contains(&surface) {
                        true => 1.0,
                        false => shell_opacity,
                    };
                    elements.extend(render_elements_from_surface_tree(
                        renderer,
                        &surface,
                        location.to_f64().to_physical(scale).to_i32_round::<i32>(),
                        scale,
                        alpha,
                        ElementKind::Unspecified,
                    ));
                    elements.extend(surface_blurs(
                        &surface,
                        location,
                        scale,
                        self.state
                            .backdrop()
                            .or_else(|| scrim(renderer, self.state.decorations.scheme)),
                    ));
                }
                Placed::Window(index, window) => {
                    let shading = Shading {
                        active: key.as_ref() == Some(window.surface()),
                        scheme: self.state.decorations.scheme,
                    };
                    let hung = window
                        .surface()
                        .client()
                        .is_some_and(|owner| self.state.watchdog.is_unresponsive(&owner.id()));
                    let presented = self.state.presented(window);
                    let (shown, look) = window_elements(
                        window,
                        renderer,
                        scale,
                        shading,
                        hung,
                        presented,
                        self.state.backdrop(),
                    );
                    elements.extend(shown);
                    elements.extend(self.state.overview_ring(window, scale));
                    if let (Some(ghost), Some(look)) = (self.state.overview_ghost(window), &look) {
                        elements.extend(look.ghost(ghost, scale, &renderer.context_id()));
                    }
                    drawn.push((index, look));
                    if let Some(preview) = &self.state.preview
                        && &preview.owner == window.surface()
                    {
                        elements.extend(preview_elements(
                            preview,
                            scale,
                            self.state.decorations.scheme,
                        ));
                    }
                }
                Placed::Closing(closing) => {
                    elements.extend(closing.render_elements(scale, &renderer.context_id()));
                }
                Placed::Overview(screen) => {
                    elements.extend(self.state.overview_elements(renderer, screen, scale));
                }
                Placed::Backdrop(screen) => {
                    elements.extend(self.state.overview_backdrop(
                        screen,
                        scale,
                        renderer.reduced_effects(),
                    ));
                }
                Placed::Scrim(owner, screen) => {
                    if let Some(scrim) = crate::effect::scrim(&owner) {
                        elements.extend(self.state.dim_screen(
                            screen,
                            scale,
                            renderer.reduced_effects(),
                            scrim.ids,
                            scrim.opacity,
                        ));
                    }
                }
            }
        }
        tie_blurs_to_what_lies_below(&mut elements, scale);
        (elements, drawn, icon)
    }

    pub(crate) fn cursor_elements(
        &self,
        renderer: &mut VulkanRenderer,
        scale: f64,
    ) -> Vec<OutputElement> {
        let state = &self.state;
        if !state.cursor_shown {
            return Vec::new();
        }
        let at: Point<i32, Physical> = state.pointer_location.to_physical(scale).to_i32_round();
        let busy = CursorImageStatus::Named(CursorIcon::Wait);
        let dragging = state.drag_cursor().map(CursorImageStatus::Named);
        let over_hung_app = state
            .seat
            .get_pointer()
            .and_then(|pointer| pointer.current_focus())
            .and_then(|surface| surface.client())
            .is_some_and(|owner| state.watchdog.is_unresponsive(&owner.id()));
        let status = match &dragging {
            Some(dragging) => dragging,
            None if over_hung_app => &busy,
            None => &state.cursor_status,
        };
        match status {
            CursorImageStatus::Hidden => Vec::new(),
            CursorImageStatus::Surface(surface) => {
                let hotspot = with_states(surface, |states| {
                    states
                        .data_map
                        .get::<Mutex<CursorImageAttributes>>()
                        .and_then(|attributes| attributes.lock().ok().map(|a| a.hotspot))
                        .unwrap_or_default()
                });
                let hotspot = hotspot.to_f64().to_physical(scale).to_i32_round::<i32>();
                render_elements_from_surface_tree(
                    renderer,
                    surface,
                    (at.x - hotspot.x, at.y - hotspot.y),
                    scale,
                    1.0,
                    ElementKind::Cursor,
                )
            }
            CursorImageStatus::Named(icon) => {
                let cursor = state
                    .named_cursors
                    .get(icon)
                    .and_then(Option::as_ref)
                    .unwrap_or(&state.cursor_arrow);
                MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    (
                        f64::from(at.x - cursor.hotspot.x),
                        f64::from(at.y - cursor.hotspot.y),
                    ),
                    &cursor.buffer,
                    None,
                    None,
                    None,
                    ElementKind::Cursor,
                )
                .map(OutputElement::Memory)
                .into_iter()
                .collect()
            }
        }
    }
}

impl State {
    pub(crate) fn placed_layers(
        &self,
        screen: usize,
        layer: Layer,
    ) -> Vec<(WlSurface, Point<i32, Logical>)> {
        let origin = self.screens[screen].settings.position;
        let map = layer_map_for_output(&self.screens[screen].output);
        map.layers_on(layer)
            .rev()
            .filter_map(|mapped| {
                let geometry = map.layer_geometry(mapped)?;
                Some((mapped.wl_surface().clone(), origin + geometry.loc))
            })
            .collect()
    }

    pub(crate) fn layer_screen(&self, surface: &WlSurface) -> Option<usize> {
        self.screens.iter().position(|screen| {
            layer_map_for_output(&screen.output)
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .is_some()
        })
    }

    pub(crate) fn layer_origin(&self, surface: &WlSurface) -> Option<Point<i32, Logical>> {
        let screen = &self.screens[self.layer_screen(surface)?];
        let map = layer_map_for_output(&screen.output);
        let layer = map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)?;
        let geometry = map.layer_geometry(layer)?;
        Some(screen.settings.position + geometry.loc)
    }

    pub(crate) fn stack(&self, screen: usize) -> Vec<(WlSurface, Point<i32, Logical>)> {
        self.scene(false, screen)
            .into_iter()
            .filter_map(|placed| match placed {
                Placed::Surface(surface, origin) => Some((surface, origin)),
                Placed::Window(_, window) => {
                    Some((window.surface().clone(), window.surface_origin()))
                }
                Placed::Closing(_)
                | Placed::Overview(_)
                | Placed::Backdrop(_)
                | Placed::Scrim(..) => None,
            })
            .collect()
    }

    pub(crate) fn scene(&self, rendered: bool, screen: usize) -> Vec<Placed<'_>> {
        if self.is_locked() {
            let locked = self.locked_stack(screen);
            let wallpaper = if rendered && locked.is_empty() && self.lock_shown() {
                self.placed_layers(screen, Layer::Background)
            } else {
                Vec::new()
            };
            return locked
                .into_iter()
                .chain(wallpaper)
                .map(|(surface, origin)| Placed::Surface(surface, origin))
                .collect();
        }

        let visible = self
            .windows
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, window)| {
                self.screen_of(window) == screen || (rendered && self.dragged_onto(window, screen))
            })
            .filter(|(_, window)| {
                if rendered {
                    window.rendered() && self.on_view(window)
                } else {
                    self.present(window)
                        && !self.desktop.hides_windows()
                        && !self.overview.is_shown()
                }
            })
            .map(|(index, window)| {
                let band = if self.on_full_screen_space(window) || self.dragged_in_overview(window)
                {
                    Band::Front
                } else if window.is_minimized() {
                    Band::Leaving
                } else if window.keep_on_top {
                    Band::OnTop
                } else {
                    Band::Rest
                };
                (band, index, window)
            })
            .collect::<Vec<_>>();
        let overview = rendered && self.overview.is_shown();
        let name = self.screens[screen].name();
        let map = layer_map_for_output(&self.screens[screen].output);
        let layers = Layers {
            map: &map,
            origin: self.screens[screen].settings.position,
            screen,
        };
        let mut placed = Vec::with_capacity(2 * (visible.len() + map.layers().count()) + 2);
        layers.push(&mut placed, Layer::Overlay);
        push_windows(&mut placed, &visible, Band::Leaving);
        push_windows(&mut placed, &visible, Band::Front);
        layers.push(&mut placed, Layer::Top);
        if overview {
            placed.push(Placed::Overview(screen));
        }
        placed.extend(
            self.closing
                .iter()
                .filter(|closing| rendered && closing.output == name)
                .map(Placed::Closing),
        );
        push_windows(&mut placed, &visible, Band::OnTop);
        push_windows(&mut placed, &visible, Band::Rest);
        layers.push(&mut placed, Layer::Bottom);
        if overview {
            placed.push(Placed::Backdrop(screen));
        }
        layers.push(&mut placed, Layer::Background);
        placed
    }
}
