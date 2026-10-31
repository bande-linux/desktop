use std::time::Duration;

use jiff::civil::Time;
use jiff::{Timestamp, Zoned};
use unbound_config::{ClockTime, NightLight as Settings, NightSchedule};
use unbound_datetime::daylight::{Place, Sky, sky, system_place};

use crate::state::Headless;

const NEUTRAL: f64 = 6500.0;
const LEAST_WARM: f64 = 5000.0;
const MOST_WARM: f64 = 2700.0;
const SCHEDULED_FADE: Duration = Duration::from_secs(60);
const SWITCHED_FADE: Duration = Duration::from_millis(1500);
const FADE_STEP: Duration = Duration::from_millis(50);
const RECHECK: Duration = Duration::from_secs(15 * 60);

fn temperature(warmth: u32) -> f64 {
    LEAST_WARM + (MOST_WARM - LEAST_WARM) * f64::from(warmth.min(100)) / 100.0
}

fn white_point(kelvin: f64) -> [f64; 3] {
    let t = kelvin / 100.0;
    let red = if t <= 66.0 {
        1.0
    } else {
        329.698_727_446 * (t - 60.0).powf(-0.133_204_759_2) / 255.0
    };
    let green = if t <= 66.0 {
        (99.470_802_586_1 * t.ln() - 161.119_568_166_1) / 255.0
    } else {
        288.122_169_528_3 * (t - 60.0).powf(-0.075_514_849_2) / 255.0
    };
    let blue = if t >= 66.0 {
        1.0
    } else if t <= 19.0 {
        0.0
    } else {
        (138.517_731_223_1 * (t - 10.0).ln() - 305.044_792_730_7) / 255.0
    };
    [red, green, blue].map(|gain| gain.clamp(0.0, 1.0))
}

fn gains(kelvin: f64) -> [f64; 3] {
    let neutral = white_point(NEUTRAL);
    let warm = white_point(kelvin);
    [0, 1, 2].map(|channel| (warm[channel] / neutral[channel]).min(1.0))
}

pub(crate) fn matrix(kelvin: f64) -> [u64; 9] {
    let [red, green, blue] =
        gains(kelvin).map(|gain| (gain * f64::from(1u32 << 31) * 2.0).round() as u64);
    [red, 0, 0, 0, green, 0, 0, 0, blue]
}

pub(crate) fn ramp(size: usize, kelvin: f64) -> [Vec<u16>; 3] {
    let last = size.saturating_sub(1).max(1) as f64;
    gains(kelvin).map(|gain| {
        (0..size)
            .map(|step| (step as f64 / last * gain * f64::from(u16::MAX)).round() as u16)
            .collect()
    })
}

fn at(now: &Zoned, days: i64, clock: ClockTime) -> Option<Timestamp> {
    let date = now.date().checked_add(jiff::Span::new().days(days)).ok()?;
    let time = Time::new(
        i8::try_from(clock.hour).ok()?,
        i8::try_from(clock.minute).ok()?,
        0,
        0,
    )
    .ok()?;
    date.to_datetime(time)
        .to_zoned(now.time_zone().clone())
        .ok()
        .map(|zoned| zoned.timestamp())
}

fn custom_night(now: &Zoned, from: ClockTime, to: ClockTime) -> Option<(bool, Timestamp)> {
    let minutes = |clock: ClockTime| u16::from(clock.hour) * 60 + u16::from(clock.minute);
    let current = u16::try_from(now.hour()).ok()? * 60 + u16::try_from(now.minute()).ok()?;
    let (start, end) = (minutes(from), minutes(to));
    let night = if start <= end {
        (start..end).contains(&current)
    } else {
        current >= start || current < end
    };
    let instant = now.timestamp();
    [(0, from), (0, to), (1, from), (1, to)]
        .into_iter()
        .filter_map(|(days, clock)| at(now, days, clock))
        .filter(|boundary| *boundary > instant)
        .min()
        .map(|next| (night, next))
}

