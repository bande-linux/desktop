use std::fs::File;
use std::io::BufWriter;
use std::process::ExitCode;

use web_content::{Fonts, Page, parse};

const WIDTH: u16 = 800;
const MAX_HEIGHT: u16 = 16384;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [input, output, rest @ ..] = arguments.as_slice() else {
        eprintln!("usage: render-png INPUT.html OUTPUT.png [WIDTH]");
        return ExitCode::FAILURE;
    };
    let width = rest
        .first()
        .and_then(|width| width.parse().ok())
        .unwrap_or(WIDTH);
    match render(input, output, width) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("render-png: {error}");
            ExitCode::FAILURE
        }
    }
}

fn render(input: &str, output: &str, width: u16) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(input)?;
    let mut fonts = Fonts::new();
    let started = std::time::Instant::now();
    let document = parse(&bytes);
    let parsed = started.elapsed();
    let mut page = Page::new(document, f32::from(width), &mut fonts);
    if let Some(directory) = std::path::Path::new(input).parent() {
        page.load_stylesheets_from(directory, &mut fonts);
        page.load_images_from(directory, &mut fonts);
    }
    let rendered = started.elapsed();
    let memory = page.memory();
    let per_element = |bytes: usize| bytes as f64 / memory.elements.max(1) as f64;
    eprintln!(
        "{} elements, parse {parsed:?}, total {rendered:?}, bytes per element: dom {:.0}, styles {:.0}, boxes {:.0}, fragments {:.0}, display list {:.0}, stylist {} KB, images {} KB",
        memory.elements,
        per_element(memory.dom),
        per_element(memory.styles),
        per_element(memory.boxes),
        per_element(memory.fragments),
        per_element(memory.display_list),
        memory.stylist / 1024,
        memory.images / 1024,
    );
    let height = (page.display_list.height.ceil() as u16).clamp(1, MAX_HEIGHT);
    let rgba = page.display_list.rasterize(0.0, width, height);
    let mut encoder = png::Encoder::new(
        BufWriter::new(File::create(output)?),
        u32::from(width),
        u32::from(height),
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.write_header()?.write_image_data(&rgba)?;
    Ok(())
}
