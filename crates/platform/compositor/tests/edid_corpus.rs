use std::path::{Path, PathBuf};
use std::time::Duration;

use bande_colour::{Profile, conversion, inverse};
use bande_compositor::{edid_profile, refresh_of, sorted_modes};
use bande_edid::{Edid, Primaries, display_name, identity, parse};
use drm_ffi::{DRM_MODE_FLAG_INTERLACE, DRM_MODE_TYPE_DRIVER, DRM_MODE_TYPE_PREFERRED};
use smithay::reexports::drm::control::Mode;

const BLOCK: usize = 128;
const DESCRIPTOR: usize = 18;
const CTA: u8 = 0x02;
const DISPLAY_ID: u8 = 0x70;
const RANGE_LIMITS: u8 = 0xfd;
const HDR_STATIC_METADATA: u8 = 0x06;
const SLOWEST_REFRESH: Duration = Duration::from_secs(1);
const FASTEST_REFRESH: Duration = Duration::from_millis(1);
const SMALLEST_SIDE: u16 = 64;
const STEREO: u8 = 1 << 5;

#[derive(Default)]
struct Tally {
    edids: usize,
    parsed: usize,
    digital: usize,
    deep_colour: usize,
    own_profile: usize,
    hdr: usize,
    variable_refresh: usize,
    display_id: usize,
    without_timings: usize,
    failures: Vec<String>,
}

fn hex_lines(text: &str) -> Vec<u8> {
    let row = |line: &str| -> Option<Vec<u8>> {
        let digits = line.replace(' ', "");
        if digits.is_empty() || digits.len() > 32 || !digits.len().is_multiple_of(2) {
            return None;
        }
        (0..digits.len() / 2)
            .map(|at| u8::from_str_radix(digits.get(2 * at..2 * at + 2)?, 16).ok())
            .collect()
    };
    text.lines()
        .map(str::trim_end)
        .skip_while(|line| row(line).is_none())
        .take_while(|line| line.is_empty() || row(line).is_some())
        .filter_map(row)
        .flatten()
        .collect()
}

