use std::sync::Arc;

use bande_colour::{Curve, DISPLAY, DISPLAY_P3, Profile, SRGB, UNMANAGED, conversion as convert};
use bande_edid::Edid;
use bande_render::OutputColour;

const COORDINATE_SCALE: f64 = 1024.0;
const SRGB_PRIMARIES: [(f64, f64); 3] = [(0.64, 0.33), (0.30, 0.60), (0.15, 0.06)];
const SAME_AS_SRGB: f64 = 0.02;
const NEUTRAL: [f64; 3] = [1.0; 3];

fn point((x, y): (u16, u16)) -> (f64, f64) {
    (
        f64::from(x) / COORDINATE_SCALE,
        f64::from(y) / COORDINATE_SCALE,
    )
}

pub(crate) fn edid_profile(edid: &Edid) -> Option<Profile> {
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

const DCP_SRGB_SURFACES: &str = "/sys/module/appledrm/parameters/srgb_surfaces";

fn display_engine_converts() -> bool {
    std::fs::read_to_string(DCP_SRGB_SURFACES).is_ok_and(|value| value.trim() == "Y")
}

fn profile(choice: Option<&str>, display: Option<&Profile>) -> Option<Profile> {
    match choice.unwrap_or(DISPLAY) {
        DISPLAY if display_engine_converts() => None,
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

pub(crate) fn conversion(choice: Option<&str>, display: Option<&Profile>) -> Option<OutputColour> {
    let converted = convert(&profile(choice, display)?, NEUTRAL)?;
    Some(OutputColour {
        matrix: converted.matrix,
        encode: Some(Arc::from(converted.encode)),
    })
}
