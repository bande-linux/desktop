use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use bande_config::read_system_input_sources;
use bande_greeter::L10N;
use bande_toolkit::{
    Color, CpuRenderer, Item, Point, Scene, TextAlign, TextRole, TextStyle, TextSystem,
};
use xkbcommon::xkb;

const KEYS: u32 = 256;
const EVDEV_OFFSET: u32 = 8;
const DEAD_KEY: u32 = 0x8000_0000;
const LEVELS: u32 = 4;
const SHIFT: u32 = 1;
const ALT_GRAPHICS: u32 = 2;
const WIDEST: f32 = 560.0;
const MAGIC: &[u8; 4] = b"BUA1";
const LANGUAGE_FALLBACK: &str = "C.UTF-8";
const MESSAGES: [&str; 4] = [
    "unlock-disk",
    "unlock-wrong",
    "unlock-recovery",
    "unlock-hints",
];

fn word(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn keysym_entry(keysym: xkb::Keysym) -> u32 {
    let name = xkb::keysym_get_name(keysym);
    if name.starts_with("dead_") {
        return DEAD_KEY | keysym.raw();
    }
    xkb::keysym_to_utf32(keysym)
}

fn layout_table(keymap: &xkb::Keymap) -> (Vec<u32>, Vec<xkb::Keysym>) {
    let shift = keymap.mod_get_index(xkb::MOD_NAME_SHIFT);
    let alt_graphics = keymap.mod_get_index("Mod5");
    let mut table = Vec::new();
    let mut symbols = Vec::new();
    for level in 0..LEVELS {
        let mut mask = 0;
        if level & SHIFT != 0 {
            mask |= 1 << shift;
        }
        if level & ALT_GRAPHICS != 0 {
            mask |= 1 << alt_graphics;
        }
        let mut state = xkb::State::new(keymap);
        state.update_mask(mask, 0, 0, 0, 0, 0);
        let mut row = Vec::new();
        for key in 0..KEYS {
            let keysym = state.key_get_one_sym((key + EVDEV_OFFSET).into());
            symbols.push(keysym);
            row.push(keysym_entry(keysym));
        }
        table.push(row);
    }
    let mut flat = Vec::with_capacity((KEYS * LEVELS) as usize);
    for key in 0..KEYS as usize {
        for row in &table {
            flat.push(row[key]);
        }
    }
    (flat, symbols)
}

fn compose_pairs(context: &xkb::Context, symbols: &[xkb::Keysym]) -> Vec<(u32, u32, u32)> {
    let locale = env::var("LANG").unwrap_or_else(|_| LANGUAGE_FALLBACK.to_owned());
    let Ok(table) = xkb::compose::Table::new_from_locale(
        context,
        locale.as_ref(),
        xkb::compose::COMPILE_NO_FLAGS,
    ) else {
        return Vec::new();
    };
    let mut pairs = Vec::new();
    let dead: Vec<xkb::Keysym> = symbols
        .iter()
        .copied()
        .filter(|keysym| keysym_entry(*keysym) & DEAD_KEY != 0)
        .collect();
    for accent in &dead {
        for base in symbols {
            let plain = keysym_entry(*base);
            if plain == 0 || plain & DEAD_KEY != 0 {
                continue;
            }
            let mut state = xkb::compose::State::new(&table, xkb::compose::STATE_NO_FLAGS);
            state.feed(*accent);
            state.feed(*base);
            if state.status() == xkb::compose::Status::Composed
                && let Some(result) = state.utf8().and_then(|text| text.chars().next())
            {
                let entry = (keysym_entry(*accent), plain, result as u32);
                if !pairs.contains(&entry) {
                    pairs.push(entry);
                }
            }
        }
    }
    pairs
}

fn bitmap(
    text: &mut TextSystem,
    renderer: &mut CpuRenderer,
    message: &str,
    scale: f64,
) -> Option<(u32, u32, Vec<u8>)> {
    let style = TextStyle::role(TextRole::Callout, Color::WHITE);
    let paragraph = text.paragraph(message, style, Some(WIDEST), TextAlign::Start);
    let width = paragraph.width().ceil().max(1.0) as u32;
    let height = paragraph.height().ceil().max(1.0) as u32;
    let scene = Scene {
        width,
        height,
        background: Color::TRANSPARENT,
        items: vec![Item::Text {
            origin: Point::new(0.0, 0.0),
            paragraph,
        }],
    };
    let wide = bande_toolkit::pixels(width, scale);
    let tall = bande_toolkit::pixels(height, scale);
    let mut rgba = vec![0; wide as usize * tall as usize * 4];
    renderer.render_into(&scene, scale, &mut rgba).ok()?;
    let alpha = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| pixel[3])
        .collect();
    Some((wide, tall, alpha))
}

fn write_assets(path: &Path) -> io::Result<()> {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let mut output = MAGIC.to_vec();
    let layouts = read_system_input_sources();
    let usable: Vec<_> = layouts
        .iter()
        .filter_map(|source| {
            let keymap = xkb::Keymap::new_from_names(
                &context,
                "",
                "",
                &source.layout,
                source.variant.as_deref().unwrap_or(""),
                None,
                xkb::KEYMAP_COMPILE_NO_FLAGS,
            )?;
            Some((source.layout.to_uppercase(), keymap))
        })
        .collect();
    if usable.is_empty() {
        return Err(io::Error::other("no usable keyboard layout"));
    }
    word(&mut output, usable.len() as u32);
    let mut every_symbol = Vec::new();
    for (name, keymap) in &usable {
        word(&mut output, name.len() as u32);
        output.extend_from_slice(name.as_bytes());
        let (flat, symbols) = layout_table(keymap);
        for entry in flat {
            word(&mut output, entry);
        }
        every_symbol.extend(symbols);
    }
    let pairs = compose_pairs(&context, &every_symbol);
    word(&mut output, pairs.len() as u32);
    for (accent, base, result) in pairs {
        word(&mut output, accent);
        word(&mut output, base);
        word(&mut output, result);
    }
    let mut text = TextSystem::default();
    let mut renderer = CpuRenderer::default();
    for scale in [1.0, 2.0] {
        for message in MESSAGES {
            let (width, height, alpha) = bitmap(&mut text, &mut renderer, &L10N.tr(message), scale)
                .ok_or_else(|| io::Error::other("cannot draw the unlock text"))?;
            word(&mut output, width);
            word(&mut output, height);
            output.extend_from_slice(&alpha);
        }
    }
    let partial = path.with_extension("partial");
    let mut file = fs::File::create(&partial)?;
    file.write_all(&output)?;
    file.sync_all()?;
    fs::rename(partial, path)
}

fn main() -> ExitCode {
    let Some(output) = env::args().nth(1) else {
        eprintln!("usage: bande-unlock-assets <output file>");
        return ExitCode::from(2);
    };
    match write_assets(Path::new(&output)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("bande-unlock-assets: {error}");
            ExitCode::FAILURE
        }
    }
}
