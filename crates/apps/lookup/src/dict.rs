use std::collections::HashMap;
use std::fs::{self, File};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"BDICT1\n\0";
const HEADER_LENGTH: u64 = 12;
const MAX_FORM_LENGTH: usize = 512;
const MIN_FIRST_PART: usize = 2;
const LONGEST_RECORD: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    pos: String,
    ending: String,
    replacement: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub kind: char,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub word: String,
    pub pos: String,
    pub lines: Vec<Line>,
}

impl Entry {
    const FORM_OF: char = 'O';

    fn form_of(&self) -> Option<&str> {
        self.lines
            .iter()
            .find(|line| line.kind == Self::FORM_OF)
            .map(|line| line.text.as_str())
    }
}

pub struct Pack {
    file: File,
    pub id: String,
    pub name: String,
    pub attribution: String,
    pub languages: Vec<String>,
    priority: i64,
    definitions: HashMap<String, String>,
    kinds: HashMap<String, String>,
    rules: HashMap<String, Vec<Rule>>,
    compound: HashMap<String, usize>,
    forms: u32,
    offsets_at: u64,
}

fn u32_at(file: &File, at: u64) -> io::Result<u32> {
    let mut bytes = [0; 4];
    file.read_exact_at(&mut bytes, at)?;
    Ok(u32::from_le_bytes(bytes))
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

fn parse_rules(text: &str) -> Vec<Rule> {
    text.split_whitespace()
        .filter_map(|rule| {
            let mut parts = rule.split(':');
            Some(Rule {
                pos: parts.next()?.to_owned(),
                ending: parts.next()?.to_owned(),
                replacement: parts.next()?.to_owned(),
            })
        })
        .collect()
}

impl Pack {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let mut magic = [0; 8];
        file.read_exact_at(&mut magic, 0)?;
        if &magic != MAGIC {
            return Err(invalid("not a dictionary pack"));
        }
        let meta_length = u32_at(&file, 8)?;
        let mut meta = vec![0; meta_length as usize];
        file.read_exact_at(&mut meta, HEADER_LENGTH)?;
        let meta = String::from_utf8(meta).map_err(|_| invalid("metadata is not UTF-8"))?;
        let forms_at = HEADER_LENGTH + u64::from(meta_length);
        let mut pack = Self {
            forms: u32_at(&file, forms_at)?,
            offsets_at: forms_at + 4,
            file,
            id: String::new(),
            name: String::new(),
            attribution: String::new(),
            languages: Vec::new(),
            priority: 0,
            definitions: HashMap::new(),
            kinds: HashMap::new(),
            rules: HashMap::new(),
            compound: HashMap::new(),
        };
        for line in meta.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.split_once('.') {
                Some(("rules", language)) => {
                    pack.rules.insert(language.to_owned(), parse_rules(value));
                }
                Some(("definitions", language)) => {
                    pack.definitions
                        .insert(language.to_owned(), value.to_owned());
                }
                Some(("kind", language)) => {
                    pack.kinds.insert(language.to_owned(), value.to_owned());
                }
                Some(("compound", language)) => {
                    if let Ok(shortest) = value.parse() {
                        pack.compound.insert(language.to_owned(), shortest);
                    }
                }
                _ => match key {
                    "id" => value.clone_into(&mut pack.id),
                    "name" => value.clone_into(&mut pack.name),
                    "attribution" => value.clone_into(&mut pack.attribution),
                    "priority" => pack.priority = value.parse().unwrap_or(0),
                    "languages" => {
                        pack.languages = value.split_whitespace().map(str::to_owned).collect();
                    }
                    _ => {}
                },
            }
        }
        match pack.id.is_empty() || pack.languages.is_empty() {
            true => Err(invalid("a pack needs an id and languages")),
            false => Ok(pack),
        }
    }

    fn kind_rank(&self, language: &str) -> u8 {
        match self.kinds.get(language).map(String::as_str) {
            None | Some("definition") => 0,
            Some("translation") => 1,
            Some("thesaurus") => 2,
            Some(_) => 3,
        }
    }

    fn hit(
        &self,
        language: &str,
        matched: Match,
        entries: Vec<Entry>,
        derived: Option<(String, String)>,
    ) -> Hit {
        Hit {
            id: self.id.clone(),
            source: self.name.clone(),
            attribution: self.attribution.clone(),
            language: language.to_owned(),
            matched,
            kind: self.kind_rank(language),
            entries,
            derived,
        }
    }

    fn record(&self, index: u32) -> io::Result<(Vec<u8>, Vec<u32>)> {
        let at = u64::from(u32_at(&self.file, self.offsets_at + u64::from(index) * 4)?);
        let mut bytes = vec![0; LONGEST_RECORD];
        let read = self.file.read_at(&mut bytes, at)?;
        bytes.truncate(read);
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| invalid("unterminated form"))?;
        let count = bytes
            .get(end + 1..end + 3)
            .map(|count| u16::from_le_bytes([count[0], count[1]]))
            .ok_or_else(|| invalid("truncated record"))?;
        let entries = bytes
            .get(end + 3..end + 3 + usize::from(count) * 4)
            .ok_or_else(|| invalid("truncated record"))?
            .as_chunks::<4>()
            .0
            .iter()
            .map(|offset| u32::from_le_bytes(*offset))
            .collect();
        bytes.truncate(end);
        Ok((bytes, entries))
    }

    fn entry(&self, at: u32) -> io::Result<Entry> {
        let length = u32_at(&self.file, u64::from(at))?;
        let mut body = vec![0; length as usize];
        self.file.read_exact_at(&mut body, u64::from(at) + 4)?;
        let body = String::from_utf8(body).map_err(|_| invalid("entry is not UTF-8"))?;
        let mut lines = body.lines();
        let word = lines.next().unwrap_or_default().to_owned();
        let pos = lines.next().unwrap_or_default().to_owned();
        let lines = lines
            .filter_map(|line| {
                let (kind, text) = line.split_once('\t')?;
                Some(Line {
                    kind: kind.chars().next()?,
                    text: text.to_owned(),
                })
            })
            .collect();
        Ok(Entry { word, pos, lines })
    }

    pub fn find(&self, language: &str, form: &str) -> io::Result<Vec<Entry>> {
        let key = format!("{language}:{form}").into_bytes();
        if key.len() > MAX_FORM_LENGTH {
            return Ok(Vec::new());
        }
        let (mut low, mut high) = (0, self.forms);
        while low < high {
            let middle = low + (high - low) / 2;
            let (found, entries) = self.record(middle)?;
            match found.cmp(&key) {
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Greater => high = middle,
                std::cmp::Ordering::Equal => {
                    return entries.into_iter().map(|at| self.entry(at)).collect();
                }
            }
        }
        Ok(Vec::new())
    }

    fn inflected(&self, language: &str, form: &str) -> io::Result<Vec<Entry>> {
        let mut found: Vec<Entry> = Vec::new();
        let Some(rules) = self.rules.get(language) else {
            return Ok(found);
        };
        let (head, tail) = match form.split_once(' ') {
            Some((head, tail)) => (head, Some(tail)),
            None => (form, None),
        };
        for rule in rules {
            let Some(stem) = head.strip_suffix(rule.ending.as_str()) else {
                continue;
            };
            let mut base = format!("{stem}{}", rule.replacement);
            if let Some(tail) = tail {
                base = format!("{base} {tail}");
            }
            for entry in self.find(language, &base)? {
                if entry.pos == rule.pos && !found.contains(&entry) {
                    found.push(entry);
                }
            }
        }
        Ok(found)
    }

    fn resolve_forms(&self, language: &str, entries: Vec<Entry>) -> io::Result<Vec<Entry>> {
        let mut resolved: Vec<Entry> = Vec::new();
        for entry in entries.iter().filter(|entry| entry.form_of().is_none()) {
            if !resolved.contains(entry) {
                resolved.push(entry.clone());
            }
        }
        for entry in entries
            .iter()
            .filter_map(|entry| Some((entry, entry.form_of()?)))
        {
            let (form, target) = entry;
            let targets = self.find(language, &target.to_lowercase())?;
            let same_part: Vec<Entry> = targets
                .into_iter()
                .filter(|candidate| candidate.pos == form.pos && candidate.form_of().is_none())
                .collect();
            match same_part.is_empty() {
                true if resolved.is_empty() => resolved.push(form.clone()),
                true => {}
                false => {
                    for candidate in same_part {
                        if !resolved.contains(&candidate) {
                            resolved.push(candidate);
                        }
                    }
                }
            }
        }
        Ok(resolved)
    }

    fn matches(&self, language: &str, form: &str) -> io::Result<Vec<Entry>> {
        let mut found = self.find(language, form)?;
        if found.is_empty() {
            found = self.inflected(language, form)?;
        }
        self.resolve_forms(language, found)
    }
}

