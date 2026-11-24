use std::sync::Arc;

use bande_colour::{Curve, DISPLAY, DISPLAY_P3, Profile, SRGB, UNMANAGED, conversion as convert};
use bande_edid::{Edid, Hdr};
use bande_render::{Encode, OutputColour, SDR_WHITE};

const COORDINATE_SCALE: f64 = 1024.0;
const SRGB_PRIMARIES: [(f64, f64); 3] = [(0.64, 0.33), (0.30, 0.60), (0.15, 0.06)];
const SAME_AS_SRGB: f64 = 0.02;
pub(crate) const REFERENCE_WHITE: f64 = SDR_WHITE as f64;
pub(crate) const DISPLAY_PEAK: f64 = 1000.0;
const GAMMA: f64 = 2.2;
const SRGB_TO_BT2020: [[f64; 3]; 3] = [
    [0.627_403_9, 0.329_283_0, 0.043_313_1],
    [0.069_097_3, 0.919_540_4, 0.011_362_3],
    [0.016_391_4, 0.088_013_3, 0.895_595_3],
];
const NEUTRAL: [f64; 3] = [1.0; 3];

fn point((x, y): (u16, u16)) -> (f64, f64) {
    (
        f64::from(x) / COORDINATE_SCALE,
        f64::from(y) / COORDINATE_SCALE,
    )
}

pub fn edid_profile(edid: &Edid) -> Option<Profile> {
    let primaries = [
        edid.primaries.red,
        edid.primaries.green,
        edid.primaries.blue,
    ]
    .map(point);
    let like_srgb = primaries.iter().zip(&SRGB_PRIMARIES).all(|(ours, srgb)| {
        (ours.0 - srgb.0).abs() <= SAME_AS_SRGB && (ours.1 - srgb.1).abs() <= SAME_AS_SRGB
    });
    if edid.default_is_srgb || like_srgb {
        return None;
    }
    let curve = edid
        .gamma
        .map_or(Curve::Srgb, |gamma| Curve::Gamma(f64::from(gamma) / 100.0));
    Profile::from_primaries(primaries, point(edid.primaries.white), curve)
}

fn profile(choice: Option<&str>, display: Option<&Profile>) -> Option<Profile> {
    match choice.unwrap_or(UNMANAGED) {
        DISPLAY => display.cloned(),
        SRGB | UNMANAGED => None,
        DISPLAY_P3 => Profile::display_p3(),
        path => std::fs::read(path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| Profile::parse_icc(&bytes).map_err(|error| error.to_string()))
            .inspect_err(|error| tracing::warn!("cannot use the colour profile {path}: {error}"))
            .ok(),
    }
}

pub(crate) fn converts(choice: Option<&str>) -> bool {
    !matches!(choice.unwrap_or(UNMANAGED), SRGB | UNMANAGED)
}

pub(crate) fn hdr(display: Option<&Hdr>, gains: [f64; 3], extended: bool) -> OutputColour {
    let peak = display
        .and_then(|display| display.max)
        .unwrap_or(DISPLAY_PEAK);
    let light = gains.map(|gain| gain.powf(GAMMA));
    OutputColour {
        matrix: SRGB_TO_BT2020
            .map(|row| std::array::from_fn(|channel| (row[channel] * light[channel]) as f32)),
        encode: Encode::Pq {
            white: REFERENCE_WHITE.min(peak) as f32,
            peak: peak as f32,
        },
        extended,
    }
}

pub(crate) fn conversion(choice: Option<&str>, display: Option<&Profile>) -> Option<OutputColour> {
    let converted = convert(&profile(choice, display)?, NEUTRAL)?;
    Some(OutputColour {
        matrix: converted.matrix,
        encode: Encode::Table(Arc::from(converted.encode)),
        extended: false,
    })
}
