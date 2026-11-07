use std::io::{self, Write};
use std::path::Path;

use bande_content::pdf::protocol::{Render, Use};
use bande_content::pdf::session::{Change, Phase, Session};
use rustix::event::{PollFd, PollFlags, Timespec, poll};

const PAGE_PATIENCE: Timespec = Timespec {
    tv_sec: 180,
    tv_nsec: 0,
};
const POINTS_PER_INCH: f32 = 72.0;
const MILLIMETRES_PER_INCH: f32 = 25.4;
const PWG_SYNC: &[u8] = b"RaS2";
const URF_SYNC: &[u8] = b"UNIRAST\0";
const PWG_HEADER: usize = 1796;
const PWG_SRGB: u32 = 19;
const PWG_GRAY: u32 = 18;
const URF_SRGB: u8 = 1;
const URF_GRAY: u8 = 0;
const URF_SIMPLEX: u8 = 1;
const URF_TUMBLE: u8 = 2;
const URF_NO_TUMBLE: u8 = 3;
const URF_QUALITY: u8 = 4;
const WHITE: u8 = 255;
const MAX_REPEAT: usize = 128;
const MAX_LITERAL: usize = 127;
const MAX_LINE_REPEAT: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Pwg,
    Urf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backside {
    pub mirror_across: bool,
    pub mirror_along: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub format: Format,
    pub dpi: u32,
    pub media_points: (f32, f32),
    pub media_keyword: String,
    pub color: bool,
    pub two_sided: bool,
    pub tumble: bool,
    pub backside: Backside,
    pub landscape: bool,
    pub copies: u32,
    pub pages: Vec<(i32, i32)>,
}

pub fn media_size(keyword: &str) -> Option<(f32, f32)> {
    let size = keyword.rsplit('_').next()?;
    let (unit_per_inch, numbers) = match (size.strip_suffix("mm"), size.strip_suffix("in")) {
        (Some(numbers), _) => (MILLIMETRES_PER_INCH, numbers),
        (_, Some(numbers)) => (1.0, numbers),
        _ => return None,
    };
    let (width, height) = numbers.split_once('x')?;
    let scale = POINTS_PER_INCH / unit_per_inch;
    Some((
        width.parse::<f32>().ok()? * scale,
        height.parse::<f32>().ok()? * scale,
    ))
}

fn compress_line(pixels: &[u8], pixel: usize, out: &mut Vec<u8>) {
    let count = pixels.len() / pixel;
    let at = |index: usize| &pixels[index * pixel..(index + 1) * pixel];
    let mut index = 0;
    while index < count {
        let mut run = 1;
        while index + run < count && run < MAX_REPEAT && at(index + run) == at(index) {
            run += 1;
        }
        if run > 1 {
            out.push((run - 1) as u8);
            out.extend_from_slice(at(index));
            index += run;
            continue;
        }
        let start = index;
        while index < count
            && index - start < MAX_LITERAL
            && (index + 1 >= count || at(index) != at(index + 1))
        {
            index += 1;
        }
        let literal = index - start;
        if literal == 1 {
            out.push(0);
            out.extend_from_slice(at(start));
        } else {
            out.push((257 - literal) as u8);
            out.extend_from_slice(&pixels[start * pixel..index * pixel]);
        }
    }
}

pub fn compress_sheet(
    pixels: &[u8],
    line_bytes: usize,
    pixel: usize,
    out: &mut dyn Write,
) -> io::Result<()> {
    let mut encoded = Vec::with_capacity(line_bytes);
    let mut lines = pixels.chunks_exact(line_bytes).peekable();
    while let Some(line) = lines.next() {
        let mut repeats = 0;
        while repeats + 1 < MAX_LINE_REPEAT && lines.peek() == Some(&line) {
            lines.next();
            repeats += 1;
        }
        encoded.clear();
        encoded.push(repeats as u8);
        compress_line(line, pixel, &mut encoded);
        out.write_all(&encoded)?;
    }
    Ok(())
}

fn put(header: &mut [u8], at: usize, value: u32) {
    header[at..at + 4].copy_from_slice(&value.to_be_bytes());
}

