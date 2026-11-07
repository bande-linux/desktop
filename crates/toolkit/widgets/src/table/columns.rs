use std::ops::RangeInclusive;

use bande_graphics::Point;

use super::{Drag, Table};
use crate::reorder::ReorderSlot;
use crate::{Context, Spring};

impl Table {
    pub fn on_column_order(self, callback: impl FnMut(&[usize]) + 'static) -> Self {
        Self {
            on_column_order: Box::new(callback),
            ..self
        }
    }

    pub fn set_column_order(&mut self, order: &[usize]) {
        if self.reorder.lifted().is_some() || order == self.order {
            return;
        }
        let mut known = order.to_vec();
        known.sort_unstable();
        if known.into_iter().eq(0..self.columns.len()) {
            self.order = order.to_vec();
        }
    }

    pub fn is_moving_column(&self) -> bool {
        self.drag == Some(Drag::Column)
    }

    pub fn column_at(&self, point: Point) -> Option<usize> {
        self.column_edges()
            .iter()
            .position(|(x, width)| point.x >= *x && point.x < x + width)
    }

    pub fn move_column(&mut self, cx: &Context, column: usize, by: isize) -> bool {
        let Some(from) = self.shown_at(column).filter(|_| self.is_movable(column)) else {
            return false;
        };
        let run = self.movable_run(from);
        let to = from
            .saturating_add_signed(by)
            .clamp(*run.start(), *run.end());
        let moved = self
            .reorder
            .move_item(&self.slots(), from, to, cx.start, cx.reduce_motion);
        if moved {
            self.rotate_order(from, to);
        }
        moved
    }

    pub(super) fn column_shifts(&self, cx: &Context) -> Vec<f32> {
        let edges = self.column_edges();
        let mut shifts = vec![0.0; self.columns.len()];
        for (shown, column) in self.order.iter().copied().enumerate() {
            shifts[column] = match self.reorder.lifted().filter(|lifted| lifted.index == shown) {
                Some(lifted) => lifted.left - edges[column].0,
                None => self
                    .reorder
                    .slide(shown)
                    .map_or(0.0, |slide| cx.sample(slide)),
            };
        }
        shifts
    }

    pub(super) fn floating_column(&self, cx: &Context) -> Option<usize> {
        let shown = self.reorder.floating(cx.now())?;
        self.order.get(shown).copied()
    }

    pub(super) fn lift_column(
        &mut self,
        cx: &mut Context,
        column: usize,
        from: Point,
        at: Point,
    ) -> bool {
        let Some(shown) = self.shown_at(column).filter(|_| self.is_movable(column)) else {
            return false;
        };
        if self.movable_run(shown).count() < 2 {
            return false;
        }
        let slots = self.slots();
        let grab = from.x - slots[shown].x;
        self.reorder.lift(&slots, shown, grab);
        self.drag = Some(Drag::Column);
        cx.animate(&mut self.rise, 1.0, Spring::QUICK);
        self.drag_column(cx, at.x);
        true
    }

    pub(super) fn drag_column(&mut self, cx: &Context, x: f32) {
        let Some(lifted) = self.reorder.lifted() else {
            return;
        };
        let run = self.movable_run(lifted.index);
        if let Some((from, to)) =
            self.reorder
                .drag_to(&self.slots(), run, x, cx.start, cx.reduce_motion)
        {
            self.rotate_order(from, to);
        }
    }

    pub(super) fn drop_column(&mut self, cx: &mut Context) {
        let moved = self
            .reorder
            .lifted()
            .is_some_and(|lifted| lifted.index != lifted.start);
        self.land_column(cx);
        if moved {
            (self.on_column_order)(&self.order);
        }
    }

    pub(super) fn cancel_column(&mut self, cx: &mut Context) {
        self.drag = None;
        if let Some((from, to)) =
            self.reorder
                .return_to_start(&self.slots(), cx.start, cx.reduce_motion)
        {
            self.rotate_order(from, to);
        }
        self.land_column(cx);
    }

    fn land_column(&mut self, cx: &Context) {
        self.reorder
            .settle(&self.slots(), cx.start, cx.reduce_motion);
        cx.animate(&mut self.rise, 0.0, Spring::QUICK);
    }

    fn rotate_order(&mut self, from: usize, to: usize) {
        let column = self.order.remove(from);
        self.order.insert(to, column);
    }

    fn shown_at(&self, column: usize) -> Option<usize> {
        self.order.iter().position(|shown| *shown == column)
    }

    fn is_movable(&self, column: usize) -> bool {
        column != 0 && self.columns.get(column).is_some_and(|shown| shown.movable)
    }

    fn movable_run(&self, shown: usize) -> RangeInclusive<usize> {
        let movable = |at: &usize| self.is_movable(self.order[*at]);
        let start = (0..shown).rev().take_while(movable).last().unwrap_or(shown);
        let end = (shown + 1..self.order.len())
            .take_while(movable)
            .last()
            .unwrap_or(shown);
        start..=end
    }

    fn slots(&self) -> Vec<ReorderSlot> {
        let edges = self.column_edges();
        self.order
            .iter()
            .map(|column| ReorderSlot {
                x: edges[*column].0,
                width: edges[*column].1,
            })
            .collect()
    }
}
