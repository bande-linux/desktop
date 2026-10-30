use std::hash::{DefaultHasher, Hash, Hasher};

use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, WaylandSurfaceTexture, render_elements_from_surface_tree,
};
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::utils::{Buffer as BufferCoords, Physical, Point, Rectangle, Scale};

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::Logical;

use crate::effect::blur_rects;
use crate::motion::{Look, Motion};
use crate::state::OutputElement;
use crate::state::mapped_size;
use crate::tile::Preview;
use crate::window::Window;
use unbound_render::{
    Blur, RenderError, RoundedRect, Shadow, VulkanFrame, VulkanRenderer, VulkanTexture,
};

pub(crate) const CORNER_RADIUS: f64 = 12.0;

struct ShadowStyle {
    blur: f64,
    offset: f64,
    opacity: f32,
}

const ACTIVE_SHADOW: ShadowStyle = ShadowStyle {
    blur: 14.0,
    offset: 10.0,
    opacity: 0.42,
};

const INACTIVE_SHADOW: ShadowStyle = ShadowStyle {
    blur: 7.0,
    offset: 4.0,
    opacity: 0.22,
};

pub(crate) type Area = Rectangle<f64, Physical>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Placement {
    pub(crate) from: Area,
    pub(crate) to: Area,
}

impl Placement {
    fn is_fixed(&self) -> bool {
        self.from == self.to
    }

    fn factor(&self) -> Scale<f64> {
        Scale::from((
            self.to.size.w / self.from.size.w.max(1.0),
            self.to.size.h / self.from.size.h.max(1.0),
        ))
    }

    fn map(&self, rect: Rectangle<i32, Physical>) -> Rectangle<i32, Physical> {
        if self.is_fixed() {
            return rect;
        }
        self.map_area(rect.to_f64()).to_i32_round()
    }

    fn map_area(&self, rect: Area) -> Area {
        let factor = self.factor();
        let relative = rect.loc - self.from.loc;
        let loc = self.to.loc + Point::from((relative.x * factor.x, relative.y * factor.y));
        Rectangle::new(loc, rect.size.upscale(factor))
    }
}

pub(crate) fn rounded(area: Area, radius: f64) -> RoundedRect {
    RoundedRect {
        x: area.loc.x as f32,
        y: area.loc.y as f32,
        width: area.size.w as f32,
        height: area.size.h as f32,
        radius: radius as f32,
    }
}

fn rounded_distance(x: f64, y: f64, clip: &RoundedRect) -> f64 {
    let (half_w, half_h) = (f64::from(clip.width) / 2.0, f64::from(clip.height) / 2.0);
    let corner = f64::from(clip.radius).min(half_w).min(half_h);
    let inset_x = (x - f64::from(clip.x) - half_w).abs() - half_w + corner;
    let inset_y = (y - f64::from(clip.y) - half_h).abs() - half_h + corner;
    inset_x.max(0.0).hypot(inset_y.max(0.0)) + inset_x.max(inset_y).min(0.0) - corner
}

const OPAQUE_DISTANCE: f64 = -0.5 - 1e-3;

fn fully_covered(x: i32, y: i32, clip: &RoundedRect) -> bool {
    rounded_distance(f64::from(x) + 0.5, f64::from(y) + 0.5, clip) <= OPAQUE_DISTANCE
}

