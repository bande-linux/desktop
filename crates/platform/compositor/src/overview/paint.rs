use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::element::{Id, Kind};
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::shell::wlr_layer::Layer;
use unbound_graphics::Palette;
use unbound_render::VulkanRenderer;

use super::layout::{inflate, real_rect};
use super::{BACKDROP_IDS, Frame, Hit, Press};
use crate::label::TITLE;
use crate::scene::{
    BlurElement, CORNER_RADIUS, FillElement, Piece, fresh, pieces_of, premultiplied, rounded,
    scaled_pieces,
};
use crate::shell::toplevel_names;
use crate::state::{OutputElement, State};
use crate::window::Window;

const PLUS_ARM: f64 = 0.3;
const PLUS_THICKNESS: f64 = 2.0;
const TITLE_GAP: f64 = 8.0;
const TITLE_MIN_WIDTH: f64 = 160.0;
const RING: f64 = 4.0;
const ADD_FILL: f32 = 0.6;
const TARGET_FILL: f32 = 0.45;

impl State {
    pub(super) fn update_title(&mut self) {
        let Some((surface, slot)) = self
            .overview
            .selected
            .clone()
            .filter(|_| self.overview.dragged().is_none())
            .and_then(|surface| {
                let slot = self.overview.slot(&surface)?.rect;
                Some((surface, slot))
            })
        else {
            self.overview.title = None;
            return;
        };
        let (title, app_id) = toplevel_names(&surface);
        let text = if title.is_empty() { app_id } else { title };
        let scale = self.window_index(&surface).map_or(1.0, |index| {
            self.screens[self.screen_of(&self.windows[index])].scale()
        });
        if self.overview.title.as_ref().is_some_and(|(known, label)| {
            known == &surface
                && label.text == text
                && label.scale == scale
                && label.scheme == self.decorations.scheme
        }) {
            return;
        }
        let width = slot.size.w.max(TITLE_MIN_WIDTH);
        let label =
            self.labels
                .get_mut()
                .render(&text, TITLE, width, self.decorations.scheme, scale);
        self.overview.title = label.map(|label| (surface, label));
    }

