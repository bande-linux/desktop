use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::Read;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::entry::Locale;

pub const DIRECTORY: &str = "inode/directory";
pub const TEXT_PLAIN: &str = "text/plain";
pub const OCTET_STREAM: &str = "application/octet-stream";

const MAGIC_HEADER: &[u8] = b"MIME-Magic\0\n";
const MAX_SNIFF_BYTES: usize = 64 * 1024;
const TEXT_SNIFF_BYTES: usize = 4096;

#[derive(Debug, Clone)]
struct Glob {
    weight: u32,
    mime: String,
    pattern: String,
    case_sensitive: bool,
}

impl Glob {
    fn matches(&self, name: &str) -> bool {
        if self.case_sensitive {
            wildcard_match(self.pattern.as_bytes(), name.as_bytes())
        } else {
            wildcard_match(
                self.pattern.to_lowercase().as_bytes(),
                name.to_lowercase().as_bytes(),
            )
        }
    }
}

fn class_match(class: &[u8], byte: u8) -> Option<(bool, usize)> {
    let mut index = 0;
    let negated = matches!(class.first(), Some(b'!' | b'^'));
    if negated {
        index += 1;
    }
    let mut matched = false;
    let mut first = true;
    while index < class.len() {
        let low = class[index];
        if low == b']' && !first {
            return Some((matched != negated, index + 1));
        }
        first = false;
        if class.get(index + 1) == Some(&b'-') && class.get(index + 2).is_some_and(|c| *c != b']') {
            let high = class[index + 2];
            matched |= (low..=high).contains(&byte);
            index += 3;
        } else {
            matched |= low == byte;
            index += 1;
        }
    }
    None
}

fn wildcard_match(pattern: &[u8], name: &[u8]) -> bool {
    let (mut p, mut n) = (0, 0);
    let mut backtrack: Option<(usize, usize)> = None;
    while n < name.len() {
        let step = match pattern.get(p) {
            Some(b'*') => {
                backtrack = Some((p, n));
                p += 1;
                continue;
            }
            Some(b'?') => Some(1),
            Some(b'[') => match class_match(&pattern[p + 1..], name[n]) {
                Some((true, length)) => Some(length + 1),
                Some((false, _)) => None,
                None => (name[n] == b'[').then_some(1),
            },
            Some(literal) => (*literal == name[n]).then_some(1),
            None => None,
        };
        match (step, backtrack) {
            (Some(length), _) => {
                p += length;
                n += 1;
            }
            (None, Some((star, start))) => {
                p = star + 1;
                n = start + 1;
                backtrack = Some((star, start + 1));
            }
            (None, None) => return false,
        }
    }
    pattern[p..].iter().all(|byte| *byte == b'*')
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Matchlet {
    indent: usize,
    offset: usize,
    value: Vec<u8>,
    mask: Option<Vec<u8>>,
    range: usize,
}

impl Matchlet {
    fn matches_at(&self, data: &[u8], at: usize) -> bool {
        let Some(window) = data.get(at..at + self.value.len()) else {
            return false;
        };
        match &self.mask {
            Some(mask) => window
                .iter()
                .zip(&self.value)
                .zip(mask)
                .all(|((byte, value), mask)| byte & mask == value & mask),
            None => window == self.value.as_slice(),
        }
    }

    fn matches(&self, data: &[u8]) -> bool {
        (self.offset..self.offset + self.range).any(|at| self.matches_at(data, at))
    }

    fn extent(&self) -> usize {
        self.offset + self.range + self.value.len()
    }
}

#[derive(Debug, Clone)]
struct MagicRule {
    priority: u32,
    mime: String,
    matchlets: Vec<Matchlet>,
}

impl MagicRule {
    fn matches(&self, data: &[u8]) -> bool {
        self.matches_from(data, 0, 0)
    }

    fn matches_from(&self, data: &[u8], start: usize, indent: usize) -> bool {
        let mut index = start;
        while let Some(matchlet) = self.matchlets.get(index) {
            if matchlet.indent < indent {
                return false;
            }
            if matchlet.indent == indent && matchlet.matches(data) {
                let child = self.matchlets.get(index + 1);
                match child {
                    Some(child) if child.indent > indent => {
                        if self.matches_from(data, index + 1, indent + 1) {
                            return true;
                        }
                    }
                    _ => return true,
                }
            }
            index += 1;
        }
        false
    }
}

fn number(bytes: &[u8], at: &mut usize) -> Option<usize> {
    let start = *at;
    while bytes.get(*at).is_some_and(u8::is_ascii_digit) {
        *at += 1;
    }
    std::str::from_utf8(&bytes[start..*at]).ok()?.parse().ok()
}

fn swap_words(bytes: &mut [u8], word: usize) {
    if cfg!(target_endian = "little") && word > 1 {
        bytes.chunks_exact_mut(word).for_each(<[u8]>::reverse);
    }
}

fn parse_matchlet(bytes: &[u8], at: &mut usize) -> Option<Matchlet> {
    let indent = if bytes.get(*at) == Some(&b'>') {
        0
    } else {
        number(bytes, at)?
    };
    (bytes.get(*at) == Some(&b'>')).then_some(())?;
    *at += 1;
    let offset = number(bytes, at)?;
    (bytes.get(*at) == Some(&b'=')).then_some(())?;
    *at += 1;
    let length = usize::from(u16::from_be_bytes([*bytes.get(*at)?, *bytes.get(*at + 1)?]));
    *at += 2;
    let mut value = bytes.get(*at..*at + length)?.to_vec();
    *at += length;
    let mut mask = None;
    if bytes.get(*at) == Some(&b'&') {
        mask = Some(bytes.get(*at + 1..*at + 1 + length)?.to_vec());
        *at += 1 + length;
    }
    let mut word = 1;
    if bytes.get(*at) == Some(&b'~') {
        *at += 1;
        word = number(bytes, at)?;
    }
    let mut range = 1;
    if bytes.get(*at) == Some(&b'+') {
        *at += 1;
        range = number(bytes, at)?;
    }
    while bytes.get(*at).is_some_and(|byte| *byte != b'\n') {
        *at += 1;
    }
    *at += 1;
    swap_words(&mut value, word);
    if let Some(mask) = &mut mask {
        swap_words(mask, word);
    }
    Some(Matchlet {
        indent,
        offset,
        value,
        mask,
        range: range.max(1),
    })
}

fn parse_magic(bytes: &[u8]) -> Vec<MagicRule> {
    let Some(mut at) = bytes.strip_prefix(MAGIC_HEADER).map(|_| MAGIC_HEADER.len()) else {
        return Vec::new();
    };
    let mut rules = Vec::new();
    while bytes.get(at) == Some(&b'[') {
        let Some(end) = bytes[at..].iter().position(|byte| *byte == b'\n') else {
            break;
        };
        let header = String::from_utf8_lossy(&bytes[at + 1..at + end]).into_owned();
        at += end + 1;
        let Some((priority, mime)) = header.trim_end_matches(']').split_once(':') else {
            break;
        };
        let mut matchlets = Vec::new();
        while bytes.get(at).is_some_and(|byte| *byte != b'[') {
            match parse_matchlet(bytes, &mut at) {
                Some(matchlet) => matchlets.push(matchlet),
                None => return rules,
            }
        }
        rules.push(MagicRule {
            priority: priority.parse().unwrap_or(50),
            mime: mime.to_owned(),
            matchlets,
        });
    }
    rules
}

fn pairs(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once(' '))
}