pub(crate) fn uncovered_pixels(clip: &RoundedRect) -> Vec<Rectangle<i32, Physical>> {
    let left = f64::from(clip.x).floor() as i32;
    let top = f64::from(clip.y).floor() as i32;
    let right = f64::from(clip.x + clip.width).ceil() as i32;
    let bottom = f64::from(clip.y + clip.height).ceil() as i32;
    let band = f64::from(clip.radius).ceil() as i32 + 1;
    let rows = (top..(top + band).min(bottom))
        .chain((bottom - band).max(top + band)..bottom)
        .collect::<Vec<_>>();
    let mut uncovered: Vec<Rectangle<i32, Physical>> = Vec::new();
    let mut push = |rect: Rectangle<i32, Physical>| {
        if rect.size.w <= 0 {
            return;
        }
        match uncovered.last_mut() {
            Some(last)
                if last.loc.x == rect.loc.x
                    && last.size.w == rect.size.w
                    && last.loc.y + last.size.h == rect.loc.y =>
            {
                last.size.h += 1;
            }
            _ => uncovered.push(rect),
        }
    };
    for &y in &rows {
        let run = (left..right)
            .take_while(|&x| !fully_covered(x, y, clip))
            .count() as i32;
        push(Rectangle::new((left, y).into(), (run, 1).into()));
    }
    for &y in &rows {
        let run = (left..right)
            .rev()
            .take_while(|&x| !fully_covered(x, y, clip))
            .count() as i32;
        push(Rectangle::new((right - run, y).into(), (run, 1).into()));
    }
    let inner_left = f64::from(clip.x).ceil() as i32;
    let inner_right = f64::from(clip.x + clip.width).floor() as i32;
    for (x, width) in [
        (left, inner_left - left),
        (inner_right, right - inner_right),
    ] {
        push(Rectangle::new(
            (x, top).into(),
            (width, bottom - top).into(),
        ));
    }
    uncovered
}

pub struct WindowElement<E> {
    inner: E,
    placement: Placement,
    clip: RoundedRect,
}

impl<E: Element> WindowElement<E> {
    pub(crate) fn new(inner: E, placement: Placement, clip: RoundedRect) -> Self {
        Self {
            inner,
            placement,
            clip,
        }
    }
}