fn pwg_header(plan: &Plan, size: (u32, u32), total: u32, back: bool) -> Vec<u8> {
    let mut header = vec![0; PWG_HEADER];
    let class = b"PwgRaster";
    header[..class.len()].copy_from_slice(class);
    let (bits, space, colors) = match plan.color {
        true => (24, PWG_SRGB, 3),
        false => (8, PWG_GRAY, 1),
    };
    put(&mut header, 272, u32::from(plan.two_sided));
    put(&mut header, 276, plan.dpi);
    put(&mut header, 280, plan.dpi);
    put(&mut header, 340, 1);
    put(&mut header, 352, plan.media_points.0.round() as u32);
    put(&mut header, 356, plan.media_points.1.round() as u32);
    put(&mut header, 368, u32::from(plan.tumble));
    put(&mut header, 372, size.0);
    put(&mut header, 376, size.1);
    put(&mut header, 384, 8);
    put(&mut header, 388, bits);
    put(&mut header, 392, size.0 * bits / 8);
    put(&mut header, 400, space);
    put(&mut header, 420, colors);
    put(&mut header, 452, total);
    let transform = |mirror: bool| if back && mirror { u32::MAX } else { 1 };
    put(&mut header, 456, transform(plan.backside.mirror_across));
    put(&mut header, 460, transform(plan.backside.mirror_along));
    put(&mut header, 472, size.0);
    put(&mut header, 476, size.1);
    let name = plan.media_keyword.as_bytes();
    let length = name.len().min(63);
    header[1732..1732 + length].copy_from_slice(&name[..length]);
    header
}

fn urf_header(plan: &Plan, size: (u32, u32)) -> [u8; 32] {
    let mut header = [0; 32];
    header[0] = if plan.color { 24 } else { 8 };
    header[1] = if plan.color { URF_SRGB } else { URF_GRAY };
    header[2] = match (plan.two_sided, plan.tumble) {
        (false, _) => URF_SIMPLEX,
        (true, true) => URF_TUMBLE,
        (true, false) => URF_NO_TUMBLE,
    };
    header[3] = URF_QUALITY;
    put(&mut header, 12, size.0);
    put(&mut header, 16, size.1);
    put(&mut header, 20, plan.dpi);
    header
}

fn sheet_size(plan: &Plan) -> (u32, u32) {
    let scale = plan.dpi as f32 / POINTS_PER_INCH;
    (
        (plan.media_points.0 * scale).round() as u32,
        (plan.media_points.1 * scale).round() as u32,
    )
}

fn placement(page: (f32, f32), sheet: (u32, u32), landscape: bool, dpi: u32) -> (f32, u32, u32) {
    let (page_width, page_height) = match landscape {
        true => (page.1, page.0),
        false => page,
    };
    let scale = (sheet.0 as f32 / page_width)
        .min(sheet.1 as f32 / page_height)
        .min(dpi as f32 / POINTS_PER_INCH);
    (
        scale,
        (page.0 * scale).round().max(1.0) as u32,
        (page.1 * scale).round().max(1.0) as u32,
    )
}

fn sheet_from(
    rgba: &[u8],
    rendered: (u32, u32),
    sheet: (u32, u32),
    landscape: bool,
    color: bool,
) -> Vec<u8> {
    let channels = if color { 3 } else { 1 };
    let (sheet_width, sheet_height) = (sheet.0 as usize, sheet.1 as usize);
    let (width, height) = (rendered.0 as usize, rendered.1 as usize);
    let (placed_width, placed_height) = if landscape {
        (height, width)
    } else {
        (width, height)
    };
    let left = sheet_width.saturating_sub(placed_width) / 2;
    let top = sheet_height.saturating_sub(placed_height) / 2;
    let mut out = vec![WHITE; sheet_width * sheet_height * channels];
    for y in 0..height {
        for x in 0..width {
            let (target_x, target_y) = match landscape {
                true => (left + height - 1 - y, top + x),
                false => (left + x, top + y),
            };
            if target_x >= sheet_width || target_y >= sheet_height {
                continue;
            }
            let source = (y * width + x) * 4;
            let Some(pixel) = rgba.get(source..source + 4) else {
                continue;
            };
            let target = (target_y * sheet_width + target_x) * channels;
            if color {
                out[target..target + 3].copy_from_slice(&pixel[..3]);
            } else {
                let (r, g, b) = (
                    u32::from(pixel[0]),
                    u32::from(pixel[1]),
                    u32::from(pixel[2]),
                );
                out[target] = ((r * 54 + g * 183 + b * 19) / 256) as u8;
            }
        }
    }
    out
}

