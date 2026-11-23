use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

use bande_render::{RenderError, VulkanFrame, VulkanRenderer, VulkanTexture};
use smithay::backend::renderer::Texture;
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, WaylandSurfaceTexture,
};
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::{Buffer as BufferCoords, Physical, Point, Rectangle, Scale, Transform};

use crate::state::OutputElement;

const REMEMBERED_FAILURES: usize = 256;

static FAILED: LazyLock<Mutex<HashSet<Id>>> = LazyLock::new(Mutex::default);
static LEFT_OUT: AtomicBool = AtomicBool::new(false);

pub(crate) fn drawing_everything<T>(draw: impl FnOnce() -> T) -> (T, bool) {
    LEFT_OUT.store(false, Ordering::Relaxed);
    let drawn = draw();
    (drawn, !LEFT_OUT.swap(false, Ordering::Relaxed))
}

fn skip_failed(id: &Id, error: RenderError) -> Result<(), RenderError> {
    if error.is_device_lost() {
        return Err(error);
    }
    LEFT_OUT.store(true, Ordering::Relaxed);
    if let Ok(mut failed) = FAILED.lock() {
        if failed.len() >= REMEMBERED_FAILURES {
            failed.clear();
        }
        if failed.insert(id.clone()) {
            tracing::warn!("leaving out an element this renderer cannot draw: {error}");
        }
    }
    Ok(())
}

pub struct ScreenElement {
    inner: OutputElement,
    by: Point<i32, Physical>,
    opaque_inset: i32,
}

impl ScreenElement {
    pub(crate) fn new(inner: OutputElement, by: Point<i32, Physical>) -> Self {
        Self {
            inner,
            by,
            opaque_inset: 0,
        }
    }
}

fn eroded(
    regions: OpaqueRegions<i32, Physical>,
    inset: i32,
) -> impl Iterator<Item = Rectangle<i32, Physical>> {
    regions.into_iter().filter_map(move |region| {
        let size = (region.size.w - 2 * inset, region.size.h - 2 * inset);
        (size.0 > 0 && size.1 > 0).then(|| {
            Rectangle::new(
                (region.loc.x + inset, region.loc.y + inset).into(),
                size.into(),
            )
        })
    })
}

pub(crate) fn keep_backdrops_whole(
    elements: &mut [ScreenElement],
    scale: f64,
    output: Rectangle<i32, Physical>,
) {
    let scale = Scale::from(scale);
    for index in 0..elements.len() {
        let reach = match &elements[index].inner {
            OutputElement::Blur(blur) => blur.reach(),
            _ => continue,
        };
        let Some(backdrop) = elements[index].geometry(scale).intersection(output) else {
            continue;
        };
        let mut covered = Vec::new();
        for above in &mut elements[..index] {
            if !above.geometry(scale).overlaps(backdrop) {
                continue;
            }
            above.opaque_inset = above.opaque_inset.max(reach);
            let at = above.location(scale);
            covered.extend(eroded(above.inner.opaque_regions(scale), reach).filter_map(
                |mut region| {
                    region.loc += at;
                    region.intersection(backdrop)
                },
            ));
        }
        let mut hasher = DefaultHasher::new();
        for rect in &covered {
            (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h).hash(&mut hasher);
        }
        let uncovered = Rectangle::subtract_rects_many([backdrop], covered)
            .iter()
            .map(|rect| {
                u64::from(rect.size.w.unsigned_abs()) * u64::from(rect.size.h.unsigned_abs())
            })
            .sum();
        if let OutputElement::Blur(blur) = &mut elements[index].inner {
            blur.cover(hasher.finish(), uncovered);
        }
    }
}

impl Element for ScreenElement {
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
        let mut geometry = self.inner.geometry(scale);
        geometry.loc += self.by;
        geometry
    }

    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.inner.location(scale) + self.by
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        match self.opaque_inset {
            0 => self.inner.opaque_regions(scale),
            inset => eroded(self.inner.opaque_regions(scale), inset).collect(),
        }
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl RenderElement<VulkanRenderer> for ScreenElement {
    fn draw(
        &self,
        frame: &mut VulkanFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), RenderError> {
        frame.move_origin((self.by.x, self.by.y));
        let drawn = self.inner.draw(frame, src, dst, damage, opaque_regions);
        frame.move_origin((0, 0));
        drawn.or_else(|error| skip_failed(self.id(), error))
    }

    fn underlying_storage(&self, renderer: &mut VulkanRenderer) -> Option<UnderlyingStorage<'_>> {
        self.inner.underlying_storage(renderer)
    }
}

pub struct SurfaceElement(WaylandSurfaceRenderElement<VulkanRenderer>);

impl From<WaylandSurfaceRenderElement<VulkanRenderer>> for SurfaceElement {
    fn from(surface: WaylandSurfaceRenderElement<VulkanRenderer>) -> Self {
        Self(surface)
    }
}

impl From<WaylandSurfaceRenderElement<VulkanRenderer>> for OutputElement {
    fn from(surface: WaylandSurfaceRenderElement<VulkanRenderer>) -> Self {
        Self::Surface(surface.into())
    }
}

impl SurfaceElement {
    pub(crate) fn texture(&self) -> Option<&VulkanTexture> {
        match self.0.texture() {
            WaylandSurfaceTexture::Texture(texture) => Some(texture),
            WaylandSurfaceTexture::SolidColor(_) => None,
        }
    }
}

impl Element for SurfaceElement {
    fn id(&self) -> &Id {
        self.0.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.0.current_commit()
    }

    fn src(&self) -> Rectangle<f64, BufferCoords> {
        let src = self.0.src();
        match (self.0.transform(), self.texture()) {
            (
                Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270,
                Some(texture),
            ) => Transform::_180.transform_rect_in(src, &texture.size().to_f64()),
            _ => src,
        }
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.0.geometry(scale)
    }

    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.0.location(scale)
    }

    fn transform(&self) -> Transform {
        self.0.transform()
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        let damage = self.0.damage_since(scale, commit);
        match self.0.transform() {
            Transform::_90 | Transform::_270 if !damage.is_empty() => {
                DamageSet::from_slice(&[Rectangle::from_size(self.geometry(scale).size)])
            }
            _ => damage,
        }
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.0.opaque_regions(scale)
    }

    fn alpha(&self) -> f32 {
        self.0.alpha()
    }

    fn kind(&self) -> Kind {
        self.0.kind()
    }
}

impl RenderElement<VulkanRenderer> for SurfaceElement {
    fn draw(
        &self,
        frame: &mut VulkanFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), RenderError> {
        self.0.draw(frame, src, dst, damage, opaque_regions)
    }

    fn underlying_storage(&self, renderer: &mut VulkanRenderer) -> Option<UnderlyingStorage<'_>> {
        self.0.underlying_storage(renderer)
    }
}