impl<E: Element> Element for WindowElement<E> {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn src(&self) -> Rectangle<f64, BufferCoords> {
        self.inner.src()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.placement.map(self.inner.geometry(scale))
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        if !self.placement.is_fixed() {
            return DamageSet::from_slice(&[Rectangle::from_size(self.geometry(scale).size)]);
        }
        self.inner.damage_since(scale, commit)
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        if !self.placement.is_fixed() {
            return OpaqueRegions::default();
        }
        let origin = self.inner.geometry(scale).loc;
        let corners = uncovered_pixels(&self.clip)
            .into_iter()
            .map(|rect| Rectangle::new(rect.loc - origin, rect.size));
        Rectangle::subtract_rects_many(self.inner.opaque_regions(scale), corners)
            .into_iter()
            .collect()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl<E: RenderElement<VulkanRenderer>> RenderElement<VulkanRenderer> for WindowElement<E> {
    fn draw(
        &self,
        frame: &mut VulkanFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), RenderError> {
        frame.clip_to(Some(self.clip));
        let drawn = self.inner.draw(frame, src, dst, damage, opaque_regions);
        frame.clip_to(None);
        drawn
    }
}

pub struct ShadowElement {
    id: Id,
    shadow: Shadow,
    commit: CommitCounter,
}

impl ShadowElement {
    pub(crate) fn new(id: Id, window: RoundedRect, active: bool, scale: f64, alpha: f32) -> Self {
        let style = if active {
            &ACTIVE_SHADOW
        } else {
            &INACTIVE_SHADOW
        };
        let shadow = Shadow {
            caster: RoundedRect {
                y: window.y + (style.offset * scale) as f32,
                ..window
            },
            window,
            blur: (style.blur * scale) as f32,
            color: [0.0, 0.0, 0.0, style.opacity * alpha],
        };
        let mut hasher = DefaultHasher::new();
        for value in [
            shadow.caster.components(),
            shadow.window.components(),
            [shadow.blur, shadow.window.radius, shadow.color[3], 0.0],
        ]
        .concat()
        {
            value.to_bits().hash(&mut hasher);
        }
        Self {
            id,
            shadow,
            commit: CommitCounter::from(hasher.finish() as usize),
        }
    }
}

impl Element for ShadowElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, BufferCoords> {
        let bounds = self.shadow.bounds();
        Rectangle::from_size((f64::from(bounds.width), f64::from(bounds.height)).into())
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        let bounds = self.shadow.bounds();
        Rectangle::new(
            (bounds.x, bounds.y).into(),
            (bounds.width, bounds.height).into(),
        )
    }
}

impl RenderElement<VulkanRenderer> for ShadowElement {
    fn draw(
        &self,
        frame: &mut VulkanFrame<'_, '_>,
        _src: Rectangle<f64, BufferCoords>,
        _dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), RenderError> {
        frame.draw_shadow(&self.shadow, damage)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Piece {
    pub(crate) id: Id,
    pub(crate) texture: VulkanTexture,
    pub(crate) src: Rectangle<f64, BufferCoords>,
    pub(crate) geometry: Rectangle<i32, Physical>,
}

pub struct PieceElement {
    piece: Piece,
    alpha: f32,
}

impl PieceElement {
    pub(crate) fn new(piece: Piece, alpha: f32) -> Self {
        Self { piece, alpha }
    }
}

impl Element for PieceElement {
    fn id(&self) -> &Id {
        &self.piece.id
    }

    fn current_commit(&self) -> CommitCounter {
        CommitCounter::default()
    }

    fn src(&self) -> Rectangle<f64, BufferCoords> {
        self.piece.src
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.piece.geometry
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }
}

impl RenderElement<VulkanRenderer> for PieceElement {
    fn draw(
        &self,
        frame: &mut VulkanFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), RenderError> {
        smithay::backend::renderer::Frame::render_texture_from_to(
            frame,
            &self.piece.texture,
            src,
            dst,
            damage,
            opaque_regions,
            smithay::utils::Transform::Normal,
            self.alpha,
        )
    }
}

const PREVIEW_FILL: [f32; 4] = [0.16, 0.16, 0.16, 0.16];
const HUNG_VEIL: [f32; 4] = [0.12, 0.12, 0.12, 0.24];
const BLUR_PASSES: u32 = 3;
const BLUR_OFFSET: f64 = 1.5;

pub struct BlurElement {
    id: Id,
    blur: Blur,
    params: u64,
    commit: CommitCounter,
}

impl BlurElement {
    pub(crate) fn new(id: Id, region: RoundedRect, scale: f64, alpha: f32) -> Self {
        let blur = Blur {
            region,
            passes: BLUR_PASSES,
            offset: (BLUR_OFFSET * scale) as f32,
            alpha,
        };
        let mut hasher = DefaultHasher::new();
        for value in [
            region.components(),
            [region.radius, blur.offset, alpha, 0.0],
        ]
        .concat()
        {
            value.to_bits().hash(&mut hasher);
        }
        let params = hasher.finish();
        Self {
            id,
            blur,
            params,
            commit: CommitCounter::from(params as usize),
        }
    }

    pub(crate) fn depend_on(&mut self, below: u64) {
        self.commit = CommitCounter::from((self.params ^ below) as usize);
    }
}

impl Element for BlurElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, BufferCoords> {
        let size = self.geometry(Scale::from(1.0)).size;
        Rectangle::from_size((f64::from(size.w), f64::from(size.h)).into())
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        let bounds = self.blur.bounds();
        Rectangle::new(
            (bounds.x, bounds.y).into(),
            (bounds.width, bounds.height).into(),
        )
    }
}

impl RenderElement<VulkanRenderer> for BlurElement {
    fn draw(
        &self,
        frame: &mut VulkanFrame<'_, '_>,
        _src: Rectangle<f64, BufferCoords>,
        _dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), RenderError> {
        frame.draw_blur(&self.blur, damage)
    }
}

pub(crate) fn surface_blurs(
    surface: &WlSurface,
    location: Point<i32, Logical>,
    scale: f64,
) -> Vec<OutputElement> {
    let Some(size) = mapped_size(surface) else {
        return Vec::new();
    };
    let bounds = Rectangle::new(location, size);
    blur_rects(surface)
        .into_iter()
        .filter_map(|(id, rect)| {
            let rect = Rectangle::new(location + rect.loc, rect.size).intersection(bounds)?;
            Some(OutputElement::Blur(BlurElement::new(
                id,
                rounded(rect.to_f64().to_physical(scale), 0.0),
                scale,
                1.0,
            )))
        })
        .collect()
}

pub(crate) fn tie_blurs_to_what_lies_below(elements: &mut [OutputElement], scale: f64) {
    let scale = Scale::from(scale);
    for index in 0..elements.len() {
        if !matches!(elements[index], OutputElement::Blur(_)) {
            continue;
        }
        let reach = elements[index].geometry(scale);
        let mut hasher = DefaultHasher::new();
        for below in &elements[index + 1..] {
            let geometry = below.geometry(scale);
            if !geometry.overlaps(reach) {
                continue;
            }
            below.id().hash(&mut hasher);
            format!("{:?}", below.current_commit()).hash(&mut hasher);
            (
                geometry.loc.x,
                geometry.loc.y,
                geometry.size.w,
                geometry.size.h,
            )
                .hash(&mut hasher);
            below.alpha().to_bits().hash(&mut hasher);
        }
        let below = hasher.finish();
        if let OutputElement::Blur(blur) = &mut elements[index] {
            blur.depend_on(below);
        }
    }
}

pub struct FillElement {
    id: Id,
    shape: RoundedRect,
    color: [f32; 4],
    commit: CommitCounter,
}

impl FillElement {
    fn new(id: Id, shape: RoundedRect, color: [f32; 4]) -> Self {
        let mut hasher = DefaultHasher::new();
        for value in [shape.components(), color, [shape.radius, 0.0, 0.0, 0.0]].concat() {
            value.to_bits().hash(&mut hasher);
        }
        Self {
            id,
            shape,
            color,
            commit: CommitCounter::from(hasher.finish() as usize),
        }
    }
}

impl Element for FillElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, BufferCoords> {
        let size = self.geometry(Scale::from(1.0)).size;
        Rectangle::from_size((f64::from(size.w), f64::from(size.h)).into())
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        let left = f64::from(self.shape.x).floor();
        let top = f64::from(self.shape.y).floor();
        let right = f64::from(self.shape.x + self.shape.width).ceil();
        let bottom = f64::from(self.shape.y + self.shape.height).ceil();
        Rectangle::new(
            (left as i32, top as i32).into(),
            ((right - left) as i32, (bottom - top) as i32).into(),
        )
    }
}

impl RenderElement<VulkanRenderer> for FillElement {
    fn draw(
        &self,
        frame: &mut VulkanFrame<'_, '_>,
        _src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), RenderError> {
        let [red, green, blue, alpha] = self.color;
        frame.clip_to(Some(self.shape));
        let drawn = smithay::backend::renderer::Frame::draw_solid(
            frame,
            dst,
            damage,
            smithay::backend::renderer::Color32F::new(red, green, blue, alpha),
        );
        frame.clip_to(None);
        drawn
    }
}

pub(crate) fn preview_elements(preview: &Preview, scale: f64) -> Vec<OutputElement> {
    let Some(look) = preview.motion.shown() else {
        return Vec::new();
    };
    if look.alpha <= 0.0 {
        return Vec::new();
    }
    let alpha = look.alpha as f32;
    let shape = rounded(look.rect.to_physical(scale), CORNER_RADIUS * scale);
    vec![
        OutputElement::Fill(FillElement::new(
            preview.fill.clone(),
            shape,
            PREVIEW_FILL.map(|channel| channel * alpha),
        )),
        OutputElement::Shadow(ShadowElement::new(
            preview.shadow.clone(),
            shape,
            false,
            scale,
            alpha,
        )),
    ]
}

#[derive(Debug, Clone)]
pub(crate) struct Drawn {
    pieces: Vec<Piece>,
    area: Area,
    pub(crate) look: Look,
    active: bool,
}

pub(crate) struct Closing {
    drawn: Drawn,
    shadow: Id,
    pub(crate) motion: Motion,
}

impl Closing {
    pub(crate) fn new(mut drawn: Drawn, motion: Motion) -> Self {
        for piece in &mut drawn.pieces {
            piece.id = Id::new();
        }
        Self {
            drawn,
            shadow: Id::new(),
            motion,
        }
    }