fn mirror(sheet: &mut [u8], width: usize, channels: usize, backside: Backside) {
    let line = width * channels;
    if backside.mirror_along {
        let height = sheet.len() / line;
        for y in 0..height / 2 {
            let (upper, lower) = sheet.split_at_mut((height - 1 - y) * line);
            upper[y * line..(y + 1) * line].swap_with_slice(&mut lower[..line]);
        }
    }
    if backside.mirror_across {
        for row in sheet.chunks_exact_mut(line) {
            for x in 0..width / 2 {
                for channel in 0..channels {
                    row.swap(x * channels + channel, (width - 1 - x) * channels + channel);
                }
            }
        }
    }
}

fn wait_for(session: &mut Session, mut until: impl FnMut(Change) -> bool) -> io::Result<()> {
    loop {
        for change in session.take() {
            if until(change) {
                return Ok(());
            }
        }
        let mut fds = [PollFd::from_borrowed_fd(session.fd(), PollFlags::IN)];
        match poll(&mut fds, Some(&PAGE_PATIENCE)) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::TimedOut)),
            Ok(_) | Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(error.into()),
        }
    }
}

fn selected(pages: usize, ranges: &[(i32, i32)]) -> Vec<usize> {
    (0..pages)
        .filter(|page| {
            let number = *page as i32 + 1;
            ranges.is_empty()
                || ranges
                    .iter()
                    .any(|(first, last)| number >= *first && number <= *last)
        })
        .collect()
}

pub fn write_document(
    path: &Path,
    plan: &Plan,
    cancelled: &dyn Fn() -> bool,
    out: &mut dyn Write,
) -> io::Result<()> {
    let mut session = Session::open(path)?;
    wait_for(&mut session, |_| true)?;
    if !matches!(session.phase(), Phase::Open) {
        return Err(io::Error::other("the document cannot be opened"));
    }
    let sizes = session.opened().pages.clone();
    let pages = selected(sizes.len(), &plan.pages);
    let total = pages.len() as u32 * plan.copies.max(1);
    let sheet = sheet_size(plan);
    match plan.format {
        Format::Pwg => out.write_all(PWG_SYNC)?,
        Format::Urf => {
            out.write_all(URF_SYNC)?;
            out.write_all(&total.to_be_bytes())?;
        }
    }
    let channels = if plan.color { 3 } else { 1 };
    let mut sheet_number = 0;
    for _ in 0..plan.copies.max(1) {
        for page in &pages {
            if cancelled() {
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            let (scale, width, height) = placement(sizes[*page], sheet, plan.landscape, plan.dpi);
            let render = Render {
                page: *page,
                scale,
                x: 0,
                y: 0,
                width,
                height,
                purpose: Use::Page,
            };
            session.want(vec![render]);
            let mut pixels = None;
            wait_for(&mut session, |change| match change {
                Change::Pixels(done, bytes) if done.page == *page => {
                    pixels = Some(bytes);
                    true
                }
                Change::Failed => true,
                _ => false,
            })?;
            session.want(Vec::new());
            let rgba = pixels.ok_or_else(|| io::Error::other("a page cannot be drawn"))?;
            let mut bytes = sheet_from(&rgba, (width, height), sheet, plan.landscape, plan.color);
            let back = plan.two_sided && sheet_number % 2 == 1;
            if back {
                mirror(&mut bytes, sheet.0 as usize, channels, plan.backside);
            }
            match plan.format {
                Format::Pwg => out.write_all(&pwg_header(plan, sheet, total, back))?,
                Format::Urf => out.write_all(&urf_header(plan, sheet))?,
            }
            compress_sheet(&bytes, sheet.0 as usize * channels, channels, out)?;
            sheet_number += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_packs_runs_and_literals_as_the_raster_formats_define() {
        let mut out = Vec::new();
        compress_line(&[7, 7, 7, 1, 2, 3, 9], 1, &mut out);
        assert_eq!(out, vec![2, 7, 253, 1, 2, 3, 9]);
    }

    #[test]
    fn equal_lines_become_one_line_with_a_repeat_count() {
        let mut out = Vec::new();
        compress_sheet(&[5, 5, 5, 5, 5, 5], 2, 1, &mut out).ok();
        assert_eq!(out, vec![2, 1, 5]);
    }

    #[test]
    fn media_keywords_give_their_size_in_points() {
        let (width, height) = media_size("iso_a4_210x297mm").unwrap_or_default();
        assert!((width - 595.3).abs() < 0.1 && (height - 841.9).abs() < 0.1);
        assert_eq!(media_size("na_letter_8.5x11in"), Some((612.0, 792.0)));
    }
}
