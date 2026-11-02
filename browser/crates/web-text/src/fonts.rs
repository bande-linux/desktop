use std::collections::HashMap;

use fontique::{
    Attributes, Collection, CollectionOptions, FallbackKey, FontStyle, FontWeight, FontWidth,
    GenericFamily, QueryFamily, QueryFont, QueryStatus, Script, SourceCache,
};
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
    pub weight: u16,
    pub italic: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FontMetrics {
    pub units_per_em: f32,
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
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
    loaded: Vec<LoadedFont>,
    by_source: HashMap<(u64, u32), FontId>,
    primary: HashMap<FontKey, Option<FontId>>,
    fallback: HashMap<(FontKey, char), Option<FontId>>,
}

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
            fallback: HashMap::new(),
        }
    }

    pub fn data(&self, font: FontId) -> Option<&FontData> {
        self.loaded.get(font.0 as usize).map(|loaded| &loaded.data)
    }

    pub fn metrics(&self, font: FontId) -> FontMetrics {
        self.loaded
            .get(font.0 as usize)
            .map(|loaded| loaded.metrics)
            .unwrap_or_default()
    }

    pub(crate) fn loaded(&self, font: FontId) -> Option<&LoadedFont> {
        self.loaded.get(font.0 as usize)
    }

    pub fn primary(&mut self, key: FontKey) -> Option<FontId> {
        if let Some(&font) = self.primary.get(&key) {
            return font;
        }
        let mut query = self.collection.query(&mut self.source_cache);
        query.set_families([QueryFamily::Generic(generic(key.family))]);
        query.set_attributes(attributes(key));
        let found = query.first_available_font();
        drop(query);
        let font = found.and_then(|found| self.load(&found));
        self.primary.insert(key, font);
        font
    }

    pub(crate) fn for_char(&mut self, key: FontKey, ch: char) -> Option<FontId> {
        let primary = self.primary(key);
        if let Some(primary) = primary
            && self.maps(primary, ch)
        {
            return Some(primary);
        }
        if let Some(&font) = self.fallback.get(&(key, ch)) {
            return font;
        }
        let script = script_of(ch);
        let mut found = None;
        let mut query = self.collection.query(&mut self.source_cache);
        query.set_families([
            QueryFamily::Generic(generic(key.family)),
            QueryFamily::Generic(GenericFamily::Emoji),
        ]);
        query.set_attributes(attributes(key));
        if let Some(script) = script {
            query.set_fallbacks(FallbackKey::new(script, None));
        }
        query.matches_with(|font| {
            if font
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
        self.fallback.insert((key, ch), font);
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
        self.loaded.push(LoadedFont {
            data: FontData::new(found.blob.clone(), found.index),
            shaper,
            metrics: FontMetrics {
                units_per_em,
                ascent: metrics.ascent,
                descent: -metrics.descent,
                line_gap: metrics.leading,
                underline_offset: underline.offset,
                underline_thickness: underline.thickness,
                strikeout_offset: strikeout.offset,
                strikeout_thickness: strikeout.thickness,
            },
        });
        self.by_source.insert(source, id);
        Some(id)
    }
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