    fn title_element(
        &self,
        renderer: &mut VulkanRenderer,
        screen: usize,
        scale: f64,
    ) -> Option<OutputElement> {
        let (surface, label) = self.overview.title.as_ref()?;
        let index = self.window_index(surface)?;
        if self.screen_of(&self.windows[index]) != screen {
            return None;
        }
        let rect = self.presented(&self.windows[index])?.rect;
        let output = self.screen_rect(screen);
        let bottom = f64::from(output.loc.y + output.size.h);
        let x = rect.loc.x + (rect.size.w - label.size.w) / 2.0;
        let y = (rect.loc.y + rect.size.h + TITLE_GAP).min(bottom - label.size.h);
        let location = Point::<f64, Logical>::from((x, y))
            .to_physical(scale)
            .to_i32_round::<i32>()
            .to_f64();
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            location,
            &label.buffer,
            Some(self.overview.progress as f32),
            None,
            None,
            Kind::Unspecified,
        )
        .ok()
        .map(OutputElement::Memory)
    }

    pub(crate) fn overview_ring(&self, window: &Window, scale: f64) -> Option<OutputElement> {
        if self.overview.selected.as_ref() != Some(window.surface())
            || self.overview.dragged().is_some()
        {
            return None;
        }
        self.overview.slot(window.surface())?;
        let shown = self.presented(window)?;
        let shrink = (shown.rect.size.w / real_rect(window)?.size.w.max(1.0)).min(1.0);
        let rect = inflate(shown.rect, RING);
        let alpha = (self.overview.progress * shown.alpha) as f32;
        Some(OutputElement::Fill(FillElement::new(
            self.overview
                .ids
                .get(self.screen_of(window))?
                .first()?
                .clone(),
            rounded(
                rect.to_physical(scale),
                (CORNER_RADIUS * shrink + RING) * scale,
            ),
            premultiplied(self.palette().accent, alpha),
        )))
    }

    fn palette(&self) -> Palette {
        Palette::for_scheme(self.decorations.scheme)
    }

    pub(crate) fn overview_backdrop(
        &self,
        screen: usize,
        scale: f64,
        reduced_effects: bool,
    ) -> Vec<OutputElement> {
        let alpha = self.overview.progress as f32;
        if alpha <= 0.0 {
            return Vec::new();
        }
        let Some(ids) = self.overview.ids.get(screen) else {
            return Vec::new();
        };
        let ids = [BACKDROP_IDS, 1].map(|from_end| {
            ids.get(ids.len().wrapping_sub(from_end))
                .cloned()
                .unwrap_or_else(Id::new)
        });
        self.dim_screen(screen, scale, reduced_effects, ids, alpha)
    }

    pub(crate) fn dim_screen(
        &self,
        screen: usize,
        scale: f64,
        reduced_effects: bool,
        [scrim, blur]: [Id; 2],
        alpha: f32,
    ) -> Vec<OutputElement> {
        if alpha <= 0.0 {
            return Vec::new();
        }
        let output = self.screen_rect(screen);
        let output = output.to_f64();
        let area = rounded(output.to_physical(scale), 0.0);
        let scrim = OutputElement::Fill(FillElement::new(
            scrim,
            area,
            premultiplied(self.palette().scrim, alpha),
        ));
        if reduced_effects {
            return vec![scrim];
        }
        vec![
            scrim,
            OutputElement::Blur(BlurElement::new(blur, area, scale, alpha)),
        ]
    }

    pub(crate) fn overview_elements(
        &self,
        renderer: &mut VulkanRenderer,
        screen: usize,
        scale: f64,
    ) -> Vec<OutputElement> {
        let progress = self.overview.progress;
        if progress <= 0.0 {
            return Vec::new();
        }
        let title = self.title_element(renderer, screen, scale);
        if self.overview.app.is_some() {
            return title.into_iter().collect();
        }
        let alpha = progress as f32;
        let shift = Point::from((0.0, self.strip_shift(screen)));
        let (thumbnails, add) = self.strip(screen);
        let factor = self.thumbnail_scale(screen);
        let output = self.screen_rect(screen);
        let origin = output.loc.to_f64();
        let output = Rectangle::new(origin, output.size.to_f64());
        let wallpaper: Vec<Piece> = self
            .placed_layers(screen, Layer::Background)
            .into_iter()
            .flat_map(|(surface, at)| {
                let surfaces: Vec<WaylandSurfaceRenderElement<VulkanRenderer>> =
                    render_elements_from_surface_tree(
                        renderer,
                        &surface,
                        at.to_f64().to_physical(scale).to_i32_round::<i32>(),
                        scale,
                        1.0,
                        Kind::Unspecified,
                    );
                pieces_of(&surfaces, scale)
            })
            .collect();
        let target = match (&self.overview.press, &self.overview.hovered) {
            (Some(Press { dragging: true, .. }), Some(hovered)) => Some(hovered),
            _ => None,
        };
        let ids = self.overview.ids.get(screen);
        let id = |index: usize| {
            ids.and_then(|ids| ids.get(index))
                .cloned()
                .unwrap_or_else(Id::new)
        };
        let palette = self.palette();
        let mut elements: Vec<OutputElement> = title.into_iter().collect();
        let plus = Rectangle::new(add.loc + shift, add.size);
        let center = plus.loc + plus.size.downscale(2.0).to_point();
        let arm = plus.size.w.min(plus.size.h) * PLUS_ARM;
        for (index, bar) in [
            Rectangle::new(
                center - Point::from((arm, PLUS_THICKNESS / 2.0)),
                (2.0 * arm, PLUS_THICKNESS).into(),
            ),
            Rectangle::new(
                center - Point::from((PLUS_THICKNESS / 2.0, arm)),
                (PLUS_THICKNESS, 2.0 * arm).into(),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            elements.push(OutputElement::Fill(FillElement::new(
                id(1 + index),
                rounded(bar.to_physical(scale), PLUS_THICKNESS / 2.0 * scale),
                premultiplied(palette.ink2, alpha),
            )));
        }
        let add_fill = if target == Some(&Hit::Add(screen)) {
            premultiplied(palette.accent, TARGET_FILL * alpha)
        } else {
            premultiplied(palette.overlay, ADD_FILL * alpha)
        };
        elements.push(OutputElement::Fill(FillElement::new(
            id(3),
            rounded(plus.to_physical(scale), CORNER_RADIUS * scale),
            add_fill,
        )));
        for (index, thumbnail) in thumbnails.iter().enumerate() {
            let thumbnail = Rectangle::new(thumbnail.loc + shift, thumbnail.size);
            let shown = thumbnail.to_physical(scale);
            let radius = CORNER_RADIUS * factor * scale;
            let map = |rect: Frame| {
                Rectangle::new(
                    thumbnail.loc + (rect.loc - origin).upscale(factor),
                    rect.size.upscale(factor),
                )
            };
            let workspaces = &self.screens[screen].workspaces;
            let space = workspaces.id_at(index);
            for window in self
                .windows
                .iter()
                .rev()
                .filter(|window| window.visible() && Some(window.workspace) == space)
            {
                let Some(rect) = real_rect(window) else {
                    continue;
                };
                let surfaces: Vec<WaylandSurfaceRenderElement<VulkanRenderer>> =
                    render_elements_from_surface_tree(
                        renderer,
                        window.surface(),
                        window
                            .surface_origin()
                            .to_f64()
                            .to_physical(scale)
                            .to_i32_round::<i32>(),
                        scale,
                        1.0,
                        Kind::Unspecified,
                    );
                let small = map(rect).to_physical(scale);
                let mut pieces = pieces_of(&surfaces, scale);
                pieces.extend(window.title_bar_piece(scale));
                elements.extend(scaled_pieces(
                    &fresh(pieces),
                    rect.to_physical(scale),
                    small,
                    rounded(small, CORNER_RADIUS * factor * scale),
                    alpha,
                ));
            }
            elements.extend(scaled_pieces(
                &fresh(wallpaper.clone()),
                output.to_physical(scale),
                shown,
                rounded(shown, radius),
                alpha,
            ));
            elements.push(OutputElement::Fill(FillElement::new(
                id(4 + 2 * index),
                rounded(shown, radius),
                premultiplied(palette.chassis, alpha),
            )));
            let ring = if target == Some(&Hit::Space(screen, index)) {
                Some(premultiplied(palette.accent, TARGET_FILL * alpha))
            } else if index == workspaces.active() {
                Some(premultiplied(palette.accent, alpha))
            } else {
                None
            };
            if let Some(ring) = ring {
                elements.push(OutputElement::Fill(FillElement::new(
                    id(5 + 2 * index),
                    rounded(
                        inflate(thumbnail, RING / 2.0).to_physical(scale),
                        radius + RING / 2.0 * scale,
                    ),
                    ring,
                )));
            }
        }
        elements
    }
}
