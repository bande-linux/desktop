use bande_graphics::{Paragraph, Point};

use crate::page::{Page, PageItem, PageSetup};

const SMALLEST_WIDOW: usize = 2;

#[derive(Debug, Clone)]
pub struct Block {
    pub paragraph: Paragraph,
    pub space_after: f32,
}

fn lines_kept(total: usize, fitting: usize) -> usize {
    if fitting == total {
        return total;
    }
    match fitting.min(total.saturating_sub(SMALLEST_WIDOW)) {
        0 | 1 => 0,
        kept => kept,
    }
}

pub fn flow(blocks: &[Block], setup: &PageSetup) -> Vec<Page> {
    let area = setup.content();
    let bottom = area.y + area.height;
    let mut pages = vec![Page::new(setup)];
    let mut cursor = area.y;
    for block in blocks {
        let extents = block.paragraph.line_extents();
        let mut first = 0;
        let mut top_of_slice = 0.0;
        while first < extents.len() {
            let fitting = extents[first..]
                .iter()
                .take_while(|(_, end)| cursor + (end - top_of_slice) <= bottom)
                .count();
            let kept = lines_kept(extents.len() - first, fitting);
            let kept = match kept == 0 && cursor == area.y {
                true => 1,
                false => kept,
            };
            if kept > 0 {
                let last_page = pages.len() - 1;
                pages[last_page].items.push(PageItem::Lines {
                    origin: Point::new(area.x, cursor - top_of_slice),
                    paragraph: block.paragraph.clone(),
                    lines: first..first + kept,
                });
                cursor += extents[first + kept - 1].1 - top_of_slice;
                first += kept;
            }
            if first < extents.len() {
                pages.push(Page::new(setup));
                cursor = area.y;
                top_of_slice = extents[first].0;
            }
        }
        cursor += block.space_after;
    }
    pages
}
