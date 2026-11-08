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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Online {
    pub protocol: String,
    pub url: String,
}

pub struct Pack {
    file: Option<File>,
    pub path: PathBuf,
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
    pub online: Option<Online>,
    default: Option<bool>,
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
        let mut pack = Self {
            file: None,
            path: path.to_owned(),
            forms: 0,
            offsets_at: 0,
            id: String::new(),
            name: String::new(),
            attribution: String::new(),
            languages: Vec::new(),
            priority: 0,
            definitions: HashMap::new(),
            kinds: HashMap::new(),
            rules: HashMap::new(),
            compound: HashMap::new(),
            online: None,
            default: None,
        };
        let meta = match path
            .extension()
            .is_some_and(|extension| extension == SOURCE)
        {
            true => fs::read_to_string(path)?,
            false => {
                let file = File::open(path)?;
                let mut magic = [0; 8];
                file.read_exact_at(&mut magic, 0)?;
                if &magic != MAGIC {
                    return Err(invalid("not a dictionary pack"));
                }
                let meta_length = u32_at(&file, 8)?;
                let mut meta = vec![0; meta_length as usize];
                file.read_exact_at(&mut meta, HEADER_LENGTH)?;
                let forms_at = HEADER_LENGTH + u64::from(meta_length);
                pack.forms = u32_at(&file, forms_at)?;
                pack.offsets_at = forms_at + 4;
                pack.file = Some(file);
                String::from_utf8(meta).map_err(|_| invalid("metadata is not UTF-8"))?
            }
        };
        pack.read_meta(&meta);
        let online = pack.online.is_some();
        match pack.id.is_empty() || pack.languages.is_empty() || (online == pack.file.is_some()) {
            true => Err(invalid(
                "a source needs an id, languages and either data or an address",
            )),
            false => Ok(pack),
        }
    }

    fn read_meta(&mut self, meta: &str) {
        let mut protocol = None;
        let mut url = None;
        for line in meta.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.split_once('.') {
                Some(("rules", language)) => {
                    self.rules.insert(language.to_owned(), parse_rules(value));
                }
                Some(("definitions", language)) => {
                    self.definitions
                        .insert(language.to_owned(), value.to_owned());
                }
                Some(("kind", language)) => {
                    self.kinds.insert(language.to_owned(), value.to_owned());
                }
                Some(("compound", language)) => {
                    if let Ok(shortest) = value.parse() {
                        self.compound.insert(language.to_owned(), shortest);
                    }
                }
                _ => match key {
                    "id" => value.clone_into(&mut self.id),
                    "name" => value.clone_into(&mut self.name),
                    "attribution" => value.clone_into(&mut self.attribution),
                    "priority" => self.priority = value.parse().unwrap_or(0),
                    "default" => self.default = Some(value == "on"),
                    "protocol" => protocol = Some(value.to_owned()),
                    "url" => url = Some(value.to_owned()),
                    "languages" => {
                        self.languages = value.split_whitespace().map(str::to_owned).collect();
                    }
                    _ => {}
                },
            }
        }
        if let (Some(protocol), Some(url)) = (protocol, url) {
            self.online = Some(Online { protocol, url });
        }
    }

    pub fn is_users(&self) -> bool {
        bande_xdg::data_home().is_some_and(|home| self.path.starts_with(home.join(PACK_FOLDER)))
    }

    fn serves(&self, language: &str) -> bool {
        self.languages
            .iter()
            .any(|known| known == language || known == ANY_LANGUAGE)
    }

    fn on_by_default(&self, languages: &[String]) -> bool {
        self.default
            .unwrap_or_else(|| languages.iter().any(|language| self.serves(language)))
    }

    fn default_place(&self, languages: &[String]) -> (usize, u8, i64) {
        let first = self
            .languages
            .first()
            .map(String::as_str)
            .unwrap_or_default();
        let defined_in = self.definitions.get(first).map_or(first, String::as_str);
        let place = languages
            .iter()
            .position(|language| language == defined_in)
            .unwrap_or(languages.len());
        (place, self.kind_rank(first), -self.priority)
    }

    pub fn kind_rank(&self, language: &str) -> u8 {
        let kind = self
            .kinds
            .get(language)
            .or_else(|| self.kinds.get(ANY_LANGUAGE))
            .map(String::as_str);
        match kind {
            None | Some("definition") => 0,
            Some("translation") => 1,
            Some("thesaurus") => 2,
            Some("encyclopedia") => 3,
            Some(_) => 4,
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
            place: 0,
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

    fn data(&self) -> io::Result<&File> {
        self.file
            .as_ref()
            .ok_or_else(|| invalid("an online source has no data"))
    }

    fn record(&self, index: u32) -> io::Result<(Vec<u8>, Vec<u32>)> {
        let file = self.data()?;
        let at = u64::from(u32_at(file, self.offsets_at + u64::from(index) * 4)?);
        let mut bytes = vec![0; LONGEST_RECORD];
        let read = file.read_at(&mut bytes, at)?;
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
        let file = self.data()?;
        let length = u32_at(file, u64::from(at))?;
        let mut body = vec![0; length as usize];
        file.read_exact_at(&mut body, u64::from(at) + 4)?;
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
                    .is_some_and(|extension| extension == PACK || extension == SOURCE)
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
    pub place: usize,
    pub id: String,
    pub source: String,
    pub attribution: String,
    pub language: String,
    pub matched: Match,
    pub kind: u8,
    pub entries: Vec<Entry>,
    pub derived: Option<(String, String)>,
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

pub fn look_up(sources: &[Pack], text: &str, word_languages: &[String]) -> io::Result<Vec<Hit>> {
    let form = cleaned(text).replace('-', " ");
    let mut hits = hits_for(sources, &form)?;
    if hits.is_empty() {
        hits = compound_hits(sources, &form)?;
    }
    hits.sort_by_cached_key(|(index, hit)| {
        (*index, position_or_last(word_languages, &hit.language))
    });
    let mut unique: Vec<Hit> = Vec::new();
    for (place, mut hit) in hits {
        hit.place = place;
        if unique.iter().all(|shown| shown.entries != hit.entries) {
            unique.push(hit);
        }
    }
    Ok(unique)
}

const PACK: &str = "bdict";
const SOURCE: &str = "source";
const ANY_LANGUAGE: &str = "*";
pub const PACK_FOLDER: &str = "bande/dict";

pub fn installed() -> Vec<Pack> {
    let folders: Vec<PathBuf> = bande_xdg::data_dirs()
        .into_iter()
        .map(|folder| folder.join(PACK_FOLDER))
        .collect();
    discover(&folders)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Choices {
    pub order: Vec<String>,
    pub on: Vec<String>,
    pub off: Vec<String>,
}

fn ids(value: &str) -> Vec<String> {
    value.split_whitespace().map(str::to_owned).collect()
}

impl Choices {
    pub const FILE: &str = "dictionary.conf";

    pub fn parse(text: &str) -> Self {
        let mut choices = Self::default();
        for line in text.lines() {
            match line.split_once('=') {
                Some(("order", value)) => choices.order = ids(value),
                Some(("on", value)) => choices.on = ids(value),
                Some(("off", value)) => choices.off = ids(value),
                _ => {}
            }
        }
        choices
    }

    pub fn to_text(&self) -> String {
        format!(
            "order={}\non={}\noff={}\n",
            self.order.join(" "),
            self.on.join(" "),
            self.off.join(" ")
        )
    }

    pub fn load() -> Self {
        bande_xdg::config_home()
            .and_then(|folder| fs::read_to_string(folder.join("bande").join(Self::FILE)).ok())
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    pub fn enabled(&self, pack: &Pack, languages: &[String]) -> bool {
        self.on.contains(&pack.id)
            || (!self.off.contains(&pack.id) && pack.on_by_default(languages))
    }

    pub fn set_enabled(&mut self, id: &str, on: bool) {
        self.on.retain(|known| known != id);
        self.off.retain(|known| known != id);
        match on {
            true => self.on.push(id.to_owned()),
            false => self.off.push(id.to_owned()),
        }
    }

    pub fn arrange(&self, mut packs: Vec<Pack>, languages: &[String]) -> Vec<Pack> {
        packs.sort_by_cached_key(|pack| {
            let chosen = self.order.iter().position(|id| *id == pack.id);
            (chosen.is_none(), chosen, pack.default_place(languages))
        });
        packs
    }
}

pub fn user_languages() -> Vec<String> {
    let mut languages = bande_l10n::preferred_languages();
    if let Some(region) = bande_l10n::region_language()
        && !languages.contains(&region)
    {
        languages.push(region);
    }
    languages
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

        let inflected = look_up(&packs, "Blorpko", &[])?;
        assert_eq!(inflected[0].source, "Zzq words");
        assert_eq!(inflected[0].entries[0].word, "blorp");
        assert_eq!(inflected[0].matched, Match::Inflected);

        let compound = look_up(&packs, "snigblorp", &[])?;
        assert_eq!(compound[0].matched, Match::CompoundPart);
        assert_eq!(
            compound[0].derived,
            Some(("snig".to_owned(), "blorp".to_owned()))
        );
        Ok(())
    }

    #[test]
    fn sources_follow_the_users_languages_and_order() -> io::Result<()> {
        let folder = tempfile::tempdir()?;
        let pack = |file: &str, meta: &str, entries: &[(&str, &str)]| {
            write_pack(&folder.path().join(file), meta, entries)
        };
        pack(
            "a.bdict",
            "id=a\nname=A\nlanguages=zzq\ndefinitions.zzq=xx\nkind.zzq=translation\n",
            &[("zzq:blorp", "blorp\nnoun\nT\tin xx\n")],
        )?;
        pack(
            "b.bdict",
            "id=b\nname=B\nlanguages=zzq\ndefinitions.zzq=yy\npriority=1\n",
            &[("zzq:blorp", "blorp\nnoun\nD\tin yy\n")],
        )?;
        pack(
            "c.bdict",
            "id=c\nname=C\nlanguages=zzq\ndefinitions.zzq=yy\npriority=9\n",
            &[("zzq:blorp", "blorp\nnoun\nD\tin yy\n")],
        )?;
        pack(
            "d.bdict",
            "id=d\nname=D\nlanguages=qqz\n",
            &[("qqz:blorp", "blorp\nnoun\nD\tin qqz\n")],
        )?;
        fs::write(
            folder.path().join("e.source"),
            "id=e\nname=E\nlanguages=*\nprotocol=test\nurl=test://{word}\n",
        )?;
        let sources =
            |choices: &Choices, languages: &[&str]| -> io::Result<(Vec<String>, Vec<String>)> {
                let languages: Vec<String> = languages.iter().map(ToString::to_string).collect();
                let arranged = choices.arrange(discover(&[folder.path().to_owned()]), &languages);
                let listed = arranged.iter().map(|pack| pack.id.clone()).collect();
                let enabled: Vec<Pack> = arranged
                    .into_iter()
                    .filter(|pack| choices.enabled(pack, &languages))
                    .collect();
                let hits = look_up(&enabled, "blorp", &languages)?;
                Ok((listed, hits.into_iter().map(|hit| hit.id).collect()))
            };

        let (listed, found) = sources(&Choices::default(), &["yy", "zzq"])?;
        assert_eq!(listed, ["c", "b", "d", "e", "a"]);
        assert_eq!(found, ["c", "a"]);

        let mut choices = Choices {
            order: vec!["d".to_owned(), "a".to_owned()],
            ..Choices::default()
        };
        choices.set_enabled("d", true);
        choices.set_enabled("c", false);
        let (listed, found) = sources(&choices, &["yy", "zzq"])?;
        assert_eq!(listed, ["d", "a", "c", "b", "e"]);
        assert_eq!(found, ["d", "a", "b"]);
        Ok(())
    }
}
