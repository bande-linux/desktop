use bande_graphics::{Elevation, Palette, Radius};
use bande_render::VulkanRenderer;
use bande_spring::Spring;
use smithay::backend::renderer::element::Id;
use smithay::input::keyboard::keysyms;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle};

use crate::motion::{Look, Motion, Tick};
use crate::overview::{arrange, real_rect};
use crate::scene::{Casting, FillElement, Shading, cast_shadows, premultiplied, rounded};
use crate::state::{Compositor, OutputElement, State};
use crate::tile::Tile;

type Frame = Rectangle<f64, Logical>;

const INSET: f64 = 12.0;
const PADDING: f64 = 24.0;
const PANEL_RADIUS: f64 = Radius::XL as f64;
const RING: f64 = 3.0;
const RING_RADIUS: f64 = Radius::L as f64;
const OPEN_SCALE: f64 = 0.96;

struct Offer {
    tiled: WlSurface,
    tile: Tile,
    screen: usize,
    panel: Frame,
    candidates: Vec<WlSurface>,
    selected: usize,
    look: Motion,
    closing: bool,
    ids: [Id; 2],
    shadow: Casting,
}

#[derive(Default)]
pub(crate) struct Assist {
    offer: Option<Offer>,
}

impl Assist {
    pub(crate) fn is_open(&self) -> bool {
        self.offer.as_ref().is_some_and(|offer| !offer.closing)
    }

    pub(crate) fn is_animating(&self) -> bool {
        self.offer
            .as_ref()
            .is_some_and(|offer| !offer.look.is_settled())
    }
}

impl State {
    pub(crate) fn offer_other_half(&mut self, index: usize) {
        self.dismiss_assist();
        let Some((tile @ (Tile::Left | Tile::Right), tiled)) = self.windows[index].tile else {
            return;
        };
        let screen = self.screen_of(&self.windows[index]);
        let area = self.usable_area(screen);
        let free = if tile == Tile::Left {
            Rectangle::new(
                (tiled.loc.x + tiled.size.w, area.loc.y).into(),
                (
                    area.loc.x + area.size.w - tiled.loc.x - tiled.size.w,
                    area.size.h,
                )
                    .into(),
            )
        } else {
            Rectangle::new(area.loc, (tiled.loc.x - area.loc.x, area.size.h).into())
        };
        let opposite = tile.opposite();
        let taken = self.windows.iter().enumerate().any(|(other, window)| {
            other != index
                && self.present(window)
                && self.screen_of(window) == screen
                && window.tile.is_some_and(|(placed, _)| placed == opposite)
        });
        let candidates: Vec<WlSurface> = self
            .windows
            .iter()
            .enumerate()
            .rev()
            .filter(|(other, window)| {
                *other != index
                    && self.present(window)
                    && self.screen_of(window) == screen
                    && real_rect(window).is_some()
            })
            .map(|(_, window)| window.surface().clone())
            .collect();
        if taken || candidates.is_empty() || free.size.w <= 2 * INSET as i32 {
            return;
        }
        let panel: Frame = Rectangle::new(
            free.loc.to_f64() + Point::from((INSET, INSET)),
            (
                f64::from(free.size.w) - 2.0 * INSET,
                f64::from(free.size.h) - 2.0 * INSET,
            )
                .into(),
        );
        let spring = if self.reduce_motion {
            Spring::FADE
        } else {
            Spring::QUICK
        };
        let from = if self.reduce_motion {
            Look::hidden(panel)
        } else {
            Look::hidden(panel).scaled(OPEN_SCALE)
        };
        self.assist.offer = Some(Offer {
            tiled: self.windows[index].surface().clone(),
            tile,
            screen,
            panel,
            candidates,
            selected: 0,
            look: Motion::new(spring, from, Look::shown(panel)),
            closing: false,
            ids: [Id::new(), Id::new()],
            shadow: Casting::new(),
        });
        self.needs_redraw = true;
    }

    pub(crate) fn dismiss_assist(&mut self) {
        let reduce = self.reduce_motion;
        if let Some(offer) = self.assist.offer.as_mut().filter(|offer| !offer.closing) {
            offer.closing = true;
            let hidden = Look::hidden(offer.panel);
            offer.look.retarget(if reduce {
                hidden
            } else {
                hidden.scaled(OPEN_SCALE)
            });
            self.needs_redraw = true;
        }
    }

    fn assist_slots(&self, offer: &Offer) -> Vec<(WlSurface, Frame)> {
        let shown = offer.look.look().rect;
        let inner = Rectangle::new(
            shown.loc + Point::from((PADDING, PADDING)),
            (
                (shown.size.w - 2.0 * PADDING).max(1.0),
                (shown.size.h - 2.0 * PADDING).max(1.0),
            )
                .into(),
        );
        let present: Vec<(WlSurface, Frame)> = offer
            .candidates
            .iter()
            .filter_map(|surface| {
                let index = self.window_index(surface)?;
                let window = &self.windows[index];
                self.present(window)
                    .then(|| real_rect(window))
                    .flatten()
                    .map(|rect| (surface.clone(), rect))
            })
            .collect();
        let rects: Vec<Frame> = present.iter().map(|(_, rect)| *rect).collect();
        present
            .into_iter()
            .map(|(surface, _)| surface)
            .zip(arrange(&rects, inner))
            .collect()
    }

