use std::ops::RangeInclusive;
use std::time::Instant;

use bande_spring::Spring;

use super::Animated;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReorderSlot {
    pub x: f32,
    pub width: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lifted {
    pub index: usize,
    pub start: usize,
    pub grab: f32,
    pub left: f32,
}

#[derive(Default)]
pub struct Reorder {
    slides: Vec<Animated>,
    lifted: Option<Lifted>,
    landing: Option<usize>,
}

impl Reorder {
    pub fn new(count: usize) -> Self {
        Self {
            slides: vec![Animated::points(0.0); count],
            lifted: None,
            landing: None,
        }
    }

    pub fn rearrange(&mut self, sources: impl IntoIterator<Item = Option<usize>>) {
        self.slides = sources
            .into_iter()
            .map(|source| {
                source
                    .and_then(|index| self.slides.get(index).copied())
                    .unwrap_or_else(|| Animated::points(0.0))
            })
            .collect();
        self.landing = None;
    }

    pub fn lifted(&self) -> Option<Lifted> {
        self.lifted
    }

    pub fn take_lifted(&mut self) -> Option<Lifted> {
        self.lifted.take()
    }

    pub fn slide(&self, index: usize) -> Option<&Animated> {
        self.slides.get(index)
    }

    pub fn floating(&self, now: Instant) -> Option<usize> {
        self.lifted.map(|lifted| lifted.index).or_else(|| {
            self.landing.filter(|index| {
                self.slides
                    .get(*index)
                    .is_some_and(|slide| slide.value_at(now).1)
            })
        })
    }

    pub fn animating(&self, now: Instant) -> bool {
        self.slides.iter().any(|slide| slide.value_at(now).1)
    }

    pub fn lift(&mut self, slots: &[ReorderSlot], index: usize, grab: f32) {
        self.landing = None;
        self.lifted = slots.get(index).map(|slot| Lifted {
            index,
            start: index,
            grab,
            left: slot.x,
        });
    }

    pub fn drag_to(
        &mut self,
        slots: &[ReorderSlot],
        run: RangeInclusive<usize>,
        x: f32,
        now: Instant,
        reduce_motion: bool,
    ) -> Option<(usize, usize)> {
        let lifted = self.lifted?;
        let from = lifted.index;
        let lowest = left_at(slots, from, *run.start())?;
        let highest = left_at(slots, from, *run.end())?;
        let left = (x - lifted.grab).clamp(lowest, highest.max(lowest));
        let to = run
            .filter_map(|to| Some((to, left_at(slots, from, to)?)))
            .min_by(|(_, a), (_, b)| (a - left).abs().total_cmp(&(b - left).abs()))
            .map_or(from, |(to, _)| to);
        self.lifted = Some(Lifted {
            index: to,
            left,
            ..lifted
        });
        self.slide_neighbours(slots, from, to, now, reduce_motion)
            .then_some((from, to))
    }

    pub fn return_to_start(
        &mut self,
        slots: &[ReorderSlot],
        now: Instant,
        reduce_motion: bool,
    ) -> Option<(usize, usize)> {
        let lifted = self.lifted.as_mut()?;
        let (from, to) = (lifted.index, lifted.start);
        lifted.index = to;
        self.slide_neighbours(slots, from, to, now, reduce_motion)
            .then_some((from, to))
    }

    pub fn settle(&mut self, slots: &[ReorderSlot], now: Instant, reduce_motion: bool) {
        let Some(lifted) = self.lifted.take() else {
            return;
        };
        let (Some(slot), Some(slide)) =
            (slots.get(lifted.index), self.slides.get_mut(lifted.index))
        else {
            return;
        };
        slide.jump_to(lifted.left - slot.x);
        slide.animate_at(now, reduce_motion, 0.0, Spring::STANDARD);
        self.landing = Some(lifted.index);
    }

    pub fn move_item(
        &mut self,
        slots: &[ReorderSlot],
        from: usize,
        to: usize,
        now: Instant,
        reduce_motion: bool,
    ) -> bool {
        let (Some(start), Some(end)) = (slots.get(from), left_at(slots, from, to)) else {
            return false;
        };
        let start = start.x;
        if !self.slide_neighbours(slots, from, to, now, reduce_motion) {
            return false;
        }
        if let Some(slide) = self.slides.get_mut(to) {
            let (current, _) = slide.value_at(now);
            slide.jump_to(current + start - end);
            slide.animate_at(now, reduce_motion, 0.0, Spring::STANDARD);
        }
        self.landing = Some(to);
        true
    }

    fn slide_neighbours(
        &mut self,
        slots: &[ReorderSlot],
        from: usize,
        to: usize,
        now: Instant,
        reduce_motion: bool,
    ) -> bool {
        if from == to || from >= self.slides.len() || to >= self.slides.len() {
            return false;
        }
        let Some(moved) = slots.get(from) else {
            return false;
        };
        let gap = match slots {
            [first, second, ..] => second.x - first.x - first.width,
            _ => 0.0,
        };
        let shift = match from < to {
            true => moved.width + gap,
            false => -(moved.width + gap),
        };
        let slide = self.slides.remove(from);
        self.slides.insert(to, slide);
        for index in (from.min(to)..=from.max(to)).filter(|index| *index != to) {
            if let Some(slide) = self.slides.get_mut(index) {
                let (current, _) = slide.value_at(now);
                slide.jump_to(current + shift);
                slide.animate_at(now, reduce_motion, 0.0, Spring::STANDARD);
            }
        }
        true
    }
}

fn left_at(slots: &[ReorderSlot], from: usize, to: usize) -> Option<f32> {
    let slot = slots.get(to)?;
    match to > from {
        true => Some(slot.x + slot.width - slots.get(from)?.width),
        false => Some(slot.x),
    }
}
