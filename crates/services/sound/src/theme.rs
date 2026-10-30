use std::f32::consts::TAU;

pub const RATE: u32 = 48_000;
const LOUDNESS: f32 = 0.1;
const PEAK: f32 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sound {
    Alert(Alert),
    Message,
    ScreenCapture,
    TrashEmpty,
    DeviceAdded,
    DeviceRemoved,
    BatteryLow,
    VolumeChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Alert {
    #[default]
    Mallet,
    Knock,
    Low,
    Tick,
}

impl Alert {
    pub const ALL: [Self; 4] = [Self::Mallet, Self::Knock, Self::Low, Self::Tick];

    pub fn keyword(self) -> &'static str {
        match self {
            Self::Mallet => "mallet",
            Self::Knock => "knock",
            Self::Low => "low",
            Self::Tick => "tick",
        }
    }

    pub fn from_keyword(keyword: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|alert| alert.keyword() == keyword)
    }
}

impl Sound {
    pub const THEME: [Self; 8] = [
        Self::Alert(Alert::Mallet),
        Self::Message,
        Self::ScreenCapture,
        Self::TrashEmpty,
        Self::DeviceAdded,
        Self::DeviceRemoved,
        Self::BatteryLow,
        Self::VolumeChange,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Alert(_) => "bell",
            Self::Message => "message-new-instant",
            Self::ScreenCapture => "screen-capture",
            Self::TrashEmpty => "trash-empty",
            Self::DeviceAdded => "device-added",
            Self::DeviceRemoved => "device-removed",
            Self::BatteryLow => "battery-low",
            Self::VolumeChange => "audio-volume-change",
        }
    }

    pub fn from_name(name: &str, alert: Alert) -> Option<Self> {
        match name {
            "bell" | "bell-window-system" | "dialog-warning" | "dialog-error" => {
                Some(Self::Alert(alert))
            }
            "message-new-instant" | "message-new-email" | "message" | "dialog-information" => {
                Some(Self::Message)
            }
            "screen-capture" | "camera-shutter" => Some(Self::ScreenCapture),
            "trash-empty" => Some(Self::TrashEmpty),
            "device-added" | "power-plug" => Some(Self::DeviceAdded),
            "device-removed" | "power-unplug" => Some(Self::DeviceRemoved),
            "battery-low" | "battery-caution" => Some(Self::BatteryLow),
            "audio-volume-change" => Some(Self::VolumeChange),
            _ => Alert::from_keyword(name.strip_prefix("alert-")?).map(Self::Alert),
        }
    }

    pub fn samples(self) -> Vec<f32> {
        let mut take = Take::new(match self {
            Self::Alert(Alert::Knock) | Self::DeviceAdded | Self::DeviceRemoved => 0.24,
            Self::Alert(Alert::Tick) => 0.05,
            Self::VolumeChange => 0.03,
            Self::ScreenCapture => 0.12,
            _ => 0.2,
        });
        match self {
            Self::Alert(Alert::Mallet) => {
                take.mallet(0.0, A4, 1.0);
                take.mallet(0.07, D5, 1.0);
            }
            Self::Alert(Alert::Knock) => {
                take.woodblock(0.0, D5, 1.0);
                take.woodblock(0.09, D5, 0.8);
            }
            Self::Alert(Alert::Low) => {
                take.mallet(0.0, D4, 1.0);
                take.mallet(0.08, A4, 0.9);
            }
            Self::Alert(Alert::Tick) => take.woodblock(0.0, A5, 1.0),
            Self::Message => {
                take.mallet(0.0, FS5, 1.0);
                take.mallet(0.05, B5, 0.45);
            }
            Self::ScreenCapture => {
                take.click(0.0, 2_600.0, 1.0);
                take.click(0.045, 1_800.0, 0.8);
                take.woodblock(0.045, D4, 0.35);
            }
            Self::TrashEmpty => {
                take.rustle(0.0, 0.11, 1.0);
                take.woodblock(0.1, D4, 0.7);
            }
            Self::DeviceAdded => {
                take.mallet(0.0, D5, 0.9);
                take.mallet(0.07, A5, 1.0);
            }
            Self::DeviceRemoved => {
                take.mallet(0.0, A5, 1.0);
                take.mallet(0.07, D5, 0.9);
            }
            Self::BatteryLow => {
                take.mallet(0.0, B4, 1.0);
                take.mallet(0.08, FS4, 1.0);
            }
            Self::VolumeChange => take.tick(D6),
        }
        take.mastered()
    }
}

