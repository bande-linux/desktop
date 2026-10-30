const WALLPAPER: u8 = 0x80;

pub fn check_golden(name: &str, size: (u32, u32), rgba: Vec<u8>) {
    let over_wallpaper = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| {
            let behind = u16::from(255 - pixel[3]) * u16::from(WALLPAPER) / 255;
            [0, 1, 2].map(|channel| (u16::from(pixel[channel]) + behind).min(255) as u8)
        })
        .collect();
    let actual = unbound_golden::RgbImage::from_raw(size.0, size.1, over_wallpaper).expect("frame");
    unbound_golden::assert_golden!(name, &actual);
}
