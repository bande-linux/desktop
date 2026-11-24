use smithay::backend::allocator::Fourcc;

const DEEP: [Fourcc; 2] = [Fourcc::Xrgb2101010, Fourcc::Xbgr2101010];
const PLAIN: [Fourcc; 2] = [Fourcc::Xrgb8888, Fourcc::Argb8888];

pub(crate) fn bits(format: Fourcc) -> u64 {
    match DEEP.contains(&format) {
        true => 10,
        false => 8,
    }
}

pub(crate) fn is_deep(format: Fourcc) -> bool {
    DEEP.contains(&format)
}

/// The formats to try for a display that takes `depth` bits per channel, best first.
pub(crate) fn formats(depth: Option<u8>) -> Vec<Fourcc> {
    let depth = depth.map_or(0, u64::from);
    DEEP.into_iter()
        .filter(|format| bits(*format) <= depth)
        .chain(PLAIN)
        .collect()
}

/// What a connector's `max bpc` has to become to send `wanted` bits, when it is below them.
pub(crate) fn raised(current: u64, (lowest, highest): (u64, u64), wanted: u64) -> Option<u64> {
    let wanted = wanted.clamp(lowest, highest.max(lowest));
    (current < wanted).then_some(wanted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_display_gets_the_bits_it_takes_and_its_link_is_only_ever_raised() {
        assert_eq!(formats(None), PLAIN);
        assert_eq!(formats(Some(6)), PLAIN);
        assert_eq!(formats(Some(8)), PLAIN);
        for depth in [10, 12, 16] {
            assert_eq!(formats(Some(depth)), [DEEP, PLAIN].concat());
        }
        assert_eq!(raised(8, (6, 12), 10), Some(10));
        assert_eq!(raised(8, (8, 8), 10), None);
        assert_eq!(raised(12, (6, 12), 10), None);
        assert_eq!(raised(6, (6, 12), 8), Some(8));
        assert_eq!(raised(6, (6, 8), 10), Some(8));
        assert_eq!(raised(0, (9, 4), 8), Some(9));
    }
}