    fn fill_with(&mut self, surface: WlSurface) {
        let Some(offer) = self.assist.offer.as_ref() else {
            return;
        };
        let opposite = offer.tile.opposite();
        self.dismiss_assist();
        let Some(index) = self.window_index(&surface) else {
            return;
        };
        self.tile_window(index, opposite, Spring::STANDARD);
        self.focus(Some(surface));
    }

    pub(crate) fn assist_key(&mut self, keysym: u32, shift: bool) {
        let Some(offer) = self.assist.offer.as_mut() else {
            return;
        };
        let count = offer.candidates.len().max(1);
        match keysym {
            keysyms::KEY_Escape => self.dismiss_assist(),
            keysyms::KEY_Return | keysyms::KEY_KP_Enter | keysyms::KEY_space => {
                if let Some(surface) = offer.candidates.get(offer.selected).cloned() {
                    self.fill_with(surface);
                }
            }
            keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab if shift => {
                offer.selected = (offer.selected + count - 1) % count;
            }
            keysyms::KEY_Left | keysyms::KEY_Up => {
                offer.selected = (offer.selected + count - 1) % count;
            }
            keysyms::KEY_Tab | keysyms::KEY_Right | keysyms::KEY_Down => {
                offer.selected = (offer.selected + 1) % count;
            }
            _ => {}
        }
        self.needs_redraw = true;
    }

    fn assist_hit(&self, at: Point<f64, Logical>) -> Option<Option<usize>> {
        let offer = self.assist.offer.as_ref().filter(|offer| !offer.closing)?;
        if !offer.look.look().rect.contains(at) {
            return Some(None);
        }
        let hit = self
            .assist_slots(offer)
            .into_iter()
            .find(|(_, rect)| rect.contains(at))
            .and_then(|(surface, _)| offer.candidates.iter().position(|known| *known == surface));
        Some(hit)
    }

    pub(crate) fn assist_pointer(&mut self, at: Point<f64, Logical>) {
        if let Some(Some(index)) = self.assist_hit(at)
            && let Some(offer) = self.assist.offer.as_mut()
            && offer.selected != index
        {
            offer.selected = index;
            self.needs_redraw = true;
        }
    }

    pub(crate) fn assist_click(&mut self) -> bool {
        match self.assist_hit(self.pointer_location) {
            Some(Some(index)) => {
                let chosen = self
                    .assist
                    .offer
                    .as_ref()
                    .and_then(|offer| offer.candidates.get(index).cloned());
                if let Some(surface) = chosen {
                    self.fill_with(surface);
                }
                true
            }
            Some(None) => {
                self.dismiss_assist();
                false
            }
            None => false,
        }
    }

    pub(crate) fn animate_assist(&mut self, tick: Tick) {
        let still_tiled = self.assist.offer.as_ref().is_some_and(|offer| {
            self.window_index(&offer.tiled).is_some_and(|index| {
                self.present(&self.windows[index])
                    && self.windows[index]
                        .tile
                        .is_some_and(|(tile, _)| tile == offer.tile)
            })
        });
        if !still_tiled {
            self.dismiss_assist();
        }
        if let Some(offer) = self.assist.offer.as_mut() {
            offer.look.advance(tick);
            if offer.closing && offer.look.is_settled() {
                self.assist.offer = None;
            }
            self.needs_redraw = true;
        }
    }

    pub(crate) fn assist_elements(
        &self,
        renderer: &mut VulkanRenderer,
        screen: usize,
        scale: f64,
    ) -> Vec<OutputElement> {
        let Some(offer) = self
            .assist
            .offer
            .as_ref()
            .filter(|offer| offer.screen == screen && !self.is_locked())
        else {
            return Vec::new();
        };
        let look = offer.look.look();
        let alpha = look.alpha as f32;
        if alpha <= 0.0 {
            return Vec::new();
        }
        let palette = Palette::for_scheme(self.decorations.scheme);
        let factor = look.rect.size.w / offer.panel.size.w.max(1.0);
        let mut elements = Vec::new();
        let selected = offer.candidates.get(offer.selected);
        let mut ring = None;
        for (surface, rect) in self.assist_slots(offer) {
            if Some(&surface) == selected {
                ring = Some(rect);
            }
            elements.extend(self.thumbnail_elements(renderer, &surface, rect, alpha, scale));
        }
        if let Some(rect) = ring {
            let around = Rectangle::new(
                rect.loc - Point::from((RING, RING)),
                (rect.size.w + 2.0 * RING, rect.size.h + 2.0 * RING).into(),
            );
            elements.push(OutputElement::Fill(FillElement::new(
                offer.ids[0].clone(),
                rounded(around.to_physical(scale), RING_RADIUS * scale),
                premultiplied(palette.accent_text, alpha),
            )));
        }
        let shape = rounded(look.rect.to_physical(scale), PANEL_RADIUS * factor * scale);
        elements.push(OutputElement::Fill(FillElement::new(
            offer.ids[1].clone(),
            shape,
            premultiplied(palette.overlay, alpha),
        )));
        let shading = Shading {
            active: true,
            scheme: self.decorations.scheme,
        };
        elements.extend(cast_shadows(
            &offer.shadow,
            shape,
            Elevation::Overlay,
            shading,
            scale,
            alpha,
        ));
        elements
    }
}

impl Compositor {
    pub fn other_half_offered(&self) -> bool {
        self.state.assist.is_open()
    }
}
