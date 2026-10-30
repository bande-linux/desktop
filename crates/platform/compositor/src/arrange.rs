use smithay::utils::{Logical, Point, Transform};
use unbound_config::{Config, DisplaySettings, Placement, Rotation};

use crate::screen::{OutputConfig, OutputMode, OutputSettings, automatic_scale};
use crate::state::Rect;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Arranged {
    pub(crate) name: String,
    pub(crate) settings: OutputSettings,
    pub(crate) mirror: Option<String>,
}

pub(crate) fn identities(heads: &[OutputConfig]) -> Vec<String> {
    heads
        .iter()
        .map(|head| {
            let identity = head.identity();
            let shared = heads
                .iter()
                .filter(|other| other.identity() == identity)
                .count();
            if shared > 1 {
                format!("{identity} ({})", head.name)
            } else {
                identity
            }
        })
        .collect()
}

fn transform(rotation: Option<Rotation>) -> Transform {
    match rotation {
        None | Some(Rotation::Upright) => Transform::Normal,
        Some(Rotation::Quarter) => Transform::_90,
        Some(Rotation::Half) => Transform::_180,
        Some(Rotation::ThreeQuarters) => Transform::_270,
    }
}

pub(crate) fn chosen_mode(head: &OutputConfig, display: &DisplaySettings) -> Option<OutputMode> {
    let preferred = head.preferred_mode()?;
    let size = display
        .resolution
        .and_then(|(width, height)| {
            let size = (i32::try_from(width).ok()?, i32::try_from(height).ok()?);
            head.modes
                .iter()
                .any(|mode| (mode.size.w, mode.size.h) == size)
                .then_some(size)
        })
        .unwrap_or((preferred.size.w, preferred.size.h));
    let sized = head
        .modes
        .iter()
        .filter(|mode| (mode.size.w, mode.size.h) == size);
    match display.refresh_rate {
        Some(rate) => sized.min_by(|a, b| {
            let off = |mode: &&OutputMode| (1.0 / mode.refresh.as_secs_f64() - rate).abs();
            off(a).total_cmp(&off(b))
        }),
        None => sized.into_iter().next(),
    }
    .copied()
}

fn settings_for(head: &OutputConfig, display: &DisplaySettings) -> Option<OutputSettings> {
    let mode = chosen_mode(head, display)?;
    Some(OutputSettings {
        mode,
        scale: display
            .scale
            .unwrap_or_else(|| automatic_scale(mode.size, head.physical_size_mm)),
        transform: transform(display.rotation),
        position: Point::default(),
    })
}

fn rect(settings: &OutputSettings) -> Rect {
    Rect {
        location: settings.position,
        size: settings.logical_size(),
    }
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.location.x < b.location.x + b.size.w
        && b.location.x < a.location.x + a.size.w
        && a.location.y < b.location.y + b.size.h
        && b.location.y < a.location.y + a.size.h
}

fn side_by_side(arranged: &mut [Arranged], main: usize) {
    let mut right = arranged[main].settings.logical_size().w;
    arranged[main].settings.position = Point::default();
    for (index, display) in arranged.iter_mut().enumerate() {
        if index == main || display.mirror.is_some() {
            continue;
        }
        display.settings.position = (right, 0).into();
        right += display.settings.logical_size().w;
    }
}

fn best_known<'a>(config: &'a Config, identities: &[String]) -> Option<&'a [Placement]> {
    config.arrangement(identities).or_else(|| {
        config
            .arrangements
            .iter()
            .filter(|placements| {
                placements
                    .iter()
                    .all(|placement| identities.contains(&placement.display))
            })
            .max_by_key(|placements| placements.len())
            .map(Vec::as_slice)
    })
}

pub(crate) fn arrange(
    heads: &[OutputConfig],
    current_main: &str,
    config: &Config,
) -> Vec<Arranged> {
    let identities = identities(heads);
    let mut arranged: Vec<Arranged> = heads
        .iter()
        .zip(&identities)
        .filter_map(|(head, identity)| {
            let display = config.displays.get(identity).copied().unwrap_or_default();
            Some(Arranged {
                name: head.name.clone(),
                settings: settings_for(head, &display)?,
                mirror: None,
            })
        })
        .collect();
    if arranged.is_empty() {
        return arranged;
    }
    let identity_of = |name: &str| {
        heads
            .iter()
            .position(|head| head.name == name)
            .map(|index| identities[index].clone())
    };
    let name_of = |identity: &str| {
        identities
            .iter()
            .position(|known| known == identity)
            .map(|index| heads[index].name.clone())
    };
    let known = best_known(config, &identities);
    let placement = |display: &Arranged| {
        let identity = identity_of(&display.name)?;
        known?
            .iter()
            .find(|placement| placement.display == identity)
    };
    let main = arranged
        .iter()
        .position(|display| placement(display).is_some_and(|placement| placement.main))
        .or_else(|| {
            arranged
                .iter()
                .position(|display| display.name == current_main)
        })
        .unwrap_or(0);
    let mut unplaced = Vec::new();
    for (index, display) in arranged.iter_mut().enumerate() {
        match placement(display).cloned() {
            Some(placed) => {
                display.settings.position = (placed.x, placed.y).into();
                display.mirror = placed
                    .mirror
                    .as_deref()
                    .and_then(name_of)
                    .filter(|_| index != main);
            }
            None => unplaced.push(index),
        }
    }
    let origin: Point<i32, Logical> = arranged[main].settings.position;
    for display in &mut arranged {
        display.settings.position -= origin;
    }
    if unplaced.contains(&main) {
        arranged[main].settings.position = Point::default();
    }
    for index in unplaced.into_iter().filter(|index| *index != main) {
        let right = arranged
            .iter()
            .enumerate()
            .filter(|(other, display)| *other != index && display.mirror.is_none())
            .map(|(_, display)| rect(&display.settings))
            .map(|rect| rect.location.x + rect.size.w)
            .max()
            .unwrap_or(0);
        arranged[index].settings.position = (right, 0).into();
    }
    let shown: Vec<Rect> = arranged
        .iter()
        .filter(|display| display.mirror.is_none())
        .map(|display| rect(&display.settings))
        .collect();
    let collides = shown
        .iter()
        .enumerate()
        .any(|(index, a)| shown[index + 1..].iter().any(|b| overlaps(*a, *b)));
    if collides {
        side_by_side(&mut arranged, main);
    }
    for index in 0..arranged.len() {
        if let Some(source) = arranged[index].mirror.clone()
            && let Some(position) = arranged
                .iter()
                .find(|display| display.name == source)
                .map(|display| display.settings.position)
        {
            arranged[index].settings.position = position;
        }
    }
    arranged.swap(0, main);
    arranged
}
