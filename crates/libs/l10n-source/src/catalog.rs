use std::cmp::Ordering;

use crate::plural::PluralRule;
use crate::{DEVELOPMENT_HEADER, key};

const MAGIC: u32 = 0x9504_12de;
const HEADER_SIZE: usize = 28;

pub struct Catalog {
    data: Box<[u8]>,
    swapped: bool,
    count: usize,
    originals: usize,
    translations: usize,
    plural: PluralRule,
    development: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub context: Option<String>,
    pub english: String,
    pub plural: Option<String>,
    pub forms: Vec<String>,
}

impl Catalog {
    pub fn parse(data: Vec<u8>) -> Result<Self, String> {
        let data = data.into_boxed_slice();
        let magic = data
            .get(..4)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
            .ok_or("too short for a catalog")?;
        let swapped = match magic {
            MAGIC => false,
            other if other.swap_bytes() == MAGIC => true,
            _ => return Err("not a gettext catalog".to_owned()),
        };
        let mut catalog = Self {
            data,
            swapped,
            count: 0,
            originals: 0,
            translations: 0,
            plural: PluralRule::default(),
            development: false,
        };
        let word = |index: usize| catalog.word(4 * index).ok_or("a damaged catalog header");
        let (count, originals, translations) = (word(2)?, word(3)?, word(4)?);
        let tables_end = count
            .checked_mul(8)
            .and_then(|size| originals.max(translations).checked_add(size));
        if tables_end.is_none_or(|end| end > catalog.data.len()) {
            return Err("a catalog shorter than its tables".to_owned());
        }
        catalog.count = count;
        catalog.originals = originals;
        catalog.translations = translations;
        if let Some(rule) = catalog.header("Plural-Forms") {
            catalog.plural = PluralRule::parse(&rule)?;
        }
        catalog.development = catalog.header(DEVELOPMENT_HEADER).as_deref() == Some("yes");
        Ok(catalog)
    }

    pub fn header(&self, name: &str) -> Option<String> {
        let header = self.find(None, "")?.first()?.to_string();
        header.lines().find_map(|line| {
            let (field, value) = line.split_once(':')?;
            (field.trim() == name).then(|| value.trim().to_owned())
        })
    }

    pub fn is_development(&self) -> bool {
        self.development
    }

    pub fn plural(&self) -> &PluralRule {
        &self.plural
    }

    pub fn find(&self, context: Option<&str>, english: &str) -> Option<Vec<&str>> {
        let wanted = key(context, english);
        let wanted = wanted.as_bytes();
        let (mut low, mut high) = (0, self.count);
        while low < high {
            let middle = low + (high - low) / 2;
            let original = self.string(self.originals, middle)?;
            let compared = original.split(|byte| *byte == 0).next().unwrap_or(original);
            match compared.cmp(wanted) {
                Ordering::Less => low = middle + 1,
                Ordering::Greater => high = middle,
                Ordering::Equal => {
                    let translation =
                        std::str::from_utf8(self.string(self.translations, middle)?).ok()?;
                    let forms = translation.split('\0').collect::<Vec<_>>();
                    return forms.iter().all(|form| !form.is_empty()).then_some(forms);
                }
            }
        }
        None
    }

    fn word(&self, offset: usize) -> Option<usize> {
        let bytes: [u8; 4] = self.data.get(offset..offset + 4)?.try_into().ok()?;
        let word = u32::from_le_bytes(bytes);
        usize::try_from(if self.swapped {
            word.swap_bytes()
        } else {
            word
        })
        .ok()
    }

    fn string(&self, table: usize, index: usize) -> Option<&[u8]> {
        let length = self.word(table + 8 * index)?;
        let offset = self.word(table + 8 * index + 4)?;
        self.data.get(offset..offset.checked_add(length)?)
    }
}

pub fn write(header: &str, entries: &[Entry]) -> Vec<u8> {
    let mut strings = entries
        .iter()
        .map(|entry| {
            let mut original = key(entry.context.as_deref(), &entry.english);
            if let Some(plural) = &entry.plural {
                original.push('\0');
                original.push_str(plural);
            }
            (original, entry.forms.join("\0"))
        })
        .chain([(String::new(), header.to_owned())])
        .collect::<Vec<_>>();
    strings.sort_by(|(left, _), (right, _)| left.as_bytes().cmp(right.as_bytes()));
    strings.dedup_by(|(left, _), (right, _)| left == right);
    let count = strings.len();
    let originals = HEADER_SIZE;
    let translations = originals + 8 * count;
    let mut data = Vec::new();
    let mut tables = Vec::new();
    let mut text = Vec::new();
    let text_start = translations + 8 * count;
    let mut place = |string: &str, tables: &mut Vec<u8>| {
        let offset = text_start + text.len();
        tables.extend(word(string.len()));
        tables.extend(word(offset));
        text.extend_from_slice(string.as_bytes());
        text.push(0);
    };
    for (original, _) in &strings {
        place(original, &mut tables);
    }
    for (_, translation) in &strings {
        place(translation, &mut tables);
    }
    for value in [
        MAGIC as usize,
        0,
        count,
        originals,
        translations,
        0,
        text_start,
    ] {
        data.extend(word(value));
    }
    data.extend(tables);
    data.extend(text);
    data
}

fn word(value: usize) -> [u8; 4] {
    u32::try_from(value).unwrap_or(u32::MAX).to_le_bytes()
}
