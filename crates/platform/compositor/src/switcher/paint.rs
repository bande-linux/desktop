use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::element::{Id, Kind};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Rectangle, Size};
use unbound_graphics::{
    Color, Elevation, Item, Paint, Palette, Radius, Rect, Scene, Shape, TextAlign, TextRole,
    TextStyle,
};
use unbound_render::VulkanRenderer;
use unbound_spring::Spring;

use super::layout::fitted;
use super::{App, Card, Content, Frame, NAME_ROW, OPEN_SCALE, Painted, Switcher};
use crate::label::Labels;
use crate::motion::{Look, Motion};
use crate::overview::real_rect;
use crate::scene::{
    Casting, FillElement, Shading, cast_shadows, fresh, pieces_of, premultiplied, rounded,
    scaled_pieces,
};
use crate::state::{OutputElement, State};

const CARD_RADIUS: f64 = Radius::XL as f64;
const TILE_RADIUS: f64 = Radius::L as f64;
const THUMBNAIL_RADIUS: f64 = Radius::S as f64;
const ICON_ROUNDING: f32 = 0.225;
const INITIAL_SHARE: f32 = 0.45;
const MINIMIZED_ALPHA: f32 = 0.5;

pub(super) fn placeholder(
    labels: &mut Labels,
    app: &App,
    rect: Rect,
    palette: Palette,
) -> Vec<Item> {
    let initial = app
        .name
        .chars()
        .next()
        .map(|character| character.to_uppercase().collect::<String>())
        .unwrap_or_default();
    let paragraph = labels.text().paragraph(
        &initial,
        TextStyle::new(rect.height * INITIAL_SHARE, palette.overlay).weight(TextStyle::SEMIBOLD),
        Some(rect.width),
        TextAlign::Center,
    );
    let top = rect.y + (rect.height - paragraph.height()) / 2.0;
    vec![
        Item::Fill {
            shape: Shape::rounded(rect, rect.width * ICON_ROUNDING),
            paint: Paint::Solid(palette.ink2),
        },
        Item::Text {
            origin: unbound_graphics::Point::new(rect.x, top),
            paragraph,
        },
    ]
}

impl Switcher {
    fn paint_icons(&mut self, labels: &mut Labels, palette: Palette, scale: f64) {
        let Some(card) = self.card.as_ref() else {
            return;
        };
        let Content::Icons(apps) = card.content.clone() else {
            return;
        };
        let text = apps
            .iter()
            .map(|app| app.id.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if card
            .icons
            .as_ref()
            .is_some_and(|painted| painted.scale == scale && painted.text == text)
        {
            return;
        }
        let layout = card.layout.clone();
        let origin = layout.card.loc;
        let mut items = Vec::new();
        let mut wanted = Vec::new();
        for (app, content) in apps.iter().zip(&layout.contents) {
            let rect = Rect::new(
                (content.loc.x - origin.x) as f32,
                (content.loc.y - origin.y) as f32,
                content.size.w as f32,
                content.size.h as f32,
            );
            let pixels = (content.size.w * scale).ceil() as u32;
            match app
                .icon
                .as_deref()
                .and_then(|icon| self.image(icon, pixels, &mut wanted))
            {
                Some(image) => items.push(Item::Image { rect, image }),
                None => items.extend(placeholder(labels, app, rect, palette)),
            }
        }
        self.load_icons(wanted);
        let scene = Scene {
            width: layout.card.size.w as u32,
            height: layout.card.size.h as u32,
            background: Color::TRANSPARENT,
            items,
        };
        let buffer = labels.paint(&scene, scale);
        if let Some(card) = self.card.as_mut() {
            card.icons = buffer.map(|buffer| Painted {
                buffer,
                scale,
                text,
            });
        }
    }

    fn paint_name(&mut self, labels: &mut Labels, text: String, palette: Palette, scale: f64) {
        let Some(card) = self.card.as_ref() else {
            return;
        };
        if card
            .name
            .as_ref()
            .is_some_and(|painted| painted.scale == scale && painted.text == text)
        {
            return;
        }
        let row = card.layout.name;
        let paragraph = labels.text().paragraph(
            &text,
            TextStyle::role(TextRole::BodyStrong, palette.ink),
            Some(row.size.w as f32),
            TextAlign::Center,
        );
        let top = ((NAME_ROW as f32 - paragraph.height()) / 2.0).max(0.0);
        let scene = Scene {
            width: row.size.w as u32,
            height: row.size.h as u32,
            background: Color::TRANSPARENT,
            items: vec![Item::Clip {
                shape: Shape::rect(Rect::new(0.0, 0.0, row.size.w as f32, row.size.h as f32)),
                items: vec![Item::Text {
                    origin: unbound_graphics::Point::new(0.0, top),
                    paragraph,
                }],
            }],
        };
        let buffer = labels.paint(&scene, scale);
        if let Some(card) = self.card.as_mut() {
            card.name = buffer.map(|buffer| Painted {
                buffer,
                scale,
                text,
            });
        }
    }
}

impl State {
    pub(super) fn refresh_card(&mut self) {
        self.refresh_card_look();
        self.announce_switcher();
    }

