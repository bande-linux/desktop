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
    pub(crate) hdr: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Wanted {
    pub(crate) deep: bool,
    pub(crate) hdr: bool,
}

impl Shape {
    pub(crate) fn plain(buffers: Buffers) -> [Self; 2] {
        PLAIN.map(|format| Self {
            buffers,
            format,
            hdr: false,
        })
    }

    pub(crate) fn bits(self) -> u8 {
        match is_deep(self.format) {
            true => 10,
            false => PLAIN_BITS,
        }
    }

    pub(crate) fn is_deep(self) -> bool {
        self.bits() > PLAIN_BITS
    }

    pub(crate) fn needs_link_of(self, other: Self) -> bool {
        (self.bits(), self.hdr) == (other.bits(), other.hdr)
    }
}

impl fmt::Display for Shape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} {}", self.format, self.buffers)?;
        match self.hdr {
            true => f.write_str(" in HDR"),
            false => Ok(()),
        }
    }
}

pub(crate) fn is_deep(format: Fourcc) -> bool {
    DEEP.contains(&format)
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Sink {
    depth: Option<u8>,
    max_clock_khz: Option<u32>,
    hdr: bool,
}

impl Sink {
    pub(crate) fn new(edid: Option<&Edid>, over_hdmi: bool, hdr_signal: bool) -> Self {
        let hdr = hdr_signal && edid.is_some_and(|edid| edid.hdr.is_some());
        match over_hdmi {
            true => edid
                .and_then(|edid| edid.hdmi)
                .map_or_else(Self::default, |hdmi| Self {
                    depth: Some(hdmi.depth),
                    max_clock_khz: hdmi.max_clock_khz,
                    hdr,
                }),
            false => Self {
                depth: edid.and_then(|edid| edid.depth),
                max_clock_khz: None,
                hdr,
            },
        }
    }

    pub(crate) fn takes_hdr(self) -> bool {
        self.hdr && self.depth.is_some_and(|depth| depth > PLAIN_BITS)
    }

    #[cfg(test)]
    pub(crate) fn of_depth(depth: u8) -> Self {
        Self {
            depth: Some(depth),
            max_clock_khz: None,
            hdr: false,
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
    CommitWhole,
    LeaveMode,
    Fail,
}

impl Step {
    pub(crate) fn submission(self) -> Option<Submission> {
        match self {
            Self::ShowUnconverted | Self::ChangeShape | Self::CommitWhole => {
                Some(Submission::Retry)
            }
            Self::LeaveMode | Self::Fail => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Whole {
    Wanted,
    Committed,
}

#[derive(Debug, Clone)]
pub(crate) struct OutputFormat<M> {
    sink: Sink,
    start: Buffers,
    unusable: Vec<Shape>,
    refused: Vec<(M, Shape)>,
    chosen_for: Option<(M, Wanted)>,
    asked: Option<Wanted>,
    shown: Option<(M, Shape)>,
    kept: Option<(M, Wanted, Shape)>,
    whole: Option<(M, Shape, Whole)>,
}

impl<M: Copy + PartialEq> OutputFormat<M> {
    pub(crate) fn new(sink: Sink, start: Buffers) -> Self {
        Self {
            sink,
            start,
            unusable: Vec::new(),
            refused: Vec::new(),
            chosen_for: None,
            asked: None,
            shown: None,
            kept: None,
            whole: None,
        }
    }

    pub(crate) fn starts_tiled(&self) -> bool {
        self.start == Buffers::Tiled
    }

    pub(crate) fn follow(&mut self, sink: Sink) -> bool {
        std::mem::replace(&mut self.sink, sink) != sink
    }

    pub(crate) fn needs_choice(&self, mode: M, wanted: Wanted) -> bool {
        self.chosen_for != Some((mode, wanted))
    }

    pub(crate) fn ask(&mut self, wanted: Wanted) {
        if self
            .asked
            .replace(wanted)
            .is_some_and(|asked| asked != wanted)
        {
            self.forget_refusals();
        }
    }

    pub(crate) fn chosen(&mut self, mode: M, wanted: Wanted) {
        self.chosen_for = Some((mode, wanted));
    }

    pub(crate) fn forget_refusals(&mut self) {
        self.refused.clear();
        self.kept = None;
        self.whole = None;
        self.chosen_for = None;
    }

    pub(crate) fn candidates(&self, (mode, clock_khz): (M, u32), wanted: Wanted) -> Vec<Shape> {
        if let Some((kept_mode, kept_for, shape)) = self.kept
            && (kept_mode, kept_for) == (mode, wanted)
        {
            return vec![shape];
        }
        let tiled = |hdr: bool| {
            DEEP.into_iter().map(move |format| Shape {
                buffers: Buffers::Tiled,
                format,
                hdr,
            })
        };
        let in_hdr = tiled(true).filter(|_| wanted.hdr && self.sink.takes_hdr());
        in_hdr
            .chain(tiled(false).filter(|_| wanted.deep))
            .filter(|shape| self.sink.takes(shape.bits(), clock_khz))
            .chain(Shape::plain(Buffers::Tiled))
            .filter(|_| self.start == Buffers::Tiled)
            .chain(Shape::plain(Buffers::Linear))
            .filter(|_| self.start != Buffers::Dumb)
            .chain(Shape::plain(Buffers::Dumb))
            .filter(|shape| !self.unusable.contains(shape))
            .filter(|shape| !self.refused.contains(&(mode, *shape)))
            .collect()
    }

    pub(crate) fn keep(&mut self, mode: M, wanted: Wanted, shape: Shape) {
        self.kept = Some((mode, wanted, shape));
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

    pub(crate) fn commits_whole(&self, mode: M, shape: Shape) -> bool {
        self.whole == Some((mode, shape, Whole::Wanted))
    }

    pub(crate) fn committed_whole(&mut self, mode: M, shape: Shape) {
        self.whole = Some((mode, shape, Whole::Committed));
        self.shown = Some((mode, shape));
    }

    pub(crate) fn flipped(&mut self, mode: M, shape: Shape) {
        self.whole = None;
        self.shown = Some((mode, shape));
    }

    pub(crate) fn after(
        &mut self,
        failure: Failure,
        (mode, shape): (M, Shape),
        converting: bool,
    ) -> Step {
        match failure {
            Failure::Other => Step::Fail,
            Failure::Buffers if converting && !shape.hdr => Step::ShowUnconverted,
            Failure::Buffers if shape.buffers == Buffers::Dumb && !shape.is_deep() => Step::Fail,
            Failure::Buffers => {
                let worse = match shape.is_deep() {
                    true => vec![shape],
                    false => Shape::plain(shape.buffers).to_vec(),
                };
                self.unusable.extend(worse);
                self.chosen_for = None;
                Step::ChangeShape
            }
            Failure::Refused => self.after_refusal(mode, shape),
        }
    }

    fn after_refusal(&mut self, mode: M, shape: Shape) -> Step {
        let tried_whole = self
            .whole
            .take()
            .is_some_and(|(of, tried, _)| (of, tried) == (mode, shape));
        if !tried_whole {
            self.whole = Some((mode, shape, Whole::Wanted));
            return Step::CommitWhole;
        }
        let wanted = self.asked;
        if let Some(wanted) = wanted
            && self.kept == Some((mode, wanted, shape))
        {
            self.kept = None;
            self.refuse(mode, shape);
            return Step::LeaveMode;
        }
        if shape.is_deep() {
            self.refuse_depth(mode, shape);
            return Step::ChangeShape;
        }
        self.refuse(mode, shape);
        let shown = self
            .shown
            .filter(|shown| shown.0 == mode && shown.1 != shape);
        match wanted.zip(shown) {
            Some((wanted, (_, working))) => {
                self.keep(mode, wanted, working);
                Step::ChangeShape
            }
            None => Step::LeaveMode,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LinkDepth {
    pub(crate) found: u64,
    pub(crate) current: u64,
    pub(crate) range: (u64, u64),
}

impl LinkDepth {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Signalled {
    ours: bool,
    left: bool,
}

impl Signalled {
    pub(crate) fn found(left: bool) -> Self {
        Self { ours: false, left }
    }

    pub(crate) fn change(self, hdr: bool) -> Option<bool> {
        (self.ours != hdr || self.left).then_some(hdr)
    }

    pub(crate) fn set(&mut self, hdr: bool) {
        *self = Self {
            ours: hdr,
            left: false,
        };
    }

    pub(crate) fn renew(&mut self) {
        self.left = self.is_on();
    }

    pub(crate) fn is_ours(self) -> bool {
        self.ours
    }

    pub(crate) fn is_on(self) -> bool {
        self.ours || self.left
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signal_of_ours_is_taken_back_until_that_succeeds_and_one_left_there_is_replaced() {
        let mut signal = Signalled::found(false);
        assert_eq!((signal.change(false), signal.is_on()), (None, false));
        signal.set(true);
        let plain = first(&deep(8), FAST);
        assert_eq!(signal.change(plain.hdr), Some(false));
        assert!(signal.is_ours(), "a refused way back leaves it ours");
        signal.set(false);
        assert!(!signal.is_ours() && !signal.is_on());

        signal.set(true);
        signal.renew();
        assert_eq!(signal.change(true), Some(true), "other luminances");
        assert_eq!(signal.change(false), Some(false));
        assert!(signal.is_ours());
        signal.set(false);
        signal.renew();
        assert_eq!(signal.change(false), None);

        let left = Signalled::found(true);
        assert_eq!(left.change(true), Some(true));
        assert_eq!(left.change(false), Some(false));
        assert!(left.is_on() && !left.is_ours());
    }

    const DEEPEST: Wanted = Wanted {
        deep: true,
        hdr: false,
    };
    const SHALLOW: Wanted = Wanted {
        deep: false,
        hdr: false,
    };
    const FAST: (u8, u32) = (144, 575_030);
    const SLOW: (u8, u32) = (60, 243_480);

    fn deep(depth: u8) -> OutputFormat<u8> {
        OutputFormat::new(Sink::of_depth(depth), Buffers::Tiled)
    }

    fn first(formats: &OutputFormat<u8>, mode: (u8, u32)) -> Shape {
        formats.candidates(mode, DEEPEST)[0]
    }

    fn tiled(format: Fourcc) -> Shape {
        Shape {
            buffers: Buffers::Tiled,
            format,
            hdr: false,
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
        assert_eq!(deep(10).candidates(FAST, SHALLOW)[0], tiled(PLAIN[0]));
        for start in [Buffers::Linear, Buffers::Dumb] {
            let untiled = OutputFormat::<u8>::new(deep(10).sink, start);
            assert_eq!(first(&untiled, FAST), Shape::plain(start)[0]);
        }
        let hdmi = OutputFormat::new(
            Sink {
                depth: Some(12),
                max_clock_khz: Some(600_000),
                hdr: false,
            },
            Buffers::Tiled,
        );
        assert_eq!(first(&hdmi, SLOW), tiled(DEEP[0]));
        assert_eq!(first(&hdmi, FAST), tiled(PLAIN[0]));
        assert_eq!(
            first(&OutputFormat::new(Sink::default(), Buffers::Tiled), SLOW),
            tiled(PLAIN[0])
        );
    }

    #[test]
    fn hdr_is_tried_only_when_asked_and_able_and_falls_back_to_what_there_was_without_it() {
        let asked = Wanted {
            deep: true,
            hdr: true,
        };
        let able = Sink {
            hdr: true,
            ..Sink::of_depth(10)
        };
        let hdr = |format| Shape {
            hdr: true,
            ..tiled(format)
        };
        let mut formats = OutputFormat::new(able, Buffers::Tiled);
        let without = formats.candidates(FAST, DEEPEST);
        assert!(without.iter().all(|shape| !shape.hdr));
        for unable in [
            OutputFormat::new(Sink::of_depth(10), Buffers::Tiled),
            OutputFormat::new(able, Buffers::Linear),
            OutputFormat::new(
                Sink {
                    depth: Some(8),
                    ..able
                },
                Buffers::Tiled,
            ),
        ] {
            let offered = unable.candidates(FAST, asked);
            assert!(offered.iter().all(|shape| !shape.hdr), "{offered:?}");
        }
        assert_eq!(
            formats.candidates(FAST, asked),
            [vec![hdr(DEEP[0]), hdr(DEEP[1])], without.clone()].concat()
        );
        let only_hdr = Wanted {
            deep: false,
            ..asked
        };
        assert_eq!(formats.candidates(FAST, only_hdr)[0], hdr(DEEP[0]));
        formats.ask(asked);
        formats.chosen(FAST.0, asked);
        assert_eq!(
            formats.after(Failure::Buffers, (FAST.0, hdr(DEEP[0])), true),
            Step::ChangeShape,
            "an HDR signal never carries unconverted frames"
        );
        assert_eq!(refuse(&mut formats, FAST, hdr(DEEP[1])), Step::ChangeShape);
        assert_eq!(formats.candidates(FAST, asked), without);
    }

    fn refuse(formats: &mut OutputFormat<u8>, mode: (u8, u32), shape: Shape) -> Step {
        assert_eq!(
            formats.after(Failure::Refused, (mode.0, shape), false),
            Step::CommitWhole
        );
        assert!(formats.commits_whole(mode.0, shape));
        formats.after(Failure::Refused, (mode.0, shape), false)
    }

    #[test]
    fn a_refused_depth_keeps_the_mode_and_is_asked_again_in_another() {
        let mut formats = deep(10);
        formats.ask(DEEPEST);
        formats.chosen(FAST.0, DEEPEST);
        let deepest = first(&formats, FAST);
        assert_eq!(refuse(&mut formats, FAST, deepest), Step::ChangeShape);
        assert!(formats.needs_choice(FAST.0, DEEPEST));
        assert_eq!(first(&formats, FAST), tiled(PLAIN[0]));
        assert_eq!(first(&formats, SLOW), tiled(DEEP[0]));
        assert_eq!(refuse(&mut formats, FAST, tiled(PLAIN[0])), Step::LeaveMode);
        formats.forget_refusals();
        assert_eq!(first(&formats, FAST), tiled(DEEP[0]));
    }

    #[test]
    fn a_shape_the_display_never_takes_leaves_the_one_it_shows_in_place() {
        let mut formats = deep(10);
        let (deepest, plain) = (tiled(DEEP[0]), tiled(PLAIN[0]));
        formats.ask(DEEPEST);
        formats.chosen(FAST.0, DEEPEST);
        formats.flipped(FAST.0, deepest);
        formats.ask(SHALLOW);
        formats.chosen(FAST.0, SHALLOW);
        assert_eq!(refuse(&mut formats, FAST, plain), Step::ChangeShape);
        assert_eq!(formats.candidates(FAST, SHALLOW), [deepest]);
        formats.chosen(FAST.0, SHALLOW);
        assert!(!formats.needs_choice(FAST.0, SHALLOW));
        assert_eq!(formats.candidates(FAST, DEEPEST)[0], deepest);
        assert_eq!(formats.candidates(SLOW, SHALLOW)[0], plain);
        let mut asked_again = formats.clone();
        asked_again.ask(DEEPEST);
        asked_again.ask(SHALLOW);
        assert_eq!(asked_again.candidates(FAST, SHALLOW)[0], plain);
        assert_eq!(
            refuse(&mut formats, FAST, deepest),
            Step::LeaveMode,
            "nothing is left when the kept shape is refused too"
        );
    }

    #[test]
    fn a_shape_that_needs_a_mode_set_is_committed_whole_once_and_shown() {
        let mut formats = deep(10);
        let deepest = tiled(DEEP[0]);
        formats.chosen(FAST.0, DEEPEST);
        assert_eq!(
            formats.after(Failure::Refused, (FAST.0, deepest), false),
            Step::CommitWhole
        );
        formats.committed_whole(FAST.0, deepest);
        assert!(!formats.commits_whole(FAST.0, deepest));
        formats.flipped(FAST.0, deepest);
        assert!(!formats.needs_choice(FAST.0, DEEPEST));
        assert_eq!(
            formats.after(Failure::Refused, (FAST.0, deepest), false),
            Step::CommitWhole
        );
        formats.committed_whole(FAST.0, deepest);
        assert_eq!(
            formats.after(Failure::Refused, (FAST.0, deepest), false),
            Step::ChangeShape,
            "refused again right after a whole commit"
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
