use std::hash::{DefaultHasher, Hash, Hasher};

use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::{Buffer as BufferCoords, Physical, Point, Rectangle, Scale};
use unbound_render::{RenderError, RoundedRect, Shadow, VulkanFrame, VulkanRenderer};

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
    pub(crate) fn fixed(area: Area) -> Self {
        Self {
            from: area,
            to: area,
        }
    }

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
        let factor = self.factor();
        let relative = rect.loc.to_f64() - self.from.loc;
        let loc = self.to.loc + Point::from((relative.x * factor.x, relative.y * factor.y));
        let size = rect.size.to_f64().upscale(factor);
        Rectangle::new(loc, size).to_i32_round()
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