fn looks_like_text(data: &[u8]) -> bool {
    let data = &data[..data.len().min(TEXT_SNIFF_BYTES)];
    if data.contains(&0) {
        return false;
    }
    match std::str::from_utf8(data) {
        Ok(_) => true,
        Err(error) => error.error_len().is_none(),
    }
}

#[derive(Debug, Default)]
pub struct MimeDatabase {
    mime_dirs: Vec<PathBuf>,
    sniff_bytes: usize,
    literals: HashMap<String, Vec<Glob>>,
    suffixes: HashMap<String, Vec<Glob>>,
    patterns: Vec<Glob>,
    magic: Vec<MagicRule>,
    subclasses: HashMap<String, Vec<String>>,
    aliases: HashMap<String, String>,
    icons: HashMap<String, String>,
    generic_icons: HashMap<String, String>,
}

impl MimeDatabase {
    pub fn load(data_dirs: &[PathBuf]) -> Self {
        let mime_dirs = data_dirs
            .iter()
            .map(|dir| dir.join("mime"))
            .collect::<Vec<_>>();
        let mut globs: Vec<Glob> = Vec::new();
        let mut magic = Vec::new();
        let mut subclasses: HashMap<String, Vec<String>> = HashMap::new();
        let mut aliases = HashMap::new();
        let mut icons = HashMap::new();
        let mut generic_icons = HashMap::new();
        for dir in mime_dirs.iter().rev() {
            if let Ok(text) = fs::read_to_string(dir.join("globs2")) {
                for line in text.lines().filter(|line| !line.starts_with('#')) {
                    let mut fields = line.split(':');
                    let (Some(weight), Some(mime), Some(pattern)) =
                        (fields.next(), fields.next(), fields.next())
                    else {
                        continue;
                    };
                    if pattern == "__NOGLOBS__" {
                        globs.retain(|glob| glob.mime != mime);
                        continue;
                    }
                    let flags = fields.next().unwrap_or_default();
                    globs.push(Glob {
                        weight: weight.parse().unwrap_or(50),
                        mime: mime.to_owned(),
                        pattern: pattern.to_owned(),
                        case_sensitive: flags.split(',').any(|flag| flag == "cs"),
                    });
                }
            }
            if let Ok(bytes) = fs::read(dir.join("magic")) {
                let rules = parse_magic(&bytes);
                let replaced = rules
                    .iter()
                    .map(|rule| rule.mime.clone())
                    .collect::<HashSet<_>>();
                magic.retain(|rule: &MagicRule| !replaced.contains(&rule.mime));
                magic.extend(rules);
            }
            if let Ok(text) = fs::read_to_string(dir.join("subclasses")) {
                for (child, parent) in pairs(&text) {
                    let parents = subclasses.entry(child.to_owned()).or_default();
                    if !parents.iter().any(|known| known == parent) {
                        parents.push(parent.to_owned());
                    }
                }
            }
            for (file, map) in [("icons", &mut icons), ("generic-icons", &mut generic_icons)] {
                if let Ok(text) = fs::read_to_string(dir.join(file)) {
                    map.extend(text.lines().filter_map(|line| {
                        let (mime, icon) = line.split_once(':')?;
                        Some((mime.to_owned(), icon.to_owned()))
                    }));
                }
            }
            if let Ok(text) = fs::read_to_string(dir.join("aliases")) {
                for (alias, mime) in pairs(&text) {
                    aliases.insert(alias.to_owned(), mime.to_owned());
                }
            }
        }
        magic.sort_by_key(|rule| std::cmp::Reverse(rule.priority));
        let sniff_bytes = magic
            .iter()
            .flat_map(|rule| &rule.matchlets)
            .map(Matchlet::extent)
            .max()
            .unwrap_or(0)
            .clamp(TEXT_SNIFF_BYTES, MAX_SNIFF_BYTES);
        let mut database = Self {
            mime_dirs,
            sniff_bytes,
            magic,
            subclasses,
            aliases,
            icons,
            generic_icons,
            ..Self::default()
        };
        for glob in globs {
            let pattern = glob.pattern.as_str();
            let wild = |text: &str| text.contains(['*', '?', '[']);
            if !wild(pattern) {
                let key = if glob.case_sensitive {
                    pattern.to_owned()
                } else {
                    pattern.to_lowercase()
                };
                database.literals.entry(key).or_default().push(glob);
            } else if let Some(suffix) = pattern.strip_prefix('*')
                && !wild(suffix)
            {
                let key = if glob.case_sensitive {
                    suffix.to_owned()
                } else {
                    suffix.to_lowercase()
                };
                database.suffixes.entry(key).or_default().push(glob);
            } else {
                database.patterns.push(glob);
            }
        }
        database
    }