    fn refresh_card_look(&mut self) {
        self.needs_redraw = true;
        let Some(session) = self.switcher.session.as_ref() else {
            return;
        };
        if !session.shown {
            return;
        }
        let content = self.switcher_content(session);
        let layout = self.switcher_layout(session, &content);
        let selected = layout.tiles.get(session.selected).copied();
        let screen = session.screen;
        let reduce = self.reduce_motion;
        let spring = if reduce { Spring::FADE } else { Spring::QUICK };
        match self.switcher.card.as_mut().filter(|card| !card.closing) {
            Some(card) => {
                let rebuilt = card.content != content || card.layout != layout;
                card.look.retarget(Look::shown(layout.card));
                if let Some(tile) = selected {
                    if rebuilt || reduce {
                        card.ring = Motion::new(spring, Look::shown(tile), Look::shown(tile));
                    } else {
                        card.ring.retarget(Look::shown(tile));
                    }
                }
                if rebuilt {
                    card.icons = None;
                }
                card.layout = layout;
                card.content = content;
            }
            None => {
                let from = if reduce {
                    Look::hidden(layout.card)
                } else {
                    Look::hidden(layout.card).scaled(OPEN_SCALE)
                };
                let tile = selected.unwrap_or(layout.card);
                self.switcher.card = Some(Card {
                    screen,
                    look: Motion::new(spring, from, Look::shown(layout.card)),
                    ring: Motion::new(spring, Look::shown(tile), Look::shown(tile)),
                    layout,
                    content,
                    closing: false,
                    icons: None,
                    name: None,
                    ids: [Id::new(), Id::new()],
                    shadow: Casting::new(),
                });
            }
        }
    }

    pub(crate) fn thumbnail_elements(
        &self,
        renderer: &mut VulkanRenderer,
        surface: &WlSurface,
        to: Frame,
        alpha: f32,
        scale: f64,
    ) -> Vec<OutputElement> {
        let Some(window) = self.window_index(surface).map(|index| &self.windows[index]) else {
            return Vec::new();
        };
        let Some(rect) = real_rect(window) else {
            return Vec::new();
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
        let mut pieces = pieces_of(&surfaces, scale);
        pieces.extend(window.title_bar_piece(scale));
        let small = to.to_physical(scale);
        let alpha = if window.is_minimized() || window.hidden {
            alpha * MINIMIZED_ALPHA
        } else {
            alpha
        };
        scaled_pieces(
            &fresh(pieces),
            rect.to_physical(scale),
            small,
            rounded(small, THUMBNAIL_RADIUS * scale),
            alpha,
        )
        .collect()
    }

    pub(crate) fn prepare_switcher(&mut self, screen: usize) {
        if !self
            .switcher
            .card
            .as_ref()
            .is_some_and(|card| card.screen == screen)
        {
            return;
        }
        let scale = self.screens[screen].scale();
        let palette = Palette::for_scheme(self.decorations.scheme);
        let name = self.selected_name();
        let labels = self.labels.get_mut();
        self.switcher.paint_icons(labels, palette, scale);
        self.switcher.paint_name(labels, name, palette, scale);
    }

    pub(crate) fn switcher_elements(
        &self,
        renderer: &mut VulkanRenderer,
        screen: usize,
        scale: f64,
    ) -> Vec<OutputElement> {
        let Some(card) = self
            .switcher
            .card
            .as_ref()
            .filter(|card| card.screen == screen && !self.is_locked())
        else {
            return Vec::new();
        };
        let look = card.look.look();
        let alpha = look.alpha as f32;
        if alpha <= 0.0 {
            return Vec::new();
        }
        let palette = Palette::for_scheme(self.decorations.scheme);
        let fill = |color: Color| premultiplied(color, alpha);
        let layout = &card.layout;
        let place = |rect: Frame| fitted(rect, &layout.card, &look.rect);
        let factor = look.rect.size.w / layout.card.size.w.max(1.0);
        let mut elements = Vec::new();
        let mut painted = |painted: &Option<Painted>, rect: Frame| {
            let painted = painted.as_ref().filter(|painted| painted.scale == scale)?;
            let shown = place(rect);
            MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                shown.loc.to_physical(scale).to_i32_round::<i32>().to_f64(),
                &painted.buffer,
                Some(alpha),
                Some(Rectangle::from_size(Size::from((
                    f64::from(unbound_graphics::pixels(rect.size.w as u32, scale)),
                    f64::from(unbound_graphics::pixels(rect.size.h as u32, scale)),
                )))),
                Some(shown.size.to_i32_round()),
                Kind::Unspecified,
            )
            .ok()
            .map(OutputElement::Memory)
        };
        elements.extend(painted(&card.name, layout.name));
        match &card.content {
            Content::Icons(_) => elements.extend(painted(&card.icons, layout.card)),
            Content::Windows(windows) => {
                for (surface, content) in windows.iter().zip(&layout.contents) {
                    elements.extend(self.thumbnail_elements(
                        renderer,
                        surface,
                        place(*content),
                        alpha,
                        scale,
                    ));
                }
            }
        }
        let ring = place(card.ring.look().rect);
        elements.push(OutputElement::Fill(FillElement::new(
            card.ids[0].clone(),
            rounded(ring.to_physical(scale), TILE_RADIUS * factor * scale),
            fill(palette.hairline),
        )));
        let shape = rounded(look.rect.to_physical(scale), CARD_RADIUS * factor * scale);
        elements.push(OutputElement::Fill(FillElement::new(
            card.ids[1].clone(),
            shape,
            fill(palette.overlay),
        )));
        let shading = Shading {
            active: true,
            scheme: self.decorations.scheme,
        };
        elements.extend(cast_shadows(
            &card.shadow,
            shape,
            Elevation::Overlay,
            shading,
            scale,
            alpha,
        ));
        elements
    }
}