pub fn discover(folders: &[PathBuf]) -> Vec<Pack> {
    let mut packs: Vec<Pack> = Vec::new();
    for folder in folders {
        let Ok(files) = fs::read_dir(folder) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = files
            .filter_map(|file| Some(file.ok()?.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "bdict")
            })
            .collect();
        paths.sort();
        for path in paths {
            match Pack::open(&path) {
                Ok(pack) if packs.iter().all(|known| known.id != pack.id) => packs.push(pack),
                Ok(_) => {}
                Err(error) => tracing::warn!("cannot read {}: {error}", path.display()),
            }
        }
    }
    packs
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Match {
    Headword,
    Inflected,
    CompoundPart,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub id: String,
    pub source: String,
    pub attribution: String,
    pub language: String,
    pub matched: Match,
    pub kind: u8,
    pub entries: Vec<Entry>,
    pub derived: Option<(String, String)>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Preferences<'a> {
    pub word_languages: &'a [String],
    pub ui_languages: &'a [String],
    pub user_order: &'a [String],
}

pub fn cleaned(text: &str) -> String {
    text.trim_matches(|character: char| !character.is_alphanumeric())
        .to_lowercase()
}

fn longest_known_ending(
    packs: &[Pack],
    language: &str,
    form: &str,
) -> io::Result<Option<(String, String)>> {
    let shortest = packs
        .iter()
        .filter_map(|pack| pack.compound.get(language))
        .min()
        .copied();
    let Some(shortest) = shortest else {
        return Ok(None);
    };
    for (at, _) in form.char_indices().skip(MIN_FIRST_PART) {
        let ending = &form[at..];
        if ending.chars().count() < shortest {
            break;
        }
        for pack in packs
            .iter()
            .filter(|pack| pack.compound.contains_key(language))
        {
            if !pack.matches(language, ending)?.is_empty() {
                return Ok(Some((form[..at].to_owned(), ending.to_owned())));
            }
        }
    }
    Ok(None)
}

