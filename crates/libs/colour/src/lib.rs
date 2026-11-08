use std::fmt;

pub type Matrix = [[f64; 3]; 3];
pub type Chromaticity = (f64, f64);

pub const ENCODE_ENTRIES: usize = 4096;
pub const DISPLAY: &str = "display";
pub const SRGB: &str = "srgb";
pub const DISPLAY_P3: &str = "display-p3";
pub const UNMANAGED: &str = "none";
pub const PROFILE_FOLDER: &str = "icc";

const D50: [f64; 3] = [0.9642, 1.0, 0.8249];
const D65: Chromaticity = (0.3127, 0.3290);
const SRGB_PRIMARIES: [Chromaticity; 3] = [(0.64, 0.33), (0.30, 0.60), (0.15, 0.06)];
const P3_PRIMARIES: [Chromaticity; 3] = [(0.680, 0.320), (0.265, 0.690), (0.150, 0.060)];
const BRADFORD: Matrix = [
    [0.8951, 0.2664, -0.1614],
    [-0.7502, 1.7135, 0.0367],
    [0.0389, -0.0685, 1.0296],
];
const SAME: f64 = 1e-3;
const BISECTIONS: usize = 40;

#[derive(Debug, Clone, PartialEq)]
pub enum Curve {
    Srgb,
    Gamma(f64),
    Parametric { kind: u16, parameters: [f64; 7] },
    Table(Vec<f64>),
}

impl Curve {
    pub fn to_linear(&self, encoded: f64) -> f64 {
        let x = encoded.clamp(0.0, 1.0);
        let y = match self {
            Self::Srgb if x <= 0.04045 => x / 12.92,
            Self::Srgb => ((x + 0.055) / 1.055).powf(2.4),
            Self::Gamma(gamma) => x.powf(*gamma),
            Self::Parametric { kind, parameters } => parametric(*kind, parameters, x),
            Self::Table(values) => match values.len() {
                0 => x,
                1 => values[0],
                count => {
                    let at = x * (count - 1) as f64;
                    let low = at.floor() as usize;
                    let high = (low + 1).min(count - 1);
                    values[low] + (values[high] - values[low]) * (at - low as f64)
                }
            },
        };
        y.clamp(0.0, 1.0)
    }

    pub fn to_encoded(&self, linear: f64) -> f64 {
        let target = linear.clamp(0.0, 1.0);
        match self {
            Self::Srgb if target <= 0.0031308 => target * 12.92,
            Self::Srgb => 1.055 * target.powf(1.0 / 2.4) - 0.055,
            Self::Gamma(gamma) => target.powf(1.0 / gamma),
            _ => {
                let (mut low, mut high) = (0.0, 1.0);
                for _ in 0..BISECTIONS {
                    let middle = (low + high) / 2.0;
                    if self.to_linear(middle) < target {
                        low = middle;
                    } else {
                        high = middle;
                    }
                }
                (low + high) / 2.0
            }
        }
    }
}

fn parametric(kind: u16, p: &[f64; 7], x: f64) -> f64 {
    let (g, a, b, c, d, e, f) = (p[0], p[1], p[2], p[3], p[4], p[5], p[6]);
    let power = |base: f64| base.max(0.0).powf(g);
    match kind {
        0 => power(x),
        1 if x >= -b / a => power(a * x + b),
        1 => 0.0,
        2 if x >= -b / a => power(a * x + b) + c,
        2 => c,
        3 if x >= d => power(a * x + b),
        3 => c * x,
        4 if x >= d => power(a * x + b) + e,
        _ => c * x + f,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub to_pcs: Matrix,
    pub curves: [Curve; 3],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IccError {
    TooShort,
    NotADisplayProfile,
    NotRgb,
    NotXyz,
    LookupTables,
    MissingTag(&'static str),
    UnsupportedCurve,
}

impl fmt::Display for IccError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort => write!(f, "the file is too short to be an ICC profile"),
            Self::NotADisplayProfile => write!(f, "it is not a display profile"),
            Self::NotRgb => write!(f, "it does not describe RGB colors"),
            Self::NotXyz => write!(f, "its connection space is not XYZ"),
            Self::LookupTables => {
                write!(
                    f,
                    "it uses lookup tables; only matrix and curve profiles are supported"
                )
            }
            Self::MissingTag(tag) => write!(f, "it has no {tag} tag"),
            Self::UnsupportedCurve => write!(f, "it has a tone curve type that is not supported"),
        }
    }
}

impl std::error::Error for IccError {}