    fn glob_candidates(&self, name: &str) -> Vec<&Glob> {
        let lower = name.to_lowercase();
        let literal = |key: &str, folded: bool| {
            self.literals
                .get(key)
                .into_iter()
                .flatten()
                .filter(move |glob| glob.case_sensitive != folded)
        };
        let literals = literal(name, false)
            .chain(literal(&lower, true))
            .collect::<Vec<_>>();
        if !literals.is_empty() {
            return literals;
        }
        let mut matches = Vec::new();
        for (index, _) in name.char_indices() {
            if let Some(found) = self.suffixes.get(&name[index..]) {
                matches.extend(found.iter().filter(|glob| glob.case_sensitive));
            }
        }
        for (index, _) in lower.char_indices() {
            if let Some(found) = self.suffixes.get(&lower[index..]) {
                matches.extend(found.iter().filter(|glob| !glob.case_sensitive));
            }
        }
        matches.extend(self.patterns.iter().filter(|glob| glob.matches(name)));
        let Some(best) = matches
            .iter()
            .map(|glob| (glob.weight, glob.pattern.len()))
            .max()
        else {
            return Vec::new();
        };
        let mut best_matches = matches
            .into_iter()
            .filter(|glob| (glob.weight, glob.pattern.len()) == best)
            .collect::<Vec<_>>();
        best_matches.dedup_by(|a, b| a.mime == b.mime);
        best_matches
    }

    pub fn for_name(&self, name: &str) -> Option<&str> {
        self.glob_candidates(name)
            .first()
            .map(|glob| self.unalias(&glob.mime))
    }

    pub fn for_data(&self, data: &[u8]) -> Option<&str> {
        self.magic
            .iter()
            .find(|rule| rule.matches(data))
            .map(|rule| self.unalias(&rule.mime))
    }

