#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Self = Self::hex(0).with_alpha(0);
    pub const BLACK: Self = Self::hex(0x000000);
    pub const WHITE: Self = Self::hex(0xFFFFFF);

    pub const fn hex(rgb: u32) -> Self {
        let [_, r, g, b] = rgb.to_be_bytes();
        Self {
            r,
            g,
            b,
            a: u8::MAX,
        }
    }

    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    pub fn with_opacity(self, opacity: f32) -> Self {
        self.with_alpha((f32::from(self.a) * opacity.clamp(0.0, 1.0)).round() as u8)
    }

    pub fn mix(self, other: Self, amount: f32) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        let channel = |from: u8, to: u8| {
            (f32::from(from) + (f32::from(to) - f32::from(from)) * amount).round() as u8
        };
        Self {
            r: channel(self.r, other.r),
            g: channel(self.g, other.g),
            b: channel(self.b, other.b),
            a: channel(self.a, other.a),
        }
    }

    pub fn oklch(lightness: f64, chroma: f64, hue: f64) -> Self {
        let (a, b) = (
            chroma * hue.to_radians().cos(),
            chroma * hue.to_radians().sin(),
        );
        let l = (lightness + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
        let m = (lightness - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
        let s = (lightness - 0.089_484_177_5 * a - 1.291_485_548 * b).powi(3);
        let encode = |linear: f64| {
            let linear = linear.clamp(0.0, 1.0);
            let encoded = if linear <= 0.003_130_8 {
                12.92 * linear
            } else {
                1.055 * linear.powf(1.0 / 2.4) - 0.055
            };
            (encoded * 255.0).round_ties_even() as u8
        };
        Self {
            r: encode(4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s),
            g: encode(-1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s),
            b: encode(-0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701 * s),
            a: u8::MAX,
        }
    }

    pub(crate) fn oklab(self) -> [f64; 3] {
        let [r, g, b] = self.linear();
        let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
        let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
        let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
        [
            0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s,
            1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s,
            0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s,
        ]
    }

    pub(crate) fn linear(self) -> [f64; 3] {
        [self.r, self.g, self.b].map(|channel| {
            let value = f64::from(channel) / 255.0;
            if value <= 0.040_45 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        })
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::TRANSPARENT
    }
}
