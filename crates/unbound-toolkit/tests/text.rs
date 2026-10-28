use unbound_toolkit::{Color, GlyphRun, Paragraph, TextAlign, TextStyle, TextSystem};

const INTER: &str = "/usr/share/fonts/opentype/inter";

fn runs(text: &mut TextSystem, content: &str, style: TextStyle) -> Vec<GlyphRun> {
    text.paragraph(content, style, None, TextAlign::Start)
        .glyph_runs()
}

fn face(file: &str) -> Vec<u8> {
    std::fs::read(format!("{INTER}/{file}")).expect("fonts-inter is installed")
}

fn advance(paragraph: &Paragraph) -> f32 {
    paragraph.width()
}

#[test]
fn latin_text_is_set_in_inter_with_the_display_cut_from_20_px() {
    let mut text = TextSystem::new();
    let ink = Color::hex(0x000000);
    for (style, file) in [
        (TextStyle::new(13.0, ink), "Inter-Regular.otf"),
        (
            TextStyle::new(13.0, ink).weight(TextStyle::SEMIBOLD),
            "Inter-SemiBold.otf",
        ),
        (TextStyle::new(19.5, ink), "Inter-Regular.otf"),
        (TextStyle::new(20.0, ink), "InterDisplay-Regular.otf"),
        (
            TextStyle::new(28.0, ink).weight(TextStyle::SEMIBOLD),
            "InterDisplay-SemiBold.otf",
        ),
    ] {
        let runs = runs(&mut text, "Settings", style);
        assert_eq!(runs.len(), 1);
        assert!(
            runs[0].font_bytes() == face(file),
            "{} px at weight {} is not set in {file}",
            style.size,
            style.weight
        );
    }
}

#[test]
fn han_characters_fall_back_to_a_font_that_has_them() {
    let mut text = TextSystem::new();
    let runs = runs(&mut text, "Wi-Fi 设置", TextStyle::new(13.0, Color::hex(0)));
    let inter = face("Inter-Regular.otf");
    let han: Vec<&GlyphRun> = runs
        .iter()
        .filter(|run| run.font_bytes() != inter.as_slice())
        .collect();
    assert_eq!(han.len(), 1, "one fallback run for the Han characters");
    assert_eq!(han[0].glyphs.len(), 2);
    assert!(
        han[0].glyphs.iter().all(|glyph| glyph.id != 0),
        "the fallback font maps both characters"
    );
}

#[test]
fn glyph_pairs_are_kerned_by_the_font() {
    let mut text = TextSystem::new();
    let style = TextStyle::new(20.0, Color::hex(0));
    let mut width =
        |content: &str| advance(&text.paragraph(content, style, None, TextAlign::Start));
    let pair = width("AV");
    let apart = width("A") + width("V");
    assert!(pair < apart - 0.5, "AV is {pair} px, A + V is {apart} px");
}

#[test]
fn glyphs_keep_fractional_pen_positions() {
    let mut text = TextSystem::new();
    let runs = runs(
        &mut text,
        "Displays, Sound & Network",
        TextStyle::new(13.0, Color::hex(0)),
    );
    let positions: Vec<f32> = runs
        .iter()
        .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.x))
        .collect();
    let fractional = positions.iter().filter(|x| x.fract() != 0.0).count();
    assert!(
        fractional * 2 > positions.len(),
        "most pen positions are fractional, as with subpixel positioning: {positions:?}"
    );
}

#[test]
fn baselines_sit_on_whole_pixels() {
    let mut text = TextSystem::new();
    let paragraph = text.paragraph(
        "A paragraph long enough to wrap onto several lines of text",
        TextStyle::new(13.0, Color::hex(0)),
        Some(120.0),
        TextAlign::Start,
    );
    for run in paragraph.glyph_runs() {
        for glyph in &run.glyphs {
            assert_eq!(glyph.y.fract(), 0.0, "baseline at {}", glyph.y);
        }
    }
}

#[test]
fn paragraphs_wrap_at_word_boundaries_within_the_width() {
    let mut text = TextSystem::new();
    let paragraph = text.paragraph(
        "A paragraph long enough to wrap onto several lines of text",
        TextStyle::new(13.0, Color::hex(0)),
        Some(120.0),
        TextAlign::Start,
    );
    assert!(
        paragraph.line_count() >= 3,
        "{} lines",
        paragraph.line_count()
    );
    assert!(paragraph.width() <= 120.0, "{} px wide", paragraph.width());
}

#[test]
fn centred_text_has_equal_space_on_both_sides() {
    let mut text = TextSystem::new();
    let style = TextStyle::new(13.0, Color::hex(0));
    let natural = text
        .paragraph("Apply", style, None, TextAlign::Start)
        .width();
    let centred = text.paragraph("Apply", style, Some(96.0), TextAlign::Center);
    let first = centred.glyph_runs()[0].glyphs[0].x;
    assert!(
        (first - (96.0 - natural) / 2.0).abs() < 0.01,
        "first glyph at {first}, natural width {natural}"
    );
}
