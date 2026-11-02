use web_style::{Display, FlexDirection};
use web_text::AtomicSize;

use crate::box_tree::{BlockBox, Contents};
use crate::flow::{Flow, frame, resolve_margins};

impl<'a, 'b> Flow<'a, 'b> {
    pub(crate) fn intrinsic(&mut self, block: &'b BlockBox, cb: (f32, Option<f32>)) -> (f32, f32) {
        let reset = &block.style.reset;
        if let Some(width) = reset.width.filter(|width| width.percent == 0.0) {
            let width = if reset.border_box {
                (width.px - frame(&block.style, cb.0).0).max(0.0)
            } else {
                width.px
            };
            return (width, width);
        }
        match &block.contents {
            Contents::Replaced(_) => {
                let width = self.replaced_size(block, cb).0;
                (width, width)
            }
            Contents::Table(table) => self.table_intrinsic(block, table, cb.0),
            Contents::Inline(inline) => {
                let outer: Vec<(f32, f32)> = inline
                    .atomics
                    .iter()
                    .map(|atomic| self.outer_intrinsic(atomic, cb))
                    .collect();
                let sizes = |pick: fn(&(f32, f32)) -> f32| -> Vec<AtomicSize> {
                    outer
                        .iter()
                        .map(|size| AtomicSize {
                            width: pick(size),
                            ascent: 0.0,
                            descent: 0.0,
                        })
                        .collect()
                };
                let min = inline.shaped.intrinsic(&sizes(|size| size.0)).0;
                let max = inline.shaped.intrinsic(&sizes(|size| size.1)).1;
                let floats = inline
                    .out_of_flow
                    .iter()
                    .filter(|child| !child.style.is_out_of_flow())
                    .map(|child| self.outer_intrinsic(child, cb))
                    .fold((0.0, 0.0), |sum: (f32, f32), size| {
                        (sum.0.max(size.0), sum.1 + size.1)
                    });
                (min.max(floats.0), max + floats.1)
            }
            Contents::Blocks(children) | Contents::Flex(children) => {
                let row = matches!(block.style.display(), Display::Flex | Display::InlineFlex)
                    && matches!(
                        reset.flex_direction,
                        FlexDirection::Row | FlexDirection::RowReverse
                    );
                let mut min: f32 = 0.0;
                let mut max: f32 = 0.0;
                let mut floats: f32 = 0.0;
                for child in children
                    .iter()
                    .filter(|child| !child.style.is_out_of_flow())
                {
                    let (child_min, child_max) = self.outer_intrinsic(child, cb);
                    min = min.max(child_min);
                    if row {
                        max += child_max;
                    } else if child.style.reset.float != web_style::Float::None {
                        floats += child_max;
                        max = max.max(floats);
                    } else {
                        floats = 0.0;
                        max = max.max(child_max);
                    }
                }
                (min, max.max(min))
            }
        }
    }

    pub(crate) fn outer_intrinsic(
        &mut self,
        block: &'b BlockBox,
        cb: (f32, Option<f32>),
    ) -> (f32, f32) {
        let (min, max) = self.intrinsic(block, cb);
        let margins = resolve_margins(&block.style, 0.0);
        let outer =
            frame(&block.style, 0.0).0 + margins.left.unwrap_or(0.0) + margins.right.unwrap_or(0.0);
        (min + outer, max + outer)
    }
}