    fn sniff(&self, path: &Path) -> Vec<u8> {
        let mut data = Vec::new();
        let opened = fs::OpenOptions::new()
            .read(true)
            .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
            .open(path);
        if let Ok(file) = opened {
            let _ = file.take(self.sniff_bytes as u64).read_to_end(&mut data);
        }
        data
    }

    pub fn for_path(&self, path: &Path) -> String {
        let file_type = fs::metadata(path).map(|metadata| metadata.file_type());
        if let Ok(file_type) = file_type {
            let special = if file_type.is_dir() {
                Some(DIRECTORY)
            } else if file_type.is_fifo() {
                Some("inode/fifo")
            } else if file_type.is_socket() {
                Some("inode/socket")
            } else if file_type.is_char_device() {
                Some("inode/chardevice")
            } else if file_type.is_block_device() {
                Some("inode/blockdevice")
            } else {
                None
            };
            if let Some(special) = special {
                return special.to_owned();
            }
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let candidates = self.glob_candidates(&name);
        if let [only] = candidates.as_slice() {
            return self.unalias(&only.mime).to_owned();
        }
        let data = self.sniff(path);
        if let Some(sniffed) = self.for_data(&data) {
            let refines = candidates.is_empty()
                || candidates
                    .iter()
                    .any(|glob| self.is_a(sniffed, &glob.mime) || self.is_a(&glob.mime, sniffed));
            if refines {
                return sniffed.to_owned();
            }
        }
        if let Some(first) = candidates.first() {
            return self.unalias(&first.mime).to_owned();
        }
        if looks_like_text(&data) {
            TEXT_PLAIN.to_owned()
        } else {
            OCTET_STREAM.to_owned()
        }
    }

    pub fn unalias<'a>(&'a self, mime: &'a str) -> &'a str {
        self.aliases.get(mime).map_or(mime, String::as_str)
    }

    pub fn ancestors(&self, mime: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut pending = VecDeque::from([self.unalias(mime).to_owned()]);
        while let Some(current) = pending.pop_front() {
            if found.contains(&current) {
                continue;
            }
            let mut parents = self.subclasses.get(&current).cloned().unwrap_or_default();
            if current.starts_with("text/") && current != TEXT_PLAIN {
                parents.push(TEXT_PLAIN.to_owned());
            }
            pending.extend(parents.iter().map(|parent| self.unalias(parent).to_owned()));
            found.push(current);
        }
        found
    }

    pub fn is_a(&self, mime: &str, parent: &str) -> bool {
        let parent = self.unalias(parent);
        self.ancestors(mime)
            .iter()
            .any(|ancestor| ancestor == parent)
    }

    pub fn icon_names(&self, mime: &str) -> Vec<String> {
        let mime = self.unalias(mime);
        let media = mime.split('/').next().unwrap_or(mime);
        let specific = self
            .icons
            .get(mime)
            .cloned()
            .unwrap_or_else(|| mime.replace('/', "-"));
        let generic = self
            .generic_icons
            .get(mime)
            .cloned()
            .unwrap_or_else(|| format!("{media}-x-generic"));
        let mut names = vec![specific, generic];
        if media == "text" {
            names.push("text-x-generic".to_owned());
        }
        names.push("application-x-generic".to_owned());
        names.dedup();
        names
    }

    pub fn description(&self, mime: &str, locale: Option<&Locale>) -> Option<String> {
        let mime = self.unalias(mime);
        let text = self
            .mime_dirs
            .iter()
            .find_map(|dir| fs::read_to_string(dir.join(format!("{mime}.xml"))).ok())?;
        let comments = comments(&text);
        locale
            .into_iter()
            .flat_map(Locale::keys)
            .find_map(|key| {
                comments
                    .iter()
                    .find(|(lang, _)| lang.as_deref() == Some(key.as_str()))
            })
            .or_else(|| comments.iter().find(|(lang, _)| lang.is_none()))
            .map(|(_, comment)| comment.clone())
    }
}

fn unescape_xml(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn comments(text: &str) -> Vec<(Option<String>, String)> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<comment") {
        rest = &rest[start + "<comment".len()..];
        let Some(close) = rest.find('>') else {
            break;
        };
        let attributes = &rest[..close];
        rest = &rest[close + 1..];
        let Some(end) = rest.find("</comment>") else {
            break;
        };
        let lang = attributes
            .split_once("xml:lang=\"")
            .and_then(|(_, value)| value.split_once('"'))
            .map(|(lang, _)| lang.to_owned());
        found.push((lang, unescape_xml(&rest[..end])));
        rest = &rest[end..];
    }
    found
}