fn languages_of(packs: &[Pack]) -> Vec<String> {
    let mut languages: Vec<String> = Vec::new();
    for language in packs.iter().flat_map(|pack| pack.languages.iter()) {
        if !languages.contains(language) {
            languages.push(language.clone());
        }
    }
    languages
}

fn hits_for(packs: &[Pack], form: &str) -> io::Result<Vec<(usize, Hit)>> {
    let mut hits = Vec::new();
    for (index, pack) in packs.iter().enumerate() {
        for language in &pack.languages {
            let entries = pack.matches(language, form)?;
            if entries.is_empty() {
                continue;
            }
            let matched = match entries
                .iter()
                .any(|entry| entry.word.to_lowercase() == form)
            {
                true => Match::Headword,
                false => Match::Inflected,
            };
            hits.push((index, pack.hit(language, matched, entries, None)));
        }
    }
    Ok(hits)
}

fn compound_hits(packs: &[Pack], form: &str) -> io::Result<Vec<(usize, Hit)>> {
    let mut hits = Vec::new();
    for language in languages_of(packs) {
        let Some((first, ending)) = longest_known_ending(packs, &language, form)? else {
            continue;
        };
        for (index, pack) in packs.iter().enumerate() {
            if !pack.languages.contains(&language) {
                continue;
            }
            let entries = pack.matches(&language, &ending)?;
            if !entries.is_empty() {
                let parts = Some((first.clone(), ending.clone()));
                hits.push((
                    index,
                    pack.hit(&language, Match::CompoundPart, entries, parts),
                ));
            }
        }
    }
    Ok(hits)
}

fn position_or_last(list: &[String], wanted: &str) -> usize {
    list.iter()
        .position(|known| known == wanted)
        .unwrap_or(list.len())
}