fn scheduled_night(
    settings: &Settings,
    now: &Zoned,
    place: Option<Place>,
) -> Option<(bool, Timestamp)> {
    match settings.schedule {
        NightSchedule::Sunset => sky(now, place).map(|(sky, next)| (sky == Sky::Night, next)),
        NightSchedule::Custom => custom_night(now, settings.from, settings.to),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Fade {
    from: f64,
    to: f64,
    start: Duration,
    length: Duration,
}

impl Fade {
    fn at(&self, now: Duration) -> f64 {
        let progress = if self.length.is_zero() {
            1.0
        } else {
            (now.saturating_sub(self.start).as_secs_f64() / self.length.as_secs_f64()).min(1.0)
        };
        let eased = progress * progress * (3.0 - 2.0 * progress);
        self.from + (self.to - self.from) * eased
    }

    fn done(&self, now: Duration) -> bool {
        now >= self.start + self.length
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct NightLight {
    settings: Settings,
    strength: f64,
    target: f64,
    fade: Option<Fade>,
    check_at: Option<Duration>,
    switched: bool,
    started: bool,
    applied: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct NightTick {
    pub kelvin: Option<f64>,
    pub wake_at: Option<Duration>,
}

impl NightLight {
    pub(crate) fn configure(&mut self, settings: Settings) {
        if settings == self.settings && self.started {
            return;
        }
        self.switched |= settings.on != self.settings.on
            || settings.schedule != self.settings.schedule
            || settings.from != self.settings.from
            || settings.to != self.settings.to;
        self.settings = settings;
        self.check_at = Some(Duration::ZERO);
    }

    pub(crate) fn recheck(&mut self) {
        self.check_at = Some(Duration::ZERO);
    }

    fn check(&mut self, now: Duration, wall: &Zoned, place: Option<Place>) {
        let scheduled = self
            .settings
            .on
            .then(|| scheduled_night(&self.settings, wall, place))
            .flatten();
        self.check_at = self.settings.on.then(|| {
            let until = scheduled
                .and_then(|(_, next)| {
                    Duration::try_from(next.duration_since(wall.timestamp())).ok()
                })
                .unwrap_or(RECHECK);
            now + until.clamp(Duration::from_secs(1), RECHECK)
        });
        let night = scheduled.is_some_and(|(night, _)| night);
        let target = if night { 1.0 } else { 0.0 };
        if target != self.target {
            let length = match (self.started, self.switched) {
                (false, _) => Duration::ZERO,
                (true, true) => SWITCHED_FADE,
                (true, false) => SCHEDULED_FADE,
            };
            self.fade = Some(Fade {
                from: self.strength,
                to: target,
                start: now,
                length,
            });
            self.target = target;
        }
        self.started = true;
        self.switched = false;
    }

    fn advance(
        &mut self,
        now: Duration,
        wall: impl FnOnce() -> (Zoned, Option<Place>),
    ) -> NightTick {
        if self.check_at.is_some_and(|at| at <= now) {
            let (wall, place) = wall();
            self.check(now, &wall, place);
        }
        if let Some(fade) = self.fade {
            self.strength = fade.at(now);
            if fade.done(now) {
                self.fade = None;
            }
        }
        let kelvin = NEUTRAL + (temperature(self.settings.warmth) - NEUTRAL) * self.strength;
        let changed = self.applied.is_none_or(|applied| {
            (applied - kelvin).abs() >= 1.0 || (self.fade.is_none() && applied != kelvin)
        });
        if changed {
            self.applied = Some(kelvin);
        }
        NightTick {
            kelvin: changed.then_some(kelvin),
            wake_at: self
                .fade
                .map(|_| now + FADE_STEP)
                .into_iter()
                .chain(self.check_at)
                .min(),
        }
    }
}

impl Headless {
    pub fn night_light_tick(&mut self, now: Duration) -> NightTick {
        self.state
            .night_light
            .advance(now, || (Zoned::now(), system_place()))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use jiff::Zoned;
    use unbound_config::{ClockTime, NightLight as Settings, NightSchedule};
    use unbound_datetime::daylight::Place;

    use super::{
        NEUTRAL, NightLight, NightTick, custom_night, matrix, ramp, scheduled_night, temperature,
    };

    fn zoned(text: &str) -> Zoned {
        text.parse().expect("zoned")
    }

    fn clock(hour: u8, minute: u8) -> ClockTime {
        ClockTime { hour, minute }
    }

    fn local(zoned_text: &str, next: jiff::Timestamp) -> String {
        next.to_zoned(zoned(zoned_text).time_zone().clone())
            .strftime("%d %H:%M")
            .to_string()
    }

    const STOCKHOLM: Place = Place {
        latitude: 59.33,
        longitude: 18.05,
    };

    #[test]
    fn the_neutral_ramp_is_the_identity_and_a_warm_one_dims_blue_most() {
        let [red, green, blue] = ramp(256, NEUTRAL);
        let identity: Vec<u16> = (0..256u32).map(|step| (step * 257) as u16).collect();
        assert_eq!((&red, &green, &blue), (&identity, &identity, &identity));

        let [red, green, blue] = ramp(1024, temperature(100));
        assert_eq!(red[1023], u16::MAX);
        assert!(green[1023] < red[1023] && blue[1023] < green[1023]);
        assert!(blue[1023] > 0);
        assert!(red.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(blue.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(temperature(0) > temperature(100));
    }

    #[test]
    fn the_colour_matrix_scales_each_channel_in_fixed_point() {
        let one = 1u64 << 32;
        assert_eq!(matrix(NEUTRAL), [one, 0, 0, 0, one, 0, 0, 0, one]);
        let warm = matrix(temperature(100));
        assert_eq!(warm[0], one);
        assert!(warm[4] < one && warm[8] < warm[4] && warm[8] > 0);
        assert_eq!(
            [warm[1], warm[2], warm[3], warm[5], warm[6], warm[7]],
            [0; 6]
        );
    }

    #[test]
    fn a_custom_schedule_spans_midnight_and_names_the_next_change() {
        let evening = "2026-10-31T23:10[Europe/Stockholm]";
        let (night, next) =
            custom_night(&zoned(evening), clock(22, 0), clock(7, 0)).expect("night");
        assert!(night);
        assert_eq!(local(evening, next), "01 07:00");

        let noon = "2026-10-31T12:00[Europe/Stockholm]";
        let (night, next) = custom_night(&zoned(noon), clock(22, 0), clock(7, 0)).expect("day");
        assert!(!night);
        assert_eq!(local(noon, next), "31 22:00");

        let dawn = "2026-10-31T06:59[Europe/Stockholm]";
        assert_eq!(
            custom_night(&zoned(dawn), clock(22, 0), clock(7, 0)).map(|(night, _)| night),
            Some(true)
        );
        let afternoon = "2026-10-31T15:00[Europe/Stockholm]";
        let (night, next) =
            custom_night(&zoned(afternoon), clock(13, 0), clock(16, 30)).expect("nap");
        assert!(night);
        assert_eq!(local(afternoon, next), "31 16:30");
    }

    #[test]
    fn sunset_to_sunrise_follows_the_sun_at_the_place() {
        let settings = Settings {
            on: true,
            ..Settings::default()
        };
        assert_eq!(settings.schedule, NightSchedule::Sunset);
        let summer_night = zoned("2026-06-21T23:00[Europe/Stockholm]");
        let (night, sunrise) =
            scheduled_night(&settings, &summer_night, Some(STOCKHOLM)).expect("night");
        assert!(night);
        assert_eq!(
            local("2026-06-21T23:00[Europe/Stockholm]", sunrise).get(..5),
            Some("22 03")
        );
        let winter_afternoon = zoned("2026-12-21T14:00[Europe/Stockholm]");
        assert_eq!(
            scheduled_night(&settings, &winter_afternoon, Some(STOCKHOLM)).map(|(night, _)| night),
            Some(false)
        );
    }

    #[test]
    fn night_light_starts_at_once_fades_over_a_minute_at_the_schedule_and_quickly_when_switched() {
        let mut light = NightLight::default();
        let warm = temperature(Settings::default().warmth);
        let night_settings = Settings {
            on: true,
            schedule: NightSchedule::Custom,
            from: clock(22, 0),
            to: clock(7, 0),
            ..Settings::default()
        };
        let evening = || (zoned("2026-10-31T23:00[Europe/Stockholm]"), None);
        let morning = || (zoned("2026-11-01T07:00[Europe/Stockholm]"), None);

        light.configure(night_settings);
        let start = light.advance(Duration::from_secs(10), evening);
        assert_eq!(start.kelvin, Some(warm));

        light.recheck();
        let fading = light.advance(Duration::from_secs(20), morning);
        assert!(
            fading
                .wake_at
                .is_some_and(|at| at <= Duration::from_millis(20_050))
        );
        let halfway = light
            .advance(Duration::from_secs(50), morning)
            .kelvin
            .expect("moved");
        assert!(halfway > warm && halfway < NEUTRAL);
        assert_eq!(
            light.advance(Duration::from_secs(81), morning).kelvin,
            Some(NEUTRAL)
        );

        light.recheck();
        light.advance(Duration::from_secs(90), evening);
        light.advance(Duration::from_secs(200), evening);
        light.configure(Settings {
            on: false,
            ..night_settings
        });
        light.advance(Duration::from_secs(201), evening);
        assert_eq!(
            light.advance(Duration::from_millis(202_600), evening),
            NightTick {
                kelvin: Some(NEUTRAL),
                wake_at: None
            }
        );

        light.configure(night_settings);
        light.advance(Duration::from_secs(300), evening);
        light.advance(Duration::from_secs(302), evening);
        light.configure(Settings {
            warmth: 100,
            ..night_settings
        });
        assert_eq!(
            light.advance(Duration::from_secs(303), evening).kelvin,
            Some(temperature(100))
        );
    }
}