fn edid_files(folder: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut folders = vec![folder.to_owned()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for path in entries.flatten().map(|entry| entry.path()) {
            if path.is_dir() {
                folders.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

fn detailed_mode(descriptor: &[u8; DESCRIPTOR], preferred: bool) -> Option<Mode> {
    let clock = u32::from(u16::from_le_bytes([descriptor[0], descriptor[1]])) * 10;
    let wide = |low: usize, high: u8| u16::from(descriptor[low]) | (u16::from(high) << 8);
    let (h, v, sync) = (descriptor[4], descriptor[7], descriptor[11]);
    let hdisplay = wide(2, h >> 4);
    let vdisplay = wide(5, v >> 4);
    let hsync_width = wide(9, (sync >> 4) & 3);
    let vsync_width = u16::from(descriptor[10] & 0xf) | (u16::from(sync & 3) << 4);
    if clock == 0
        || hdisplay < SMALLEST_SIDE
        || vdisplay < SMALLEST_SIDE
        || hsync_width == 0
        || vsync_width == 0
        || descriptor[17] & STEREO != 0
    {
        return None;
    }
    let hsync_start = hdisplay.checked_add(wide(8, (sync >> 6) & 3))?;
    let hsync_end = hsync_start.checked_add(hsync_width)?;
    let vsync_start =
        vdisplay.checked_add(u16::from(descriptor[10] >> 4) | (u16::from((sync >> 2) & 3) << 4))?;
    let vsync_end = vsync_start.checked_add(vsync_width)?;
    let htotal = hdisplay
        .checked_add(wide(3, h & 0xf))?
        .max(hsync_end.checked_add(1)?);
    let vtotal = vdisplay
        .checked_add(wide(6, v & 0xf))?
        .max(vsync_end.checked_add(1)?);
    let mut type_ = DRM_MODE_TYPE_DRIVER;
    if preferred {
        type_ |= DRM_MODE_TYPE_PREFERRED;
    }
    Some(Mode::from(drm_ffi::drm_mode_modeinfo {
        clock,
        hdisplay,
        hsync_start,
        hsync_end,
        htotal,
        vdisplay,
        vsync_start,
        vsync_end,
        vtotal,
        flags: if descriptor[17] & 0x80 != 0 {
            DRM_MODE_FLAG_INTERLACE
        } else {
            0
        },
        type_,
        ..Default::default()
    }))
}

fn blocks(bytes: &[u8]) -> impl Iterator<Item = &[u8; BLOCK]> {
    bytes.as_chunks().0.iter()
}

fn descriptors(base: &[u8; BLOCK]) -> &[[u8; DESCRIPTOR]] {
    base[54..126].as_chunks().0
}

fn cta_data_blocks(block: &[u8; BLOCK]) -> impl Iterator<Item = &[u8]> {
    let end = usize::from(block[2]).clamp(4, BLOCK - 1);
    let mut at = 4;
    std::iter::from_fn(move || {
        let length = usize::from(*block.get(at)? & 0x1f);
        let data = block.get(at..(at + 1 + length).min(end))?;
        at += 1 + length;
        (at <= end).then_some(data)
    })
}

fn detailed_modes(bytes: &[u8]) -> Vec<Mode> {
    let base = blocks(bytes).take(1).flat_map(descriptors);
    let cta = blocks(bytes)
        .skip(1)
        .filter(|block| block[0] == CTA && block[2] >= 4)
        .flat_map(|block| {
            block
                .get(usize::from(block[2])..BLOCK - 1)
                .unwrap_or_default()
                .as_chunks()
                .0
        });
    base.chain(cta)
        .enumerate()
        .filter_map(|(index, descriptor)| detailed_mode(descriptor, index == 0))
        .collect()
}

fn range_limits(base: &[u8; BLOCK]) -> Option<(u16, u16)> {
    descriptors(base).iter().find_map(|descriptor| {
        (descriptor[..3] == [0, 0, 0] && descriptor[3] == RANGE_LIMITS).then(|| {
            let offset = |bit: u8| if descriptor[4] & bit != 0 { 255 } else { 0 };
            (
                u16::from(descriptor[5]) + offset(1),
                u16::from(descriptor[6]) + offset(2),
            )
        })
    })
}

fn clean(text: &str) -> bool {
    !text.chars().any(char::is_control) && text == text.trim()
}

fn check_identity(edid: &Edid) -> Result<(), String> {
    for (field, text) in [
        ("make", &edid.make),
        ("model", &edid.model),
        ("serial", &edid.serial),
    ] {
        if !clean(text) {
            return Err(format!(
                "{field} {text:?} has control characters or padding"
            ));
        }
    }
    if edid.make.is_empty() || edid.model.is_empty() {
        return Err(format!(
            "make {:?} or model {:?} is empty",
            edid.make, edid.model
        ));
    }
    if !clean(&identity(&edid.make, &edid.model, &edid.serial)) {
        return Err("identity is not clean".to_owned());
    }
    display_name(&edid.make, &edid.model)
        .filter(|name| clean(name))
        .map(|_| ())
        .ok_or_else(|| "no clean display name".to_owned())
}

fn plausible(primaries: &Primaries) -> bool {
    let [red, green, blue, white] = [
        primaries.red,
        primaries.green,
        primaries.blue,
        primaries.white,
    ]
    .map(|(x, y)| (f64::from(x), f64::from(y)));
    let side = |(ax, ay): (f64, f64), (bx, by): (f64, f64)| {
        (bx - ax) * (white.1 - ay) - (by - ay) * (white.0 - ax)
    };
    red.0 > green.0.max(blue.0)
        && green.1 > red.1.max(blue.1)
        && blue.1 < red.1
        && [side(red, green), side(green, blue), side(blue, red)]
            .iter()
            .all(|turn| *turn > 0.0)
}

fn check_profile(edid: &Edid, profile: &Profile) -> Result<(), String> {
    if !plausible(&edid.primaries) {
        return Err(format!("profile from implausible {:?}", edid.primaries));
    }
    if profile
        .to_pcs
        .iter()
        .flatten()
        .any(|value| !value.is_finite())
    {
        return Err(format!("profile from {:?} is not finite", edid.primaries));
    }
    let Some(converted) = conversion(profile, [1.0; 3]) else {
        return Ok(());
    };
    let matrix = converted.matrix.map(|row| row.map(f64::from));
    if converted
        .matrix
        .iter()
        .flatten()
        .any(|value| !value.is_finite())
        || inverse(&matrix).is_none()
    {
        return Err(format!(
            "conversion from {:?} is not invertible",
            edid.primaries
        ));
    }
    let rising = (0..3).all(|channel| {
        converted
            .encode
            .windows(2)
            .all(|pair| pair[0][channel] <= pair[1][channel])
    });
    if !rising
        || converted.encode.first() != Some(&[0; 3])
        || converted.encode.last() != Some(&[255; 3])
    {
        return Err(format!(
            "encoding curve for gamma {:?} is not 0..=255 rising",
            edid.gamma
        ));
    }
    Ok(())
}

fn check_modes(bytes: &[u8]) -> Result<bool, String> {
    let modes = detailed_modes(bytes);
    let Some(preferred) = modes.first() else {
        return Ok(false);
    };
    let sorted = sorted_modes(&modes);
    let chosen = sorted.first().ok_or("no mode left")?;
    if chosen.size() != preferred.size() {
        return Err(format!(
            "chose {:?} over the preferred {:?}",
            chosen.size(),
            preferred.size()
        ));
    }
    let refresh = refresh_of(chosen);
    if !(FASTEST_REFRESH..=SLOWEST_REFRESH).contains(&refresh) {
        return Err(format!(
            "chose {:?} refreshing every {refresh:?}",
            chosen.size()
        ));
    }
    let shown = |mode: &Mode| (mode.size(), refresh_of(mode));
    match modes
        .iter()
        .find(|mode| !sorted.iter().any(|kept| shown(kept) == shown(mode)))
    {
        Some(lost) => Err(format!("lost the mode {:?}", shown(lost))),
        None => Ok(true),
    }
}

fn check(bytes: &[u8], tally: &mut Tally) -> Result<(), String> {
    for length in [0, 8, BLOCK - 1] {
        if parse(&bytes[..length.min(bytes.len())]).is_some() {
            return Err(format!("parsed {length} bytes"));
        }
    }
    let Some(edid) = parse(bytes) else {
        return Ok(());
    };
    tally.parsed += 1;
    check_identity(&edid)?;
    if let Some(profile) = edid_profile(&edid) {
        tally.own_profile += 1;
        check_profile(&edid, &profile)?;
    }
    let mut blocks = blocks(bytes);
    let base = blocks.next().ok_or("parsed less than a block")?;
    if base[20] & 0x80 != 0 {
        tally.digital += 1;
        tally.deep_colour += usize::from(base[19] >= 4 && (3..=6).contains(&((base[20] >> 4) & 7)));
    }
    let extensions: Vec<_> = blocks.collect();
    tally.display_id += usize::from(extensions.iter().any(|block| block[0] == DISPLAY_ID));
    tally.hdr += usize::from(
        extensions
            .iter()
            .filter(|block| block[0] == CTA)
            .any(|block| {
                cta_data_blocks(block)
                    .any(|data| data[0] >> 5 == 7 && data.get(1) == Some(&HDR_STATIC_METADATA))
            }),
    );
    if let Some((slowest, fastest)) = range_limits(base) {
        tally.variable_refresh +=
            usize::from(base[24] & 1 != 0 && slowest > 0 && fastest > slowest);
    }
    tally.without_timings += usize::from(!check_modes(bytes)?);
    Ok(())
}

fn run(files: &[PathBuf]) -> Tally {
    let mut tally = Tally::default();
    for file in files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let bytes = hex_lines(&text);
        if bytes.is_empty() {
            continue;
        }
        tally.edids += 1;
        if let Err(failure) = check(&bytes, &mut tally) {
            tally
                .failures
                .push(format!("{}: {failure}", file.display()));
        }
    }
    let share = |count: usize| 100.0 * count as f64 / tally.parsed.max(1) as f64;
    println!(
        "{} EDIDs, {} parsed: {:.1}% digital, {:.1}% advertise 10 or more bits per colour, \
         {:.1}% get a colour profile of their own, {:.1}% HDR, {:.1}% a variable refresh range, \
         {:.1}% DisplayID, {:.1}% no detailed timing",
        tally.edids,
        tally.parsed,
        share(tally.digital),
        share(tally.deep_colour),
        share(tally.own_profile),
        share(tally.hdr),
        share(tally.variable_refresh),
        share(tally.display_id),
        share(tally.without_timings),
    );
    tally
}

#[test]
fn curated_edids_give_sane_identities_colours_and_modes() {
    let tally = run(&edid_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/edid"),
    ));
    assert!(tally.edids > 0);
    assert!(tally.failures.is_empty(), "{}", tally.failures.join("\n"));
}

#[test]
fn linuxhw_edids_give_sane_identities_colours_and_modes() {
    let corpus = std::env::var_os("BANDE_EDID_CORPUS")
        .map(PathBuf::from)
        .or_else(|| bande_xdg::cache_home().map(|cache| cache.join("bande-edid")));
    let files = corpus
        .map(|corpus| edid_files(&corpus.join("edids")))
        .unwrap_or_default();
    if files.is_empty() {
        println!("skipped: no EDID corpus, fetch one with ci/edid-corpus.sh");
        return;
    }
    let tally = run(&files);
    assert!(
        tally.failures.is_empty(),
        "{} failures:\n{}",
        tally.failures.len(),
        tally.failures.join("\n")
    );
}
