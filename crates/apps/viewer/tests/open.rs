#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::time::{Duration, Instant};

use bande_toolkit::Appearance;
use bande_viewer::FileView;
use image::codecs::gif::GifEncoder;
use image::{Frame, Rgba, RgbaImage};

fn until(view: &mut FileView, wanted: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        view.pump();
        view.paint(800, 600);
        let shown = format!("{:?}", view.ui_ref().accessibility());
        if !view.is_loading() && shown.contains(wanted) {
            return shown;
        }
        assert!(Instant::now() < deadline, "no “{wanted}” in {shown}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

const PDF: [&str; 9] = [
    "<< /Type /Catalog /Pages 2 0 R /Outlines 7 0 R >>",
    "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 300 200] /Resources << /Font << /F1 9 0 R >> >> >>",
    "<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>",
    "<< /Type /Page /Parent 2 0 R /Contents 6 0 R >>",
    "<< /Length 41 >>\nstream\nBT /F1 18 Tf 20 150 Td (First page) Tj ET\nendstream",
    "<< /Length 42 >>\nstream\nBT /F1 18 Tf 20 150 Td (Hello Bande) Tj ET\nendstream",
    "<< /Type /Outlines /First 8 0 R /Last 8 0 R /Count 1 >>",
    "<< /Title (Chapter Two) /Parent 7 0 R /Dest [4 0 R /Fit] >>",
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
];

fn write_pdf(path: &Path) {
    let mut pdf = String::from("%PDF-1.7\n");
    let mut xref = format!("xref\n0 {}\n0000000000 65535 f \n", PDF.len() + 1);
    for (index, object) in PDF.iter().enumerate() {
        xref += &format!("{:010} 00000 n \n", pdf.len());
        pdf += &format!("{} 0 obj\n{object}\nendobj\n", index + 1);
    }
    let end = format!(
        "trailer << /Size 10 /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
        pdf.len()
    );
    std::fs::write(path, pdf + &xref + &end).unwrap();
}

#[test]
fn a_folder_steps_through_pictures_and_hostile_files_never_panic_hang_or_wake() {
    let folder = tempfile::tempdir().unwrap();
    let at = |name: &str| folder.path().join(name);
    let green = RgbaImage::from_pixel(64, 48, Rgba([10, 200, 30, 255]));
    green.save(at("a.png")).unwrap();
    let frames = [[255, 0, 0, 255], [0, 0, 255, 255]]
        .map(|color| Frame::new(RgbaImage::from_pixel(30, 20, Rgba(color))));
    let gif = std::fs::File::create(at("b.gif")).unwrap();
    GifEncoder::new(gif).encode_frames(frames).unwrap();
    let png = std::fs::read(at("a.png")).unwrap();
    std::fs::write(at("c.png"), &png[..png.len() / 2]).unwrap();
    let mut bomb = std::fs::read(at("b.gif")).unwrap();
    bomb[6..10].copy_from_slice(&[255; 4]);
    std::fs::write(at("d.gif"), bomb).unwrap();
    let fifo = std::process::Command::new("mkfifo")
        .arg(at("e.jpg"))
        .status();
    assert!(fifo.unwrap().success());

    let mut view = FileView::new(Appearance::default(), vec![at("a.png")]);
    until(&mut view, "a.png, 64 × 48");
    assert_eq!(view.deadline(), None, "a still picture never wakes");
    view.go(true);
    until(&mut view, "b.gif, 30 × 20");
    let first = view.deadline().expect("the next frame");
    assert!(view.tick(first + Duration::from_millis(1)));
    view.toggle_playing();
    assert_eq!(view.deadline(), None, "a paused animation never wakes");
    view.go(true);
    until(&mut view, "“c.png” could not be opened.");
    view.go(true);
    until(&mut view, "too large to open (65535 × 65535 pixels)");
    view.go(true);
    until(&mut view, "“e.jpg” could not be opened.");
    view.go(true);
    until(&mut view, "a.png, 64 × 48");
}

fn use_viewer_as_worker() {
    // SAFETY: the tests of this binary read the environment only through std::env, which
    // serialises with set_var, and all set the same value.
    unsafe { std::env::set_var("BANDE_PREVIEW_WORKER", env!("CARGO_BIN_EXE_bande-viewer")) };
}

#[test]
fn heif_avif_and_svg_text_decode_in_the_confined_picture_worker() {
    use_viewer_as_worker();
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let mut view = FileView::new(Appearance::default(), vec![data.join("camera.heic")]);
    until(&mut view, "camera.heic, 64 × 64");
    view.go(true);
    until(&mut view, "small.avif, 32 × 24");

    let folder = tempfile::tempdir().unwrap();
    let svg = folder.path().join("text.svg");
    std::fs::write(
        &svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="60"><text x="10" y="40" font-size="32">Bande</text></svg>"#,
    )
    .unwrap();
    let decoded = bande_content::decode_confined(&svg, 512).unwrap();
    let bande_content::Pixels::Still(levels) = decoded.pixels else {
        panic!("an SVG is a still picture");
    };
    assert!(
        levels[0]
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] > 0),
        "the text is drawn"
    );
}

#[test]
fn a_pdf_renders_in_its_own_process_finds_text_and_lists_its_outline() {
    use_viewer_as_worker();
    let folder = tempfile::tempdir().unwrap();
    write_pdf(&folder.path().join("doc.pdf"));
    std::fs::write(folder.path().join("bad.pdf"), "%PDF-1.7\n1 0 obj <<").unwrap();

    let mut view = FileView::new(Appearance::default(), vec![folder.path().join("doc.pdf")]);
    let shown = until(&mut view, "doc.pdf, 1 / 2");
    assert!(shown.contains("Chapter Two"), "{shown}");
    view.search("bande");
    until(&mut view, "doc.pdf, 2 / 2");
    assert_eq!(view.matches(), (Some(0), 1));
    let mut damaged = FileView::new(Appearance::default(), vec![folder.path().join("bad.pdf")]);
    until(&mut damaged, "“bad.pdf” could not be opened.");
}

#[test]
fn saving_a_rotation_changes_only_the_jpeg_orientation() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("a.jpg");
    image::RgbImage::from_pixel(64, 48, image::Rgb([200, 120, 40]))
        .save(&path)
        .unwrap();
    let scan = |bytes: &[u8]| {
        let start = bytes
            .windows(2)
            .position(|pair| pair == [0xFF, 0xDA])
            .unwrap();
        bytes[start..].to_vec()
    };
    let picture = scan(&std::fs::read(&path).unwrap());

    let mut view = FileView::new(Appearance::default(), vec![path.clone()]);
    until(&mut view, "a.jpg, 64 × 48");
    view.rotate(true);
    view.save_rotation();
    until(&mut view, "a.jpg, 48 × 64");
    view.rotate(true);
    view.save_rotation();
    until(&mut view, "a.jpg, 64 × 48");
    assert_eq!(scan(&std::fs::read(&path).unwrap()), picture);
    assert_eq!(view.rotation_saving(), None);
}
