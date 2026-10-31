#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bande_graphics::{
    Color, Cut, GlyphRun, Paragraph, TextAlign, TextRole, TextStyle, TextSystem, shortened,
};
use skrifa::{FontRef, MetadataProvider};

const INTER: &str = "/usr/share/fonts/opentype/inter";
const MANROPE: &str = "/usr/share/fonts/truetype/manrope";

fn runs(text: &mut TextSystem, content: &str, style: TextStyle) -> Vec<GlyphRun> {
    text.paragraph(content, style, None, TextAlign::Start)
        .glyph_runs()
}

fn face(file: &str) -> Vec<u8> {
    let directory = if file.starts_with("Manrope") {
        MANROPE
    } else {
        INTER
    };
    std::fs::read(format!("{directory}/{file}")).expect("the Datum fonts are installed")
}

fn advance(paragraph: &Paragraph) -> f32 {
    paragraph.width()
}

#[test]
fn interface_text_is_set_in_inter_and_display_text_in_manrope() {
    let mut text = TextSystem::new();
    let ink = Color::hex(0x000000);
    for (style, file) in [
        (TextStyle::new(13.0, ink), "Inter-Regular.otf"),
        (
            TextStyle::new(13.0, ink).weight(TextStyle::SEMIBOLD),
            "Inter-SemiBold.otf",
        ),
        (TextStyle::new(19.5, ink), "Inter-Regular.otf"),
        (TextStyle::new(20.0, ink), "Manrope-Regular.ttf"),
        (
            TextStyle::role(TextRole::WindowTitle, ink),
            "Manrope-Bold.ttf",
        ),
        (TextStyle::role(TextRole::Heading, ink), "Manrope-Bold.ttf"),
        (TextStyle::role(TextRole::Label, ink), "Inter-SemiBold.otf"),
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
fn inter_uses_the_datum_alternates_for_l_and_capital_i() {
    let mut text = TextSystem::new();
    let inter = face("Inter-Regular.otf");
    let font = FontRef::new(&inter).expect("Inter parses");
    let charmap = font.charmap();
    let runs = runs(&mut text, "lI", TextStyle::new(13.0, Color::hex(0)));
    let shaped: Vec<u32> = runs[0].glyphs.iter().map(|glyph| glyph.id).collect();
    let plain: Vec<u32> = ['l', 'I']
        .into_iter()
        .map(|character| charmap.map(character).expect("mapped").to_u32())
        .collect();
    assert_eq!(shaped.len(), 2);
    assert_ne!(shaped[0], plain[0], "l carries its tail (cv05)");
    assert_ne!(shaped[1], plain[1], "I carries its serifs (cv08)");
}

#[test]
fn labels_are_set_in_tracked_capitals() {
    let mut text = TextSystem::new();
    let ink = Color::hex(0);
    let label = text.paragraph(
        "places",
        TextStyle::role(TextRole::Label, ink),
        None,
        TextAlign::Start,
    );
    let capitals = text.paragraph(
        "PLACES",
        TextStyle {
            tracking: 0.0,
            ..TextStyle::role(TextRole::Label, ink)
        },
        None,
        TextAlign::Start,
    );
    assert!(advance(&label) > advance(&capitals) + 2.0);
    assert_eq!(label.height(), 14.0);
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
fn tabular_digits_share_one_width_so_changing_numbers_do_not_jitter() {
    let mut text = TextSystem::new();
    let proportional = TextStyle::new(13.0, Color::hex(0));
    let mut width = |content: &str, style: TextStyle| {
        advance(&text.paragraph(content, style, None, TextAlign::Start))
    };
    let (ones, zeros) = (width("1111", proportional), width("0000", proportional));
    assert!(ones < zeros - 1.0, "1111 is {ones} px, 0000 is {zeros} px");
    let tabular = proportional.tabular();
    let (ones, zeros) = (width("1111", tabular), width("0000", tabular));
    assert!(
        (ones - zeros).abs() < 0.01,
        "1111 is {ones} px, 0000 is {zeros} px"
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

#[test]
fn a_name_cut_in_the_middle_keeps_its_extension_and_fits_the_width() {
    let mut text = TextSystem::new();
    let style = TextStyle::new(13.0, Color::hex(0x000000));
    let mut width_of = |content: &str| {
        text.paragraph(content, style, None, TextAlign::Start)
            .width()
    };
    let long = "quarterly-report-final-version-2-for-the-board.pdf";
    let shown = shortened(long, Cut::Middle, |candidate| width_of(candidate) <= 160.0);
    assert!(shown.ends_with("ard.pdf"), "{shown}");
    assert!(shown.starts_with("quarterly-"), "{shown}");
    assert!(shown.contains('\u{2026}'), "{shown}");
    assert!(width_of(&shown) <= 160.0, "{shown}");
    let ended = shortened(long, Cut::End, |candidate| width_of(candidate) <= 160.0);
    assert!(
        ended.ends_with('\u{2026}') && !ended.contains(".pdf"),
        "{ended}"
    );
    assert_eq!(
        shortened("notes.txt", Cut::Middle, |candidate| width_of(candidate)
            <= 160.0),
        "notes.txt"
    );
}
