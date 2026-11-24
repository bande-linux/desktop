use std::fmt;

use bande_edid::Edid;
use smithay::backend::allocator::Fourcc;

use crate::clock::Submission;

const DEEP: [Fourcc; 2] = [Fourcc::Xrgb2101010, Fourcc::Xbgr2101010];
const PLAIN: [Fourcc; 2] = [Fourcc::Xrgb8888, Fourcc::Argb8888];
const PLAIN_BITS: u8 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Buffers {
    Tiled,
    Linear,
    Dumb,
}

impl fmt::Display for Buffers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Tiled => "tiled GBM buffers",
            Self::Linear => "linear GBM buffers",
            Self::Dumb => "dumb buffers",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Shape {
    pub(crate) buffers: Buffers,
    pub(crate) format: Fourcc,
}

impl Shape {
    pub(crate) fn plain(buffers: Buffers) -> [Self; 2] {
        PLAIN.map(|format| Self { buffers, format })
    }

    pub(crate) fn bits(self) -> u8 {
        match DEEP.contains(&self.format) {
            true => 10,
            false => PLAIN_BITS,
        }
    }

    pub(crate) fn is_deep(self) -> bool {
        self.bits() > PLAIN_BITS
    }
}

impl fmt::Display for Shape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} {}", self.format, self.buffers)
    }
}

pub(crate) fn plain_formats() -> [Fourcc; 2] {
    PLAIN
}

pub(crate) fn fallback(buffers: Buffers) -> Option<Buffers> {
    match buffers {
        Buffers::Tiled => Some(Buffers::Linear),
        Buffers::Linear => Some(Buffers::Dumb),
        Buffers::Dumb => None,
    }
}

/// What a display says it takes over its connector.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Sink {
    depth: Option<u8>,
    max_clock_khz: Option<u32>,
}

