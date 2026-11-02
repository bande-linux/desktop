use std::collections::HashMap;
use std::os::fd::BorrowedFd;
use std::path::Path;

use bande_content::pdf::protocol::{MAX_RENDER_PIXELS, Render, Use};
use bande_content::pdf::session::{Change, Phase, Session};
use bande_toolkit::Image;

pub const GAP: f32 = 12.0;
const MAX_SIDE: f32 = 8192.0;

pub struct PdfView {
    session: Session,
    pub pages: Vec<(f32, f32)>,
    pub rendered: HashMap<usize, (f32, Image)>,
    pub thumbnails: HashMap<usize, Image>,
    tops: Vec<f32>,
    width: f32,
}

pub fn render(page: usize, size: (f32, f32), scale: f32, purpose: Use) -> Render {
    let fit = (MAX_SIDE / (size.0.max(size.1) * scale))
        .min(((MAX_RENDER_PIXELS as f32) / (size.0 * size.1 * scale * scale)).sqrt())
        .min(1.0);
    let scale = scale * fit;
    Render {
        page,
        scale,
        x: 0,
        y: 0,
        width: (size.0 * scale).ceil().max(1.0) as u32,
        height: (size.1 * scale).ceil().max(1.0) as u32,
        purpose,
    }
}

impl PdfView {
    pub fn open(path: &Path) -> Option<Self> {
        let session = Session::open(path)
            .inspect_err(|error| tracing::warn!("cannot start the PDF renderer: {error}"))
            .ok()?;
        Some(Self {
            session,
            pages: Vec::new(),
            rendered: HashMap::new(),
            thumbnails: HashMap::new(),
            tops: Vec::new(),
            width: 0.0,
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.session.fd()
    }

    pub fn phase(&self) -> Phase {
        self.session.phase()
    }

    pub fn is_locked(&self) -> bool {
        matches!(self.session.phase(), Phase::Locked { .. })
    }

    pub fn unlock(&mut self, password: &str) {
        self.session.unlock(password);
    }

    pub fn take(&mut self) -> bool {
        let mut changed = false;
        for change in self.session.take() {
            match change {
                Change::Opened => {
                    self.pages = self.session.opened().pages.clone();
                    self.width = 0.0;
                }
                Change::Pixels(render, pixels) => {
                    let Some(image) =
                        Image::from_premultiplied(render.width, render.height, pixels)
                    else {
                        continue;
                    };
                    match render.purpose {
                        Use::Thumbnail => {
                            self.thumbnails.insert(render.page, image);
                        }
                        _ => {
                            self.rendered.insert(render.page, (render.scale, image));
                        }
                    }
                }
                Change::Locked | Change::Failed | Change::Text(..) => {}
            }
            changed = true;
        }
        changed
    }

    pub fn lay_out(&mut self, width: f32) -> f32 {
        if (width - self.width).abs() > 0.5 || self.tops.len() != self.pages.len() {
            self.width = width;
            let mut top = 0.0;
            self.tops = self
                .pages
                .iter()
                .map(|(page_width, page_height)| {
                    let at = top;
                    top += page_height * width / page_width.max(1.0) + GAP;
                    at
                })
                .collect();
        }
        self.height()
    }

    pub fn height(&self) -> f32 {
        match (self.tops.last(), self.pages.last()) {
            (Some(top), Some((width, height))) => top + height * self.width / width.max(1.0),
            _ => 0.0,
        }
    }

    pub fn page_rect(&self, page: usize) -> Option<(f32, f32)> {
        let top = *self.tops.get(page)?;
        let (width, height) = self.pages.get(page)?;
        Some((top, height * self.width / width.max(1.0)))
    }

    pub fn top_of(&self, page: usize) -> f32 {
        self.tops.get(page).copied().unwrap_or(0.0)
    }

    pub fn page_at(&self, offset: f32) -> usize {
        self.tops
            .partition_point(|top| *top <= offset + 1.0)
            .saturating_sub(1)
    }

    pub fn want(&mut self, renders: Vec<Render>) {
        let missing: Vec<Render> = renders
            .into_iter()
            .filter(|render| match render.purpose {
                Use::Thumbnail => !self.thumbnails.contains_key(&render.page),
                _ => self
                    .rendered
                    .get(&render.page)
                    .is_none_or(|(scale, _)| (scale - render.scale).abs() > 0.01),
            })
            .collect();
        self.session.want(missing);
    }
}