    pub(crate) fn render_elements(&self, scale: f64) -> Vec<OutputElement> {
        let Some(look) = self.motion.shown() else {
            return Vec::new();
        };
        if look.alpha <= 0.0 {
            return Vec::new();
        }
        let shown = look.rect.to_physical(scale);
        let clip = rounded(shown, shaped_radius(self.drawn.area, shown, scale));
        let placement = Placement {
            from: self.drawn.area,
            to: shown,
        };
        let alpha = look.alpha as f32;
        self.drawn
            .pieces
            .iter()
            .map(|piece| {
                OutputElement::Closing(WindowElement::new(
                    PieceElement::new(piece.clone(), alpha),
                    placement,
                    clip,
                ))
            })
            .chain([OutputElement::Shadow(ShadowElement::new(
                self.shadow.clone(),
                clip,
                self.drawn.active,
                scale,
                alpha,
            ))])
            .collect()
    }
}

fn shaped_radius(area: Area, shown: Area, scale: f64) -> f64 {
    let shrink = (shown.size.w / area.size.w.max(1.0))
        .min(shown.size.h / area.size.h.max(1.0))
        .min(1.0);
    CORNER_RADIUS * scale * shrink
}

pub(crate) fn window_elements(
    window: &Window,
    renderer: &mut VulkanRenderer,
    scale: f64,
    active: bool,
    hung: bool,
) -> (Vec<OutputElement>, Option<Drawn>) {
    let look = window.shown_look();
    let alpha = look.map_or(1.0, |look| look.alpha) as f32;
    if alpha <= 0.0 {
        return (Vec::new(), None);
    }
    let origin = window
        .surface_origin()
        .to_f64()
        .to_physical(scale)
        .to_i32_round::<i32>();
    let surfaces: Vec<WaylandSurfaceRenderElement<VulkanRenderer>> =
        render_elements_from_surface_tree(
            renderer,
            window.surface(),
            origin,
            scale,
            alpha,
            Kind::Unspecified,
        );
    let fullscreen = window.has_state(xdg_toplevel::State::Fullscreen);
    let Some(geometry) = window.geometry().filter(|_| !fullscreen || look.is_some()) else {
        return (
            surfaces.into_iter().map(OutputElement::Surface).collect(),
            None,
        );
    };
    let real = Rectangle::new(geometry.location.to_f64(), geometry.size.to_f64());
    let area = real.to_physical(scale);
    let look = look.unwrap_or(Look::shown(real));
    let shown = look.rect.to_physical(scale);
    let radius = if fullscreen {
        0.0
    } else {
        shaped_radius(area, shown, scale)
    };
    let clip = rounded(shown, radius);
    let placement = Placement {
        from: area,
        to: shown,
    };
    let pieces = surfaces
        .iter()
        .filter_map(|surface| match surface.texture() {
            WaylandSurfaceTexture::Texture(texture) => Some(Piece {
                id: surface.id().clone(),
                texture: texture.clone(),
                src: surface.src(),
                geometry: surface.geometry(Scale::from(scale)),
            }),
            WaylandSurfaceTexture::SolidColor(_) => None,
        })
        .collect();
    let shadow = (!fullscreen).then(|| {
        OutputElement::Shadow(ShadowElement::new(
            window.shadow.clone(),
            clip,
            active,
            scale,
            alpha,
        ))
    });
    let origin = window.surface_origin().to_f64();
    let blurs = blur_rects(window.surface())
        .into_iter()
        .filter_map(|(id, rect)| {
            let rect = Rectangle::new(origin + rect.loc.to_f64(), rect.size.to_f64())
                .to_physical(scale)
                .intersection(area)?;
            let mapped = placement.map_area(rect);
            Some(OutputElement::Blur(BlurElement::new(
                id,
                rounded(mapped, radius),
                scale,
                alpha,
            )))
        });
    let veil = hung.then(|| {
        OutputElement::Fill(FillElement::new(
            window.veil.clone(),
            clip,
            HUNG_VEIL.map(|channel| channel * alpha),
        ))
    });
    let elements = veil
        .into_iter()
        .chain(
            surfaces
                .into_iter()
                .map(|surface| OutputElement::Window(WindowElement::new(surface, placement, clip))),
        )
        .chain(shadow)
        .chain(blurs)
        .collect();
    (
        elements,
        Some(Drawn {
            pieces,
            area,
            look,
            active,
        }),
    )
}
