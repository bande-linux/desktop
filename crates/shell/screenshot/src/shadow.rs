use unbound_toolkit::ScreenImage;

const BLUR: f64 = 22.0;
const DROP: f64 = 12.0;
const OPACITY: f32 = 0.5;
const PASSES: usize = 3;

fn box_blur(values: &mut [f32], width: usize, height: usize, radius: usize) {
    if radius == 0 || width == 0 || height == 0 {
        return;
    }
    let span = (2 * radius + 1) as f32;
    let mut line = Vec::new();
    for row in 0..height {
        line.clear();
        line.extend_from_slice(&values[row * width..(row + 1) * width]);
        let mut sum: f32 = line.iter().take(radius + 1).sum();
        for x in 0..width {
            values[row * width + x] = sum / span;
            if x + radius + 1 < width {
                sum += line[x + radius + 1];
            }
            if x >= radius {
                sum -= line[x - radius];
            }
        }
    }
    for column in 0..width {
        line.clear();
        line.extend((0..height).map(|row| values[row * width + column]));
        let mut sum: f32 = line.iter().take(radius + 1).sum();
        for y in 0..height {
            values[y * width + column] = sum / span;
            if y + radius + 1 < height {
                sum += line[y + radius + 1];
            }
            if y >= radius {
                sum -= line[y - radius];
            }
        }
    }
}

pub fn with_shadow(window: &ScreenImage, scale: f64) -> ScreenImage {
    let blur = (BLUR * scale).round() as usize;
    let drop = (DROP * scale).round() as usize;
    let margin = blur * 2;
    let (inner_width, inner_height) = (window.width as usize, window.height as usize);
    let width = inner_width + 2 * margin;
    let height = inner_height + 2 * margin + drop;
    let mut shadow = vec![0.0f32; width * height];
    for y in 0..inner_height {
        for x in 0..inner_width {
            let alpha = window.rgba[(y * inner_width + x) * 4 + 3];
            shadow[(y + margin + drop) * width + x + margin] = f32::from(alpha) / 255.0;
        }
    }
    for _ in 0..PASSES {
        box_blur(&mut shadow, width, height, blur / 2);
    }
    let mut rgba = vec![0u8; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            let below = shadow[y * width + x] * OPACITY;
            let inside =
                x >= margin && y >= margin && x < margin + inner_width && y < margin + inner_height;
            let [red, green, blue, alpha] = match inside {
                true => {
                    let at = ((y - margin) * inner_width + x - margin) * 4;
                    let pixel = &window.rgba[at..at + 4];
                    [pixel[0], pixel[1], pixel[2], pixel[3]]
                }
                false => [0, 0, 0, 0],
            };
            let over = f32::from(alpha) / 255.0;
            let combined = over + below * (1.0 - over);
            let at = (y * width + x) * 4;
            if combined <= 0.0 {
                continue;
            }
            let channel = |value: u8| (f32::from(value) * over / combined).round() as u8;
            rgba[at] = channel(red);
            rgba[at + 1] = channel(green);
            rgba[at + 2] = channel(blue);
            rgba[at + 3] = (combined * 255.0).round() as u8;
        }
    }
    ScreenImage {
        width: width as u32,
        height: height as u32,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_at(image: &ScreenImage, x: u32, y: u32) -> u8 {
        image.rgba[((y * image.width + x) * 4 + 3) as usize]
    }

    #[test]
    fn the_window_keeps_its_pixels_inside_a_soft_shadow_that_falls_below_it() {
        let window = ScreenImage {
            width: 40,
            height: 30,
            rgba: [0x20, 0x40, 0x60, 0xff].repeat(40 * 30),
        };

        let framed = with_shadow(&window, 1.0);

        let margin = 44;
        assert_eq!(
            (framed.width, framed.height),
            (40 + 2 * margin, 30 + 2 * margin + 12)
        );
        let center = ((margin + 15) * framed.width + margin + 20) as usize * 4;
        assert_eq!(&framed.rgba[center..center + 4], &[0x20, 0x40, 0x60, 0xff]);
        assert_eq!(alpha_at(&framed, 0, 0), 0);
        let above = alpha_at(&framed, margin + 20, margin - 6);
        let below = alpha_at(&framed, margin + 20, margin + 30 + 6);
        assert!(below > above, "below {below}, above {above}");
        assert_eq!(
            framed.rgba[(margin * framed.width + margin + 20) as usize * 4 + 1],
            0x40
        );
    }
}
