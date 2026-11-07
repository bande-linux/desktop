use bande_edid::{Edid, Primaries};
use bande_render::OutputColour;

type Matrix = [[f64; 3]; 3];
type Point = (f64, f64);

const SRGB_PRIMARIES: [Point; 3] = [(0.64, 0.33), (0.30, 0.60), (0.15, 0.06)];
const D65: Point = (0.3127, 0.3290);
const COORDINATE_SCALE: f64 = 1024.0;
const SAME_AS_SRGB: f64 = 0.02;

fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    std::array::from_fn(|row| {
        std::array::from_fn(|column| (0..3).map(|k| a[row][k] * b[k][column]).sum())
    })
}

fn inverse(m: &Matrix) -> Option<Matrix> {
    let cofactor =
        |r0: usize, c0: usize, r1: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    let determinant = m[0][0] * cofactor(1, 1, 2, 2) - m[0][1] * cofactor(1, 0, 2, 2)
        + m[0][2] * cofactor(1, 0, 2, 1);
    (determinant.abs() > 1e-9).then(|| {
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

fn to_xyz(primaries: &[Point; 3]) -> Option<Matrix> {
    let column = |(x, y): Point| [x / y, 1.0, (1.0 - x - y) / y];
    let [red, green, blue] = primaries.map(column);
    let chromaticities = [
        [red[0], green[0], blue[0]],
        [red[1], green[1], blue[1]],
        [red[2], green[2], blue[2]],
    ];
    let white = column(D65);
    let invert = inverse(&chromaticities)?;
    let gains: [f64; 3] =
        std::array::from_fn(|row| (0..3).map(|k| invert[row][k] * white[k]).sum());
    Some(chromaticities.map(|row| std::array::from_fn(|k| row[k] * gains[k])))
}

fn points(primaries: Primaries) -> [Point; 3] {
    [primaries.red, primaries.green, primaries.blue].map(|(x, y)| {
        (
            f64::from(x) / COORDINATE_SCALE,
            f64::from(y) / COORDINATE_SCALE,
        )
    })
}

pub(crate) fn srgb_to_display(edid: &Edid) -> Option<OutputColour> {
    if edid.default_is_srgb {
        return None;
    }
    let display = points(edid.primaries);
    let differs = display.iter().zip(&SRGB_PRIMARIES).any(|(ours, srgb)| {
        (ours.0 - srgb.0).abs() > SAME_AS_SRGB || (ours.1 - srgb.1).abs() > SAME_AS_SRGB
    });
    if !differs {
        return None;
    }
    let matrix = multiply(&inverse(&to_xyz(&display)?)?, &to_xyz(&SRGB_PRIMARIES)?);
    Some(matrix.map(|row| row.map(|value| value as f32)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edid(red: (u16, u16), green: (u16, u16), blue: (u16, u16), default_is_srgb: bool) -> Edid {
        Edid {
            make: String::new(),
            model: String::new(),
            serial: String::new(),
            primaries: Primaries { red, green, blue },
            default_is_srgb,
        }
    }

    #[test]
    fn a_wide_gamut_display_keeps_white_and_narrows_red() {
        let matrix =
            srgb_to_display(&edid((690, 325), (287, 676), (146, 46), false)).expect("a conversion");
        for row in matrix {
            assert!((row.iter().sum::<f32>() - 1.0).abs() < 1e-3, "{row:?}");
        }
        assert!(matrix[0][0] < 0.95, "{matrix:?}");
    }

    #[test]
    fn an_srgb_display_is_left_alone() {
        assert_eq!(
            srgb_to_display(&edid((690, 325), (287, 676), (146, 46), true)),
            None
        );
        assert_eq!(
            srgb_to_display(&edid((655, 338), (307, 614), (154, 61), false)),
            None
        );
    }
}
