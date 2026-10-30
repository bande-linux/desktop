pub struct Radius;

impl Radius {
    pub const XS: f32 = 4.0;
    pub const S: f32 = 6.0;
    pub const M: f32 = 8.0;
    pub const L: f32 = 10.0;
    pub const XL: f32 = 14.0;

    pub fn concentric(outer: f32, inset: f32) -> f32 {
        (outer - inset).max(Self::XS)
    }
}