pub fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    std::array::from_fn(|row| {
        std::array::from_fn(|column| (0..3).map(|k| a[row][k] * b[k][column]).sum())
    })
}

fn apply(m: &Matrix, v: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|row| (0..3).map(|k| m[row][k] * v[k]).sum())
}

pub fn inverse(m: &Matrix) -> Option<Matrix> {
    let cofactor =
        |r0: usize, c0: usize, r1: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    let determinant = m[0][0] * cofactor(1, 1, 2, 2) - m[0][1] * cofactor(1, 0, 2, 2)
        + m[0][2] * cofactor(1, 0, 2, 1);
    (determinant.abs() > 1e-12).then(|| {
        [
            [
                cofactor(1, 1, 2, 2),
                cofactor(0, 2, 2, 1),
                cofactor(0, 1, 1, 2),
            ],
            [
                cofactor(1, 2, 2, 0),
                cofactor(0, 0, 2, 2),
                cofactor(0, 2, 1, 0),
            ],
            [
                cofactor(1, 0, 2, 1),
                cofactor(0, 1, 2, 0),
                cofactor(0, 0, 1, 1),
            ],
        ]
        .map(|row| row.map(|value| value / determinant))
    })
}

fn xyz((x, y): Chromaticity) -> [f64; 3] {
    [x / y, 1.0, (1.0 - x - y) / y]
}

impl Profile {
    pub fn from_primaries(
        primaries: [Chromaticity; 3],
        white: Chromaticity,
        curve: Curve,
    ) -> Option<Self> {
        let [red, green, blue] = primaries.map(xyz);
        let columns = [
            [red[0], green[0], blue[0]],
            [red[1], green[1], blue[1]],
            [red[2], green[2], blue[2]],
        ];
        let white = xyz(white);
        let gains = apply(&inverse(&columns)?, white);
        let to_xyz: Matrix = columns.map(|row| std::array::from_fn(|k| row[k] * gains[k]));
        let source_cone = apply(&BRADFORD, white);
        let target_cone = apply(&BRADFORD, D50);
        let scale: Matrix = std::array::from_fn(|row| {
            std::array::from_fn(|column| {
                if row == column {
                    target_cone[row] / source_cone[row]
                } else {
                    0.0
                }
            })
        });
        let adapt = multiply(&inverse(&BRADFORD)?, &multiply(&scale, &BRADFORD));
        Some(Self {
            to_pcs: multiply(&adapt, &to_xyz),
            curves: [curve.clone(), curve.clone(), curve],
        })
    }

    pub fn srgb() -> Self {
        Self::from_primaries(SRGB_PRIMARIES, D65, Curve::Srgb).unwrap_or(Self {
            to_pcs: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            curves: [Curve::Srgb, Curve::Srgb, Curve::Srgb],
        })
    }

    pub fn display_p3() -> Option<Self> {
        Self::from_primaries(P3_PRIMARIES, D65, Curve::Srgb)
    }