impl Sink {
    pub(crate) fn new(edid: Option<&Edid>, over_hdmi: bool) -> Self {
        match over_hdmi {
            true => edid
                .and_then(|edid| edid.hdmi)
                .map_or_else(Self::default, |hdmi| Self {
                    depth: Some(hdmi.depth),
                    max_clock_khz: hdmi.max_clock_khz,
                }),
            false => Self {
                depth: edid.and_then(|edid| edid.depth),
                max_clock_khz: None,
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn of_depth(depth: u8) -> Self {
        Self {
            depth: Some(depth),
            max_clock_khz: None,
        }
    }

    fn takes(self, bits: u8, clock_khz: u32) -> bool {
        let wide = |clock: u32| u64::from(clock) * u64::from(bits) / u64::from(PLAIN_BITS);
        bits <= PLAIN_BITS
            || self.depth.is_some_and(|depth| bits <= depth)
                && self
                    .max_clock_khz
                    .is_none_or(|most| wide(clock_khz) <= u64::from(most))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    Buffers,
    Refused,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    ShowUnconverted,
    ChangeShape,
    LeaveMode,
    Fail,
}

impl Step {
    pub(crate) fn submission(self) -> Option<Submission> {
        match self {
            Self::ShowUnconverted | Self::ChangeShape => Some(Submission::Retry),
            Self::LeaveMode | Self::Fail => None,
        }
    }
}

/// The frames a display gets: the best shape it, its mode and its buffers have not refused.
#[derive(Debug, Clone)]
pub(crate) struct OutputFormat<M> {
    sink: Sink,
    tiled: bool,
    unusable: Vec<Shape>,
    refused: Vec<(M, Shape)>,
    chosen_for: Option<(M, bool)>,
}

impl<M: Copy + PartialEq> OutputFormat<M> {
    pub(crate) fn new(sink: Sink, tiled: bool) -> Self {
        Self {
            sink,
            tiled,
            unusable: Vec::new(),
            refused: Vec::new(),
            chosen_for: None,
        }
    }

    pub(crate) fn needs_choice(&self, mode: M, deep: bool) -> bool {
        self.chosen_for != Some((mode, deep))
    }

    pub(crate) fn chosen(&mut self, mode: M, deep: bool) {
        self.chosen_for = Some((mode, deep));
    }

    pub(crate) fn candidates(&self, (mode, clock_khz): (M, u32), deep: bool) -> Vec<Shape> {
        let tiled = DEEP.into_iter().map(|format| Shape {
            buffers: Buffers::Tiled,
            format,
        });
        tiled
            .filter(|shape| deep && self.sink.takes(shape.bits(), clock_khz))
            .chain(Shape::plain(Buffers::Tiled))
            .chain(Shape::plain(Buffers::Linear))
            .filter(|_| self.tiled)
            .chain(Shape::plain(Buffers::Dumb))
            .filter(|shape| !self.unusable.contains(shape))
            .filter(|shape| !self.refused.contains(&(mode, *shape)))
            .collect()
    }

    pub(crate) fn choose_again(&mut self) {
        self.chosen_for = None;
    }

    pub(crate) fn refuse_depth(&mut self, mode: M, shape: Shape) {
        for format in DEEP {
            self.refuse(mode, Shape { format, ..shape });
        }
    }

    pub(crate) fn refuse(&mut self, mode: M, shape: Shape) {
        if !self.refused.contains(&(mode, shape)) {
            self.refused.push((mode, shape));
        }
        self.chosen_for = None;
    }

    pub(crate) fn after(
        &mut self,
        failure: Failure,
        (mode, shape): (M, Shape),
        converting: bool,
    ) -> Step {
        match failure {
            Failure::Other => Step::Fail,
            Failure::Buffers if converting => Step::ShowUnconverted,
            Failure::Buffers if shape.buffers == Buffers::Dumb => Step::Fail,
            Failure::Buffers => {
                let worse = match shape.is_deep() {
                    true => vec![shape],
                    false => Shape::plain(shape.buffers).to_vec(),
                };
                self.unusable.extend(worse);
                self.chosen_for = None;
                Step::ChangeShape
            }
            Failure::Refused if shape.is_deep() => {
                self.refuse_depth(mode, shape);
                Step::ChangeShape
            }
            Failure::Refused => Step::LeaveMode,
        }
    }
}

/// A connector's `max bpc`: what it was when the display was found, and what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LinkDepth {
    pub(crate) found: u64,
    pub(crate) current: u64,
    pub(crate) range: (u64, u64),
}

impl LinkDepth {
    /// The value to set for frames of `bits`, if it has to change.
    pub(crate) fn change(self, bits: u8, deep: bool) -> Option<u64> {
        let (lowest, highest) = self.range;
        let wanted = match deep {
            true => self.found.max(u64::from(bits)),
            false => u64::from(PLAIN_BITS),
        };
        let wanted = wanted.clamp(lowest, highest.max(lowest));
        (wanted != self.current).then_some(wanted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAST: (u8, u32) = (144, 575_030);
    const SLOW: (u8, u32) = (60, 243_480);

    fn deep(depth: u8) -> OutputFormat<u8> {
        OutputFormat::new(Sink::of_depth(depth), true)
    }

    fn first(formats: &OutputFormat<u8>, mode: (u8, u32)) -> Shape {
        formats.candidates(mode, true)[0]
    }

    fn tiled(format: Fourcc) -> Shape {
        Shape {
            buffers: Buffers::Tiled,
            format,
        }
    }

    #[test]
    fn a_display_gets_the_deepest_frames_it_its_link_and_its_owner_allow() {
        for depth in [10, 12, 16] {
            assert_eq!(first(&deep(depth), FAST), tiled(DEEP[0]));
        }
        for depth in [6, 8] {
            assert_eq!(first(&deep(depth), FAST), tiled(PLAIN[0]));
        }
        assert_eq!(deep(10).candidates(FAST, false)[0], tiled(PLAIN[0]));
        let untiled = OutputFormat::<u8>::new(deep(10).sink, false);
        assert_eq!(first(&untiled, FAST), Shape::plain(Buffers::Dumb)[0]);
        let hdmi = OutputFormat::new(
            Sink {
                depth: Some(12),
                max_clock_khz: Some(600_000),
            },
            true,
        );
        assert_eq!(first(&hdmi, SLOW), tiled(DEEP[0]));
        assert_eq!(first(&hdmi, FAST), tiled(PLAIN[0]));
        assert_eq!(
            first(&OutputFormat::new(Sink::default(), true), SLOW),
            tiled(PLAIN[0])
        );
    }

    #[test]
    fn a_refused_depth_keeps_the_mode_and_is_asked_again_in_another() {
        let mut formats = deep(10);
        formats.chosen(FAST.0, true);
        let deepest = first(&formats, FAST);
        assert_eq!(
            formats.after(Failure::Refused, (FAST.0, deepest), false),
            Step::ChangeShape
        );
        assert!(formats.needs_choice(FAST.0, true));
        assert_eq!(first(&formats, FAST), tiled(PLAIN[0]));
        assert_eq!(first(&formats, SLOW), tiled(DEEP[0]));
        assert_eq!(
            formats.after(Failure::Refused, (FAST.0, tiled(PLAIN[0])), false),
            Step::LeaveMode
        );
    }

    #[test]
    fn failing_buffers_lose_depth_before_tiling_and_tiling_before_the_gpu() {
        let mut formats = deep(10);
        let mut seen = Vec::new();
        loop {
            let shape = first(&formats, FAST);
            seen.push(shape);
            match formats.after(Failure::Buffers, (FAST.0, shape), false) {
                Step::ChangeShape => {}
                step => {
                    assert_eq!(step, Step::Fail);
                    break;
                }
            }
        }
        assert_eq!(
            seen,
            [
                tiled(DEEP[0]),
                tiled(DEEP[1]),
                tiled(PLAIN[0]),
                Shape::plain(Buffers::Linear)[0],
                Shape::plain(Buffers::Dumb)[0],
            ]
        );
        assert_ne!(first(&formats, SLOW), tiled(DEEP[0]), "kept for good");
    }

    #[test]
    fn a_link_is_raised_for_deep_frames_lowered_for_its_owner_and_put_back() {
        let mut link = LinkDepth {
            found: 8,
            current: 8,
            range: (6, 12),
        };
        assert_eq!(link.change(8, true), None);
        assert_eq!(link.change(10, true), Some(10));
        link.current = 10;
        assert_eq!(link.change(10, true), None);
        assert_eq!(link.change(8, true), Some(8), "put back with 8-bit frames");
        let default = LinkDepth {
            found: 12,
            current: 12,
            range: (6, 12),
        };
        assert_eq!(default.change(10, true), None);
        assert_eq!(default.change(8, true), None);
        assert_eq!(default.change(8, false), Some(8));
        let narrow = LinkDepth {
            found: 8,
            current: 8,
            range: (8, 8),
        };
        assert_eq!(narrow.change(10, true), None);
        let reversed = LinkDepth {
            range: (9, 4),
            ..narrow
        };
        assert_eq!(reversed.change(8, true), Some(9));
    }
}