const D4: f32 = 293.66;
const FS4: f32 = 369.99;
const A4: f32 = 440.0;
const B4: f32 = 493.88;
const D5: f32 = 587.33;
const FS5: f32 = 739.99;
const A5: f32 = 880.0;
const B5: f32 = 987.77;
const D6: f32 = 1_174.66;

struct Take {
    samples: Vec<f32>,
    noise: u32,
}

impl Take {
    fn new(seconds: f32) -> Self {
        Self {
            samples: vec![0.0; (seconds * RATE as f32) as usize],
            noise: 0x9E37_79B9,
        }
    }

    fn white(&mut self) -> f32 {
        self.noise ^= self.noise << 13;
        self.noise ^= self.noise >> 17;
        self.noise ^= self.noise << 5;
        self.noise as f32 / u32::MAX as f32 * 2.0 - 1.0
    }

    fn add(&mut self, start: f32, mut voice: impl FnMut(&mut Self, f32) -> f32) {
        let first = (start * RATE as f32) as usize;
        for index in first..self.samples.len() {
            let time = (index - first) as f32 / RATE as f32;
            let value = voice(self, time);
            self.samples[index] += value;
        }
    }

    fn partials(&mut self, start: f32, pitch: f32, gain: f32, partials: &[(f32, f32, f32)]) {
        let attack = 0.002;
        let partials = partials.to_vec();
        self.add(start, move |_, time| {
            let onset = (time / attack).min(1.0);
            let onset = 0.5 - 0.5 * (onset * std::f32::consts::PI).cos();
            gain * onset
                * partials
                    .iter()
                    .map(|(ratio, level, decay)| {
                        level * (-time / decay).exp() * (TAU * pitch * ratio * time).sin()
                    })
                    .sum::<f32>()
        });
    }

    fn mallet(&mut self, start: f32, pitch: f32, gain: f32) {
        self.partials(
            start,
            pitch,
            gain,
            &[(1.0, 1.0, 0.032), (3.93, 0.28, 0.01), (9.2, 0.08, 0.004)],
        );
    }

    fn woodblock(&mut self, start: f32, pitch: f32, gain: f32) {
        self.partials(
            start,
            pitch,
            gain,
            &[(1.0, 1.0, 0.012), (2.71, 0.45, 0.005), (5.4, 0.2, 0.002)],
        );
        self.click(start, pitch * 4.0, gain * 0.25);
    }

    fn tick(&mut self, pitch: f32) {
        self.partials(0.0, pitch, 1.0, &[(1.0, 1.0, 0.004), (2.71, 0.4, 0.002)]);
        self.click(0.0, pitch * 3.0, 0.2);
    }

    fn click(&mut self, start: f32, centre: f32, gain: f32) {
        let mut low = 0.0;
        let mut band = 0.0;
        let tune = 2.0 * (std::f32::consts::PI * centre / RATE as f32).sin();
        let damping = 0.6;
        self.add(start, move |take, time| {
            let input = take.white() * (-time / 0.003).exp();
            low += tune * band;
            let high = input - low - damping * band;
            band += tune * high;
            gain * band
        });
    }

    fn rustle(&mut self, start: f32, length: f32, gain: f32) {
        let mut smooth = 0.0;
        let mut previous = 0.0;
        self.add(start, move |take, time| {
            if time > length {
                return 0.0;
            }
            let shape = (std::f32::consts::PI * time / length).sin();
            let flutter = 0.55 + 0.45 * (TAU * 37.0 * time + 4.0 * (TAU * 11.0 * time).sin()).sin();
            let white = take.white();
            smooth += 0.35 * (white - smooth);
            let bright = smooth - previous;
            previous = smooth;
            gain * 0.6 * shape * flutter * bright
        });
    }

    fn mastered(mut self) -> Vec<f32> {
        let peak = self
            .samples
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        let active: Vec<f32> = self
            .samples
            .iter()
            .copied()
            .filter(|sample| sample.abs() > peak * 0.05)
            .collect();
        let rms = (active.iter().map(|sample| sample * sample).sum::<f32>()
            / active.len().max(1) as f32)
            .sqrt();
        if rms > 0.0 {
            let gain = (LOUDNESS / rms).min(PEAK / peak);
            for sample in &mut self.samples {
                *sample *= gain;
            }
        }
        let fade = (0.004 * RATE as f32) as usize;
        let length = self.samples.len();
        for (index, sample) in self
            .samples
            .iter_mut()
            .enumerate()
            .skip(length.saturating_sub(fade))
        {
            *sample *= (length - index) as f32 / fade as f32;
        }
        self.samples
    }
}

pub fn wav(samples: &[f32]) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
        wav.extend_from_slice(&value.to_le_bytes());
    }
    wav
}