    pub fn parse_icc(bytes: &[u8]) -> Result<Self, IccError> {
        let u32_at = |at: usize| {
            bytes
                .get(at..at + 4)
                .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
                .ok_or(IccError::TooShort)
        };
        if bytes.len() < 132 {
            return Err(IccError::TooShort);
        }
        if &bytes[12..16] != b"mntr" {
            return Err(IccError::NotADisplayProfile);
        }
        if &bytes[16..20] != b"RGB " {
            return Err(IccError::NotRgb);
        }
        if &bytes[20..24] != b"XYZ " {
            return Err(IccError::NotXyz);
        }
        let count = u32_at(128)? as usize;
        let mut tags = Vec::with_capacity(count.min(256));
        for index in 0..count.min(256) {
            let at = 132 + index * 12;
            let signature = bytes.get(at..at + 4).ok_or(IccError::TooShort)?;
            let offset = u32_at(at + 4)? as usize;
            let size = u32_at(at + 8)? as usize;
            let data = bytes
                .get(offset..offset.saturating_add(size))
                .ok_or(IccError::TooShort)?;
            tags.push((signature, data));
        }
        let tag = |name: &'static str| {
            tags.iter()
                .find(|(signature, _)| *signature == name.as_bytes())
                .map(|(_, data)| *data)
        };
        if tag("rXYZ").is_none() && tag("A2B0").is_some() {
            return Err(IccError::LookupTables);
        }
        let colorant = |name: &'static str| -> Result<[f64; 3], IccError> {
            let data = tag(name).ok_or(IccError::MissingTag(name))?;
            if data.len() < 20 || &data[..4] != b"XYZ " {
                return Err(IccError::MissingTag(name));
            }
            Ok(std::array::from_fn(|k| fixed(&data[8 + 4 * k..12 + 4 * k])))
        };
        let [red, green, blue] = [colorant("rXYZ")?, colorant("gXYZ")?, colorant("bXYZ")?];
        let to_pcs = [
            [red[0], green[0], blue[0]],
            [red[1], green[1], blue[1]],
            [red[2], green[2], blue[2]],
        ];
        let curve = |name: &'static str| parse_curve(tag(name).ok_or(IccError::MissingTag(name))?);
        Ok(Self {
            to_pcs,
            curves: [curve("rTRC")?, curve("gTRC")?, curve("bTRC")?],
        })
    }

    pub fn to_icc(&self, description: &str) -> Vec<u8> {
        let mut tags: Vec<([u8; 4], Vec<u8>)> = Vec::new();
        tags.push((*b"desc", text(description)));
        tags.push((*b"cprt", text("No copyright, use freely")));
        tags.push((*b"wtpt", xyz_tag(D50)));
        for (name, column) in [(*b"rXYZ", 0), (*b"gXYZ", 1), (*b"bXYZ", 2)] {
            tags.push((
                name,
                xyz_tag(std::array::from_fn(|row| self.to_pcs[row][column])),
            ));
        }
        for (name, curve) in [*b"rTRC", *b"gTRC", *b"bTRC"].into_iter().zip(&self.curves) {
            tags.push((name, curve_tag(curve)));
        }
        let table_end = 132 + 12 * tags.len();
        let mut body = Vec::new();
        let mut entries = Vec::new();
        for (name, data) in &tags {
            let offset = table_end + body.len();
            entries.push((*name, offset, data.len()));
            body.extend_from_slice(data);
            while body.len() % 4 != 0 {
                body.push(0);
            }
        }
        let size = table_end + body.len();
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(&(size as u32).to_be_bytes());
        out.extend_from_slice(b"none");
        out.extend_from_slice(&[4, 0x30, 0, 0]);
        out.extend_from_slice(b"mntrRGB XYZ ");
        out.extend_from_slice(&[0; 12]);
        out.extend_from_slice(b"acsp");
        out.extend_from_slice(&[0; 24]);
        out.extend_from_slice(&[0, 0, 0, 0]);
        for value in D50 {
            out.extend_from_slice(&to_fixed(value));
        }
        out.extend_from_slice(&[0; 48]);
        out.extend_from_slice(&(tags.len() as u32).to_be_bytes());
        for (name, offset, length) in entries {
            out.extend_from_slice(&name);
            out.extend_from_slice(&(offset as u32).to_be_bytes());
            out.extend_from_slice(&(length as u32).to_be_bytes());
        }
        out.extend_from_slice(&body);
        out
    }

    fn is_srgb(&self) -> bool {
        let srgb = Self::srgb();
        self.curves.iter().all(|curve| *curve == Curve::Srgb)
            && self
                .to_pcs
                .iter()
                .flatten()
                .zip(srgb.to_pcs.iter().flatten())
                .all(|(ours, theirs)| (ours - theirs).abs() < SAME)
    }
}

fn fixed(bytes: &[u8]) -> f64 {
    f64::from(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])) / 65536.0
}

fn to_fixed(value: f64) -> [u8; 4] {
    ((value * 65536.0).round() as i32).to_be_bytes()
}

fn xyz_tag(value: [f64; 3]) -> Vec<u8> {
    let mut data = b"XYZ \0\0\0\0".to_vec();
    for component in value {
        data.extend_from_slice(&to_fixed(component));
    }
    data
}

fn text(value: &str) -> Vec<u8> {
    let utf16: Vec<u8> = value.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let mut data = b"mluc\0\0\0\0".to_vec();
    data.extend_from_slice(&1u32.to_be_bytes());
    data.extend_from_slice(&12u32.to_be_bytes());
    data.extend_from_slice(b"enUS");
    data.extend_from_slice(&(utf16.len() as u32).to_be_bytes());
    data.extend_from_slice(&28u32.to_be_bytes());
    data.extend_from_slice(&utf16);
    data
}

