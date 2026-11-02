use web_style::{Align, FlexDirection};

use crate::box_tree::BlockBox;
use crate::flow::{Bfc, Flow, Laid, Sizing, frame, resolve_margins};
use crate::fragment::Fragment;

struct Item<'b> {
    block: &'b BlockBox,
    margin: (f32, f32, f32, f32),
    frame: (f32, f32),
    base: f32,
    minimum: f32,
    maximum: f32,
    main: f32,
    grow: f32,
    shrink: f32,
}

impl Item<'_> {
    fn outer_main(&self, row: bool) -> f32 {
        let (margin, frame) = if row {
            (self.margin.3 + self.margin.1, self.frame.0)
        } else {
            (self.margin.0 + self.margin.2, self.frame.1)
        };
        self.main + margin + frame
    }
}

fn alignment(container: Align, own: Align) -> Align {
    match own {
        Align::Auto => container,
        own => own,
    }
}

impl<'a, 'b> Flow<'a, 'b> {
    pub(crate) fn flex(
        &mut self,
        block: &'b BlockBox,
        items: &'b [BlockBox],
        size: (f32, Option<f32>),
        origin: (f32, f32),
        offset: (f32, f32),
        children: &mut Vec<Fragment>,
    ) -> f32 {
        let reset = &block.style.reset;
        let row = matches!(
            reset.flex_direction,
            FlexDirection::Row | FlexDirection::RowReverse
        );
        let reverse = matches!(
            reset.flex_direction,
            FlexDirection::RowReverse | FlexDirection::ColumnReverse
        );
        let (width, height) = size;
        let main_size = if row { Some(width) } else { height };
        let main_gap = if row {
            reset.column_gap.resolve(width)
        } else {
            reset.row_gap.resolve(height.unwrap_or(0.0))
        };
        let cross_gap = if row {
            reset.row_gap.resolve(height.unwrap_or(0.0))
        } else {
            reset.column_gap.resolve(width)
        };
        let mut ordered: Vec<&'b BlockBox> = Vec::new();
        for item in items {
            if item.style.is_out_of_flow() {
                self.register_absolute(item, origin);
            } else {
                ordered.push(item);
            }
        }
        ordered.sort_by_key(|item| item.style.reset.order);
        let mut prepared: Vec<Item<'b>> = Vec::with_capacity(ordered.len());
        for item in ordered {
            let style = &item.style;
            let item_reset = &style.reset;
            let margins = resolve_margins(style, width).map(|side| side.unwrap_or(0.0));
            let item_frame = frame(style, width);
            let main_frame = if row { item_frame.0 } else { item_frame.1 };
            let to_content = |value: f32| {
                if item_reset.border_box {
                    (value - main_frame).max(0.0)
                } else {
                    value
                }
            };
            let basis = item_reset
                .flex_basis
                .map(|basis| basis.resolve(main_size.unwrap_or(0.0)))
                .or_else(|| {
                    if row {
                        item_reset
                            .width
                            .map(|value| to_content(value.resolve(width)))
                    } else {
                        item_reset.height.and_then(|value| {
                            height.map(|height| to_content(value.resolve(height)))
                        })
                    }
                });
            let (intrinsic_min, base) = if row {
                let (min, max) = self.intrinsic(item, (width, height));
                (min, basis.unwrap_or(max))
            } else {
                let cross = (width - margins.left - margins.right).max(0.0);
                let measured = match basis {
                    Some(basis) => basis,
                    None => {
                        let laid = self.block(
                            item,
                            (cross, None),
                            origin,
                            &mut Bfc::default(),
                            Sizing {
                                width: Some(cross),
                                ..Sizing::default()
                            },
                        );
                        laid.fragment.rect.height - item_frame.1
                    }
                };
                (0.0, measured)
            };
            let explicit_min = if row {
                item_reset.min_width.resolve(width)
            } else {
                item_reset.min_height.resolve(height.unwrap_or(0.0))
            };
            let minimum = if explicit_min > 0.0 || item_reset.clips {
                explicit_min
            } else {
                intrinsic_min.min(base)
            };
            let maximum = if row {
                item_reset.max_width.map(|max| max.resolve(width))
            } else {
                item_reset
                    .max_height
                    .map(|max| max.resolve(height.unwrap_or(0.0)))
            }
            .unwrap_or(f32::INFINITY);
            prepared.push(Item {
                block: item,
                margin: (margins.top, margins.right, margins.bottom, margins.left),
                frame: item_frame,
                base,
                minimum,
                maximum,
                main: base.clamp(minimum, maximum.max(minimum)),
                grow: item_reset.flex_grow,
                shrink: item_reset.flex_shrink,
            });
        }
        let mut lines: Vec<Vec<Item<'b>>> = Vec::new();
        let mut current: Vec<Item<'b>> = Vec::new();
        let mut used = 0.0;
        for item in prepared {
            let outer = item.outer_main(row);
            let limit = main_size.unwrap_or(f32::INFINITY);
            if reset.flex_wrap && !current.is_empty() && used + main_gap + outer > limit {
                lines.push(std::mem::take(&mut current));
                used = 0.0;
            }
            used += if current.is_empty() {
                outer
            } else {
                main_gap + outer
            };
            current.push(item);
        }
        if !current.is_empty() {
            lines.push(current);
        }
        let mut cross_offset = 0.0;
        let mut main_extent: f32 = 0.0;
        let line_count = lines.len();
        for (line_index, mut line) in lines.into_iter().enumerate() {
            let gaps = main_gap * line.len().saturating_sub(1) as f32;
            if let Some(main_size) = main_size {
                let used: f32 = line.iter().map(|item| item.outer_main(row)).sum::<f32>() + gaps;
                let free = main_size - used;
                if free > 0.0 {
                    let grow: f32 = line.iter().map(|item| item.grow).sum();
                    if grow > 0.0 {
                        for item in &mut line {
                            item.main = (item.main + free * item.grow / grow).min(item.maximum);
                        }
                    }
                } else if free < 0.0 {
                    let weight: f32 = line.iter().map(|item| item.shrink * item.base).sum();
                    if weight > 0.0 {
                        for item in &mut line {
                            let share = -free * item.shrink * item.base / weight;
                            item.main = (item.main - share).max(item.minimum);
                        }
                    }
                }
            }
            let mut laid: Vec<Laid> = Vec::with_capacity(line.len());
            for item in &line {
                laid.push(self.flex_item(item, row, (width, height), origin, None));
            }
            let line_cross = if line_count == 1
                && let Some(cross) = if row { height } else { Some(width) }
            {
                cross
            } else {
                laid.iter()
                    .map(|laid| if row { laid.outer().1 } else { laid.outer().0 })
                    .fold(0.0, f32::max)
            };
            for (item, laid) in line.iter().zip(laid.iter_mut()) {
                let align = alignment(reset.align_items, item.block.style.reset.align_self);
                let auto_cross = if row {
                    item.block.style.reset.height.is_none()
                } else {
                    item.block.style.reset.width.is_none()
                };
                if matches!(align, Align::Stretch | Align::Normal) && auto_cross {
                    let cross = if row {
                        line_cross - item.margin.0 - item.margin.2
                    } else {
                        line_cross - item.margin.1 - item.margin.3
                    };
                    *laid =
                        self.flex_item(item, row, (width, height), origin, Some(cross.max(0.0)));
                }
            }
            let used: f32 = laid
                .iter()
                .map(|laid| if row { laid.outer().0 } else { laid.outer().1 })
                .sum::<f32>()
                + gaps;
            let free = main_size.map_or(0.0, |main| (main - used).max(0.0));
            let count = laid.len() as f32;
            let (mut position, between) = match reset.justify_content {
                Align::End => (free, 0.0),
                Align::Center => (free / 2.0, 0.0),
                Align::SpaceBetween if count > 1.0 => (0.0, free / (count - 1.0)),
                Align::SpaceAround if count > 0.0 => (free / count / 2.0, free / count),
                Align::SpaceEvenly if count > 0.0 => (free / (count + 1.0), free / (count + 1.0)),
                _ => (0.0, 0.0),
            };
            for (item, laid) in line.iter().zip(laid) {
                let (outer_main, outer_cross) = if row {
                    laid.outer()
                } else {
                    (laid.outer().1, laid.outer().0)
                };
                let align = alignment(reset.align_items, item.block.style.reset.align_self);
                let cross_position = match align {
                    Align::End => line_cross - outer_cross,
                    Align::Center => (line_cross - outer_cross) / 2.0,
                    _ => 0.0,
                };
                let main_position = if reverse {
                    main_size.unwrap_or(used) - position - outer_main
                } else {
                    position
                };
                let mut fragment = laid.fragment;
                let (x, y) = if row {
                    (
                        main_position + laid.margin.left,
                        cross_offset + cross_position + laid.margin.top,
                    )
                } else {
                    (
                        cross_offset + cross_position + laid.margin.left,
                        main_position + laid.margin.top,
                    )
                };
                fragment.rect.x = offset.0 + x;
                fragment.rect.y = offset.1 + y;
                children.push(Fragment::Box(fragment));
                position += outer_main + main_gap + between;
            }
            main_extent = main_extent.max(used);
            cross_offset += line_cross;
            if line_index + 1 < line_count {
                cross_offset += cross_gap;
            }
        }
        if row {
            cross_offset
        } else {
            main_size.unwrap_or(main_extent)
        }
    }

    fn flex_item(
        &mut self,
        item: &Item<'b>,
        row: bool,
        size: (f32, Option<f32>),
        origin: (f32, f32),
        cross: Option<f32>,
    ) -> Laid {
        let main = item.main + if row { item.frame.0 } else { item.frame.1 };
        let sizing = if row {
            Sizing {
                width: Some(main),
                height: cross,
                shrink: false,
                root: false,
            }
        } else {
            Sizing {
                width: cross.or(Some((size.0 - item.margin.1 - item.margin.3).max(0.0))),
                height: Some(main),
                shrink: false,
                root: false,
            }
        };
        self.block(item.block, size, origin, &mut Bfc::default(), sizing)
    }
}
