#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    pub fn area(&self) -> u64 {
        if self.is_empty() {
            0
        } else {
            u64::from(self.width.unsigned_abs()) * u64::from(self.height.unsigned_abs())
        }
    }

    pub fn translated(&self, dx: i32, dy: i32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.width, self.height)
    }

    pub fn intersection(&self, other: &Self) -> Option<Self> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = (self.x + self.width).min(other.x + other.width);
        let bottom = (self.y + self.height).min(other.y + other.height);
        let overlap = Self::new(left, top, right - left, bottom - top);
        (!overlap.is_empty()).then_some(overlap)
    }

    fn bounding(&self, other: &Self) -> Self {
        let left = self.x.min(other.x);
        let top = self.y.min(other.y);
        let right = (self.x + self.width).max(other.x + other.width);
        let bottom = (self.y + self.height).max(other.y + other.height);
        Self::new(left, top, right - left, bottom - top)
    }
}

pub fn disjoint(rects: impl IntoIterator<Item = Rect>, bounds: Rect) -> Vec<Rect> {
    let mut merged: Vec<Rect> = Vec::new();
    for rect in rects
        .into_iter()
        .filter_map(|rect| rect.intersection(&bounds))
    {
        let mut rect = rect;
        while let Some(index) = merged
            .iter()
            .position(|other| other.intersection(&rect).is_some())
        {
            rect = rect.bounding(&merged.swap_remove(index));
        }
        merged.push(rect);
    }
    merged
}
