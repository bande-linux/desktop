use std::collections::HashMap;
use std::sync::Arc;

use fontique::{
    Attributes, Collection, CollectionOptions, FallbackKey, FontStyle, FontWeight, FontWidth,
    GenericFamily, QueryFamily, QueryFont, QueryStatus, Script, SourceCache,
};
use harfrust::{BufferClusterLevel, Direction, ShapeOptions, UnicodeBuffer};
use icu_properties::props::Script as UnicodeScript;
use icu_properties::{CodePointMapData, PropertyNamesShort};
use linebender_resource_handle::FontData;
use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use web_style::FontFamily;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FontId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FontKey {
    pub family: FontFamily,
    pub names: u16,
    pub weight: u16,
    pub italic: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FontMetrics {
    pub units_per_em: f32,
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
    pub x_height: f32,
    pub underline_offset: f32,
    pub underline_thickness: f32,
    pub strikeout_offset: f32,
    pub strikeout_thickness: f32,
}

pub(crate) struct LoadedFont {
    pub(crate) data: FontData,
    pub(crate) shaper: harfrust::ShaperData,
    pub(crate) metrics: FontMetrics,
}

pub struct Fonts {
    collection: Collection,
    source_cache: SourceCache,
    loaded: Vec<Option<LoadedFont>>,
    by_source: HashMap<(u64, u32), FontId>,
    primary: HashMap<FontKey, Option<FontId>>,
    chosen: HashMap<(FontKey, char), Option<FontId>>,
    names: Vec<Arc<[Arc<str>]>>,
    plans: HashMap<(FontId, harfrust::Script, bool), harfrust::ShapePlan>,
    words: HashMap<(FontId, u32, bool), WordCache>,
    cached_words: usize,
    buffer: Option<UnicodeBuffer>,
    scope: u32,
    scopes: u32,
    faces: Vec<WebFace>,
}

struct WebFace {
    scope: u32,
    family: Box<str>,
    alias: Arc<str>,
    blob: u64,
    bytes: usize,
    ranges: Box<[(u32, u32)]>,
    registered: Vec<(fontique::FamilyId, FontWidth, FontStyle, FontWeight)>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WordGlyph {
    pub(crate) id: u32,
    pub(crate) cluster: u32,
    pub(crate) advance: f32,
    pub(crate) x_offset: f32,
    pub(crate) y_offset: f32,
}

const WORD_CACHE_LIMIT: usize = 65536;

type WordCache = HashMap<Box<str>, Box<[WordGlyph]>>;

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

impl Fonts {
    pub fn new() -> Self {
        Self {
            collection: Collection::new(CollectionOptions {
                shared: false,
                system_fonts: true,
            }),
            source_cache: SourceCache::default(),
            loaded: Vec::new(),
            by_source: HashMap::new(),
            primary: HashMap::new(),
            chosen: HashMap::new(),
            names: Vec::new(),
            plans: HashMap::new(),
            words: HashMap::new(),
            cached_words: 0,
            buffer: None,
            scope: 0,
            scopes: 0,
            faces: Vec::new(),
        }
    }

    pub fn new_scope(&mut self) -> u32 {
        self.scopes = self.scopes.wrapping_add(1).max(1);
        self.scopes
    }

    pub fn set_scope(&mut self, scope: u32) {
        self.scope = scope;
    }

    pub fn add_face(
        &mut self,
        scope: u32,
        family: &str,
        bytes: &[u8],
        weight: Option<(u16, u16)>,
        italic: Option<bool>,
        ranges: &[(u32, u32)],
    ) -> bool {
        let Some(sfnt) = crate::woff::decode(bytes) else {
            return false;
        };
        if skrifa::FontRef::from_index(&sfnt, 0).is_err() {
            return false;
        }
        let family = family.to_lowercase();
        let alias: Arc<str> =
            match self.faces.iter().find(|face| {
                face.scope == scope && *face.family == *family && *face.ranges == *ranges
            }) {
                Some(face) => face.alias.clone(),
                None => format!("bande-web-{scope}-{}-{family}", self.faces.len()).into(),
            };
        let size = sfnt.len();
        let blob = fontique::Blob::new(Arc::new(sfnt));
        let id = blob.id();
        let registered = self.collection.register_fonts(
            blob,
            Some(fontique::FontInfoOverride {
                family_name: Some(&alias),
                weight: weight
                    .filter(|(start, end)| start == end)
                    .map(|(start, _)| FontWeight::new(f32::from(start))),
                style: italic.map(|italic| {
                    if italic {
                        FontStyle::Italic
                    } else {
                        FontStyle::Normal
                    }
                }),
                ..Default::default()
            }),
        );
        if registered.is_empty() {
            return false;
        }
        self.faces.push(WebFace {
            scope,
            family: family.into(),
            alias,
            blob: id,
            bytes: size,
            ranges: ranges.into(),
            registered: registered
                .iter()
                .flat_map(|(family, fonts)| {
                    fonts
                        .iter()
                        .map(|font| (*family, font.width(), font.style(), font.weight()))
                })
                .collect(),
        });
        self.primary.clear();
        self.chosen.clear();
        true
    }

    pub fn has_face(&self, scope: u32, family: &str) -> bool {
        self.faces
            .iter()
            .any(|face| face.scope == scope && face.family.eq_ignore_ascii_case(family))
    }

    pub fn face_bytes(&self, scope: u32) -> usize {
        self.faces
            .iter()
            .filter(|face| face.scope == scope)
            .map(|face| face.bytes)
            .sum()
    }

    pub fn forget_scope(&mut self, scope: u32) {
        let (gone, kept): (Vec<WebFace>, Vec<WebFace>) = std::mem::take(&mut self.faces)
            .into_iter()
            .partition(|face| face.scope == scope);
        self.faces = kept;
        if gone.is_empty() {
            return;
        }
        for face in &gone {
            for &(family, width, style, weight) in &face.registered {
                self.collection
                    .unregister_font(family, width, style, weight);
            }
        }
        let blobs: Vec<u64> = gone.iter().map(|face| face.blob).collect();
        let fonts: Vec<FontId> = self
            .by_source
            .iter()
            .filter(|((blob, _), _)| blobs.contains(blob))
            .map(|(_, &font)| font)
            .collect();
        self.by_source.retain(|(blob, _), _| !blobs.contains(blob));
        for font in &fonts {
            if let Some(slot) = self.loaded.get_mut(font.0 as usize) {
                *slot = None;
            }
        }
        self.words.retain(|(font, _, _), _| !fonts.contains(font));
        self.plans.retain(|(font, _, _), _| !fonts.contains(font));
        self.primary.clear();
        self.chosen.clear();
    }

    pub fn data(&self, font: FontId) -> Option<&FontData> {
        self.loaded(font).map(|loaded| &loaded.data)
    }

    pub fn metrics(&self, font: FontId) -> FontMetrics {
        self.loaded(font)
            .map(|loaded| loaded.metrics)
            .unwrap_or_default()
    }

    fn loaded(&self, font: FontId) -> Option<&LoadedFont> {
        self.loaded.get(font.0 as usize)?.as_ref()
    }

    pub(crate) fn shape_word(
        &mut self,
        font: FontId,
        size: f32,
        rtl: bool,
        word: &str,
    ) -> Option<&[WordGlyph]> {
        let key = (font, size.to_bits(), rtl);
        let cached = self
            .words
            .get(&key)
            .is_some_and(|words| words.contains_key(word));
        if !cached {
            let glyphs = self.shape_uncached(font, size, rtl, word)?;
            if self.cached_words >= WORD_CACHE_LIMIT {
                self.words.clear();
                self.cached_words = 0;
            }
            self.cached_words += 1;
            self.words
                .entry(key)
                .or_default()
                .insert(word.into(), glyphs);
        }
        self.words.get(&key)?.get(word).map(|glyphs| &glyphs[..])
    }

    fn shape_uncached(
        &mut self,
        font: FontId,
        size: f32,
        rtl: bool,
        word: &str,
    ) -> Option<Box<[WordGlyph]>> {
        let loaded = self.loaded.get(font.0 as usize)?.as_ref()?;
        let font_ref =
            harfrust::FontRef::from_index(loaded.data.data.data(), loaded.data.index).ok()?;
        let shaper = loaded.shaper.shaper(&font_ref).build();
        let mut buffer = self.buffer.take().unwrap_or_default();
        buffer.push_str(word);
        buffer.guess_segment_properties();
        buffer.set_direction(if rtl {
            Direction::RightToLeft
        } else {
            Direction::LeftToRight
        });
        buffer.set_cluster_level(BufferClusterLevel::MonotoneCharacters);
        let script = buffer.script();
        let plan = self.plans.entry((font, script, rtl)).or_insert_with(|| {
            harfrust::ShapePlan::new(&shaper, buffer.direction(), Some(script), None, &[])
        });
        let output = shaper.shape(
            buffer,
            ShapeOptions::new().plan(Some(plan)).point_size(Some(size)),
        );
        let scale = size / loaded.metrics.units_per_em;
        let mut glyphs: Box<[WordGlyph]> = output
            .glyph_infos()
            .iter()
            .zip(output.glyph_positions())
            .map(|(info, position)| WordGlyph {
                id: info.glyph_id,
                cluster: info.cluster,
                advance: position.x_advance as f32 * scale,
                x_offset: position.x_offset as f32 * scale,
                y_offset: position.y_offset as f32 * scale,
            })
            .collect();
        if rtl {
            glyphs.reverse();
        }
        self.buffer = Some(output.clear());
        Some(glyphs)
    }

    pub fn family_names(&mut self, names: &Arc<[Arc<str>]>) -> u16 {
        if names.is_empty() {
            return 0;
        }
        let scoped = self.scope != 0
            && self.faces.iter().any(|face| {
                face.scope == self.scope
                    && names.iter().any(|name| name.to_lowercase() == *face.family)
            });
        let mapped: Arc<[Arc<str>]>;
        let names = if scoped {
            let mut expanded: Vec<Arc<str>> = Vec::new();
            for name in names.iter() {
                let lowered = name.to_lowercase();
                let before = expanded.len();
                for face in self.faces.iter().rev() {
                    if face.scope == self.scope
                        && *face.family == *lowered
                        && !expanded.contains(&face.alias)
                    {
                        expanded.push(face.alias.clone());
                    }
                }
                if expanded.len() == before {
                    expanded.push(name.clone());
                }
            }
            mapped = expanded.into();
            &mapped
        } else {
            names
        };
        match self.names.iter().position(|known| known == names) {
            Some(index) => (index + 1) as u16,
            None if self.names.len() < usize::from(u16::MAX) - 1 => {
                self.names.push(names.clone());
                self.names.len() as u16
            }
            None => 0,
        }
    }

    pub fn register(&mut self, bytes: Vec<u8>) {
        self.collection
            .register_fonts(fontique::Blob::new(Arc::new(bytes)), None);
        self.primary.clear();
        self.chosen.clear();
    }

    pub fn primary(&mut self, key: FontKey) -> Option<FontId> {
        if let Some(&font) = self.primary.get(&key) {
            return font;
        }
        let names: Vec<Arc<str>> = usize::from(key.names)
            .checked_sub(1)
            .and_then(|index| self.names.get(index))
            .map(|names| names.to_vec())
            .unwrap_or_default();
        let mut query = self.collection.query(&mut self.source_cache);
        query.set_families(
            names
                .iter()
                .map(|name| QueryFamily::Named(name))
                .chain([QueryFamily::Generic(generic(key.family))]),
        );
        query.set_attributes(attributes(key));
        let faces = &self.faces;
        let mut found = None;
        query.matches_with(|font| {
            if allows(faces, font.blob.id(), ' ') {
                found = Some(font.clone());
                QueryStatus::Stop
            } else {
                QueryStatus::Continue
            }
        });
        drop(query);
        let font = found.and_then(|found| self.load(&found));
        self.primary.insert(key, font);
        font
    }

    pub(crate) fn for_char(&mut self, key: FontKey, ch: char) -> Option<FontId> {
        if let Some(&font) = self.chosen.get(&(key, ch)) {
            return font;
        }
        let primary = self.primary(key);
        if let Some(primary) = primary
            && self.maps(primary, ch)
            && self
                .loaded(primary)
                .is_none_or(|loaded| allows(&self.faces, loaded.data.data.id(), ch))
        {
            self.chosen.insert((key, ch), Some(primary));
            return Some(primary);
        }
        let script = script_of(ch);
        let mut found = None;
        let names: Vec<Arc<str>> = usize::from(key.names)
            .checked_sub(1)
            .and_then(|index| self.names.get(index))
            .map(|names| names.to_vec())
            .unwrap_or_default();
        let mut query = self.collection.query(&mut self.source_cache);
        query.set_families(names.iter().map(|name| QueryFamily::Named(name)).chain([
            QueryFamily::Generic(generic(key.family)),
            QueryFamily::Generic(GenericFamily::Emoji),
        ]));
        query.set_attributes(attributes(key));
        if let Some(script) = script {
            query.set_fallbacks(FallbackKey::new(script, None));
        }
        let faces = &self.faces;
        query.matches_with(|font| {
            if allows(faces, font.blob.id(), ch)
                && font
                    .charmap()
                    .is_some_and(|charmap| charmap.map(ch).is_some())
            {
                found = Some(font.clone());
                QueryStatus::Stop
            } else {
                QueryStatus::Continue
            }
        });
        drop(query);
        let font = found.and_then(|found| self.load(&found)).or(primary);
        self.chosen.insert((key, ch), font);
        font
    }

    fn maps(&self, font: FontId, ch: char) -> bool {
        self.loaded(font).is_some_and(|loaded| {
            skrifa::FontRef::from_index(loaded.data.data.data(), loaded.data.index)
                .is_ok_and(|font| font.charmap().map(ch).is_some())
        })
    }

    fn load(&mut self, found: &QueryFont) -> Option<FontId> {
        let source = (found.blob.id(), found.index);
        if let Some(&font) = self.by_source.get(&source) {
            return Some(font);
        }
        let font = skrifa::FontRef::from_index(found.blob.data(), found.index).ok()?;
        let metrics = font.metrics(Size::unscaled(), LocationRef::default());
        let shaper = harfrust::ShaperData::new(&font);
        let units_per_em = f32::from(metrics.units_per_em.max(1));
        let underline = metrics.underline.unwrap_or(skrifa::metrics::Decoration {
            offset: -units_per_em / 10.0,
            thickness: units_per_em / 16.0,
        });
        let strikeout = metrics.strikeout.unwrap_or(skrifa::metrics::Decoration {
            offset: metrics.ascent / 3.0,
            thickness: underline.thickness,
        });
        let id = FontId(u32::try_from(self.loaded.len()).ok()?);
        self.loaded.push(Some(LoadedFont {
            data: FontData::new(found.blob.clone(), found.index),
            shaper,
            metrics: FontMetrics {
                units_per_em,
                ascent: metrics.ascent,
                descent: -metrics.descent,
                line_gap: metrics.leading,
                x_height: metrics.x_height.unwrap_or(units_per_em / 2.0),
                underline_offset: underline.offset,
                underline_thickness: underline.thickness,
                strikeout_offset: strikeout.offset,
                strikeout_thickness: strikeout.thickness,
            },
        }));
        self.by_source.insert(source, id);
        Some(id)
    }
}

fn allows(faces: &[WebFace], blob: u64, ch: char) -> bool {
    let code = ch as u32;
    faces
        .iter()
        .find(|face| face.blob == blob)
        .is_none_or(|face| {
            face.ranges.is_empty()
                || face
                    .ranges
                    .iter()
                    .any(|&(start, end)| (start..=end).contains(&code))
        })
}

fn generic(family: FontFamily) -> GenericFamily {
    match family {
        FontFamily::Serif => GenericFamily::Serif,
        FontFamily::SansSerif => GenericFamily::SansSerif,
        FontFamily::Monospace => GenericFamily::Monospace,
    }
}

fn attributes(key: FontKey) -> Attributes {
    Attributes::new(
        FontWidth::NORMAL,
        if key.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
        FontWeight::new(f32::from(key.weight)),
    )
}

pub(crate) fn script_of(ch: char) -> Option<Script> {
    let script = CodePointMapData::<UnicodeScript>::new().get(ch);
    if matches!(
        script,
        UnicodeScript::Common | UnicodeScript::Inherited | UnicodeScript::Unknown
    ) {
        return None;
    }
    let tag = PropertyNamesShort::<UnicodeScript>::new().get_locale_script(script)?;
    let bytes: [u8; 4] = tag.as_str().as_bytes().try_into().ok()?;
    Some(Script::from_bytes(bytes))
}

pub(crate) fn is_inherited(ch: char) -> bool {
    CodePointMapData::<UnicodeScript>::new().get(ch) == UnicodeScript::Inherited
}