fn curve_tag(curve: &Curve) -> Vec<u8> {
    let (kind, parameters): (u16, Vec<f64>) = match curve {
        Curve::Srgb => (
            3,
            vec![2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045],
        ),
        Curve::Gamma(gamma) => (0, vec![*gamma]),
        Curve::Parametric { kind, parameters } => {
            let used = [1, 3, 4, 5, 7][usize::from(*kind).min(4)];
            (*kind, parameters[..used].to_vec())
        }
        Curve::Table(values) => {
            let mut data = b"curv\0\0\0\0".to_vec();
            data.extend_from_slice(&(values.len() as u32).to_be_bytes());
            for value in values {
                data.extend_from_slice(
                    &((value.clamp(0.0, 1.0) * 65535.0).round() as u16).to_be_bytes(),
                );
            }
            return data;
        }
    };
    let mut data = b"para\0\0\0\0".to_vec();
    data.extend_from_slice(&kind.to_be_bytes());
    data.extend_from_slice(&[0, 0]);
    for value in parameters {
        data.extend_from_slice(&to_fixed(value));
    }
    data
}

fn parse_curve(data: &[u8]) -> Result<Curve, IccError> {
    let u32_at = |at: usize| {
        data.get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or(IccError::TooShort)
    };
    match data.get(..4) {
        Some(b"curv") => {
            let count = u32_at(8)? as usize;
            let entry = |index: usize| -> Result<u16, IccError> {
                data.get(12 + 2 * index..14 + 2 * index)
                    .map(|b| u16::from_be_bytes([b[0], b[1]]))
                    .ok_or(IccError::TooShort)
            };
            match count {
                0 => Ok(Curve::Gamma(1.0)),
                1 => Ok(Curve::Gamma(f64::from(entry(0)?) / 256.0)),
                _ => (0..count)
                    .map(|index| entry(index).map(|value| f64::from(value) / 65535.0))
                    .collect::<Result<Vec<_>, _>>()
                    .map(Curve::Table),
            }
        }
        Some(b"para") => {
            let kind = data
                .get(8..10)
                .map(|b| u16::from_be_bytes([b[0], b[1]]))
                .ok_or(IccError::TooShort)?;
            let used = *[1usize, 3, 4, 5, 7]
                .get(usize::from(kind))
                .ok_or(IccError::UnsupportedCurve)?;
            let mut parameters = [0.0; 7];
            for (index, parameter) in parameters.iter_mut().take(used).enumerate() {
                *parameter = fixed(
                    data.get(12 + 4 * index..16 + 4 * index)
                        .ok_or(IccError::TooShort)?,
                );
            }
            Ok(Curve::Parametric { kind, parameters })
        }
        _ => Err(IccError::UnsupportedCurve),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Conversion {
    pub matrix: [[f32; 3]; 3],
    pub encode: Vec<[u8; 3]>,
}

pub fn conversion(display: &Profile, gains: [f64; 3]) -> Option<Conversion> {
    let neutral = gains.iter().all(|gain| (gain - 1.0).abs() < SAME);
    if neutral && display.is_srgb() {
        return None;
    }
    let matrix = multiply(&inverse(&display.to_pcs)?, &Profile::srgb().to_pcs);
    let warmed: Matrix = std::array::from_fn(|row| matrix[row].map(|value| value * gains[row]));
    let encode = (0..ENCODE_ENTRIES)
        .map(|index| {
            let position = index as f64 / (ENCODE_ENTRIES - 1) as f64;
            let linear = position * position;
            std::array::from_fn(|channel| {
                (display.curves[channel].to_encoded(linear) * 255.0).round() as u8
            })
        })
        .collect();
    Some(Conversion {
        matrix: warmed.map(|row| row.map(|value| value as f32)),
        encode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_profile_reads_back_and_converts_sensibly() {
        let wide = Profile::from_primaries(
            [(0.674, 0.317), (0.280, 0.660), (0.143, 0.045)],
            D65,
            Curve::Gamma(2.2),
        )
        .expect("a profile");
        let read = Profile::parse_icc(&wide.to_icc("Test")).expect("parses");
        for (ours, theirs) in read
            .to_pcs
            .iter()
            .flatten()
            .zip(wide.to_pcs.iter().flatten())
        {
            assert!((ours - theirs).abs() < 1e-4, "{ours} {theirs}");
        }
        assert!(matches!(read.curves[0], Curve::Parametric { kind: 0, .. }));
        let converted = conversion(&read, [1.0; 3]).expect("a conversion");
        for row in converted.matrix {
            assert!((row.iter().sum::<f32>() - 1.0).abs() < 1e-3, "{row:?}");
        }
        assert_eq!(converted.encode[ENCODE_ENTRIES - 1], [255, 255, 255]);
        assert!(conversion(&Profile::srgb(), [1.0; 3]).is_none());
        let mut not_display = Profile::srgb().to_icc("x");
        not_display[12..16].copy_from_slice(b"scnr");
        assert_eq!(
            Profile::parse_icc(&not_display),
            Err(IccError::NotADisplayProfile)
        );
    }
}