pub fn look_up(packs: &[Pack], text: &str, wanted: Preferences<'_>) -> io::Result<Vec<Hit>> {
    let form = cleaned(text).replace('-', " ");
    let mut hits = hits_for(packs, &form)?;
    if hits.is_empty() {
        hits = compound_hits(packs, &form)?;
    }
    hits.sort_by_cached_key(|(index, hit)| {
        let pack = &packs[*index];
        let defined_in = pack
            .definitions
            .get(&hit.language)
            .map_or(hit.language.as_str(), String::as_str);
        let priority = match wanted.user_order.iter().position(|id| *id == pack.id) {
            Some(place) => (0, place as i64),
            None => (1, -pack.priority),
        };
        (
            hit.matched,
            position_or_last(wanted.ui_languages, defined_in),
            pack.kind_rank(&hit.language),
            priority,
            position_or_last(wanted.word_languages, &hit.language),
        )
    });
    let mut unique: Vec<Hit> = Vec::new();
    for (_, hit) in hits {
        if unique.iter().all(|shown| shown.entries != hit.entries) {
            unique.push(hit);
        }
    }
    Ok(unique)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_pack(path: &Path, meta: &str, entries: &[(&str, &str)]) -> io::Result<()> {
        let mut forms: Vec<(String, usize)> = Vec::new();
        for (index, (keys, _)) in entries.iter().enumerate() {
            forms.extend(keys.split(' ').map(|key| (key.to_owned(), index)));
        }
        forms.sort();
        let offsets_at = HEADER_LENGTH as usize + meta.len() + 4;
        let records: usize = forms.iter().map(|(key, _)| key.len() + 7).sum();
        let mut entry_at = offsets_at + forms.len() * 4 + records;
        let mut bodies = Vec::new();
        let mut places = Vec::new();
        for (_, body) in entries {
            places.push(entry_at as u32);
            bodies.extend((body.len() as u32).to_le_bytes());
            bodies.extend(body.as_bytes());
            entry_at += 4 + body.len();
        }
        let mut file = MAGIC.to_vec();
        file.extend((meta.len() as u32).to_le_bytes());
        file.extend(meta.as_bytes());
        file.extend((forms.len() as u32).to_le_bytes());
        let mut record_at = offsets_at + forms.len() * 4;
        let mut records = Vec::new();
        for (key, index) in &forms {
            file.extend((record_at as u32).to_le_bytes());
            records.extend(key.as_bytes());
            records.push(0);
            records.extend(1u16.to_le_bytes());
            records.extend(places[*index].to_le_bytes());
            record_at += key.len() + 7;
        }
        file.extend(records);
        file.extend(bodies);
        fs::write(path, file)
    }

    #[test]
    fn a_made_up_language_pack_works_without_code() -> io::Result<()> {
        let folder = tempfile::tempdir()?;
        write_pack(
            &folder.path().join("zzq.bdict"),
            "id=zzq-test\nname=Zzq words\nattribution=Made up\nlanguages=zzq\nrules.zzq=noun:ko:\ncompound.zzq=3\n",
            &[("zzq:blorp", "blorp\nnoun\nD\ta made-up thing\n")],
        )?;
        let packs = discover(&[folder.path().to_owned()]);

        let inflected = look_up(&packs, "Blorpko", Preferences::default())?;
        assert_eq!(inflected[0].source, "Zzq words");
        assert_eq!(inflected[0].entries[0].word, "blorp");
        assert_eq!(inflected[0].matched, Match::Inflected);

        let compound = look_up(&packs, "snigblorp", Preferences::default())?;
        assert_eq!(compound[0].matched, Match::CompoundPart);
        assert_eq!(
            compound[0].derived,
            Some(("snig".to_owned(), "blorp".to_owned()))
        );
        Ok(())
    }

    #[test]
    fn overlapping_packs_keep_their_own_sections_in_ranked_order() -> io::Result<()> {
        let folder = tempfile::tempdir()?;
        let pack = |file: &str, meta: &str, entries: &[(&str, &str)]| {
            write_pack(&folder.path().join(file), meta, entries)
        };
        pack(
            "a.bdict",
            "id=a\nname=A\nlanguages=zzq\ndefinitions.zzq=xx\nkind.zzq=translation\npriority=50\n",
            &[("zzq:blorp", "blorp\nnoun\nT\tin xx\n")],
        )?;
        pack(
            "b.bdict",
            "id=b\nname=B\nlanguages=zzq\ndefinitions.zzq=yy\nkind.zzq=definition\npriority=1\n",
            &[("zzq:blorp", "blorp\nnoun\nD\tin yy\n")],
        )?;
        pack(
            "c.bdict",
            "id=c\nname=C\nlanguages=zzq\ndefinitions.zzq=yy\nkind.zzq=definition\npriority=9\n",
            &[("zzq:blorp", "blorp\nnoun\nD\tother in yy\n")],
        )?;
        pack(
            "d.bdict",
            "id=d\nname=D\nlanguages=zzq\ndefinitions.zzq=yy\nkind.zzq=definition\npriority=2\n",
            &[("zzq:blorp", "blorp\nnoun\nD\tin yy\n")],
        )?;
        pack(
            "e.bdict",
            "id=e\nname=E\nlanguages=zzq\ndefinitions.zzq=yy\npriority=99\n",
            &[("zzq:blorpa", "blorp\nnoun\nD\tan inflected match\n")],
        )?;
        let packs = discover(&[folder.path().to_owned()]);
        let order = |form: &str, ui: &[&str], user: &[&str]| -> io::Result<Vec<String>> {
            let ui: Vec<String> = ui.iter().map(ToString::to_string).collect();
            let user_order: Vec<String> = user.iter().map(ToString::to_string).collect();
            let hits = look_up(
                &packs,
                form,
                Preferences {
                    ui_languages: &ui,
                    user_order: &user_order,
                    ..Preferences::default()
                },
            )?;
            Ok(hits.into_iter().map(|hit| hit.id).collect())
        };

        assert_eq!(order("blorp", &["xx"], &[])?, ["a", "c", "d"]);
        assert_eq!(order("blorp", &["yy"], &[])?, ["c", "d", "a"]);
        assert_eq!(order("blorp", &["yy"], &["b"])?, ["b", "c", "a"]);
        assert_eq!(order("blorpa", &["yy"], &[])?, ["e"]);
        Ok(())
    }
}
