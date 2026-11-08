use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use bande_dict::{Line, NewEntry, PACK_FOLDER};
use flate2::read::GzDecoder;

use crate::L10N;

const DICT_PORT: u16 = 2628;
const DICT_TIMEOUT: Duration = Duration::from_secs(10);
const ANY_LANGUAGE: &str = "*";
const TARBALLS: [&str; 5] = [".tar.gz", ".tgz", ".tar.bz2", ".tar.xz", ".tar"];

#[derive(Debug)]
pub enum Failure {
    Unsupported,
    Broken(String),
}

impl Failure {
    pub fn message(&self, name: &str) -> String {
        match self {
            Self::Unsupported => L10N.tr_args("import-unsupported", [("name", name.into())]),
            Self::Broken(detail) => L10N.tr_args(
                "import-broken",
                [("name", name.into()), ("detail", detail.as_str().into())],
            ),
        }
    }
}

impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        Self::Broken(error.to_string())
    }
}

fn broken(detail: &str) -> Failure {
    Failure::Broken(detail.to_owned())
}

fn users_folder() -> Result<PathBuf, Failure> {
    bande_xdg::data_home()
        .map(|home| home.join(PACK_FOLDER))
        .ok_or_else(|| broken("no data folder"))
}

fn identifier(name: &str) -> String {
    let readable: String = name
        .chars()
        .map(|character| match character.is_ascii_alphanumeric() {
            true => character.to_ascii_lowercase(),
            false => '-',
        })
        .collect();
    let hash = name.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
    });
    format!("user-{}-{:08x}", readable.trim_matches('-'), hash as u32)
}

fn meta(id: &str, name: &str, attribution: &str, extra: &str) -> String {
    format!(
        "id={id}\nname={}\nattribution={}\nlanguages={ANY_LANGUAGE}\nkind.{ANY_LANGUAGE}=definition\ndefault=on\n{extra}",
        one_line(name),
        one_line(attribution)
    )
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn read_maybe_gzipped(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let file = File::open(path)?;
    match path
        .extension()
        .is_some_and(|extension| extension == "dz" || extension == "gz")
    {
        true => GzDecoder::new(file).read_to_end(&mut bytes)?,
        false => BufReader::new(file).read_to_end(&mut bytes)?,
    };
    Ok(bytes)
}

fn first_existing(base: &Path, endings: &[&str]) -> Option<PathBuf> {
    endings
        .iter()
        .map(|ending| PathBuf::from(format!("{}{ending}", base.display())))
        .find(|path| path.is_file())
}

fn plain_text(markup: &str) -> Vec<String> {
    let mut text = String::new();
    let mut tag = String::new();
    let mut in_tag = false;
    for character in markup.chars() {
        match (in_tag, character) {
            (false, '<') => {
                in_tag = true;
                tag.clear();
            }
            (true, '>') => {
                in_tag = false;
                let name = tag
                    .trim_start_matches('/')
                    .split(|c: char| c.is_whitespace() || c == '/')
                    .next()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if matches!(
                    name.as_str(),
                    "br" | "p" | "div" | "li" | "tr" | "def" | "ex" | "blockquote"
                ) {
                    text.push('\n');
                }
            }
            (true, other) => tag.push(other),
            (false, other) => text.push(other),
        }
    }
    let text = text
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&");
    text.lines()
        .map(one_line)
        .filter(|line| !line.is_empty())
        .collect()
}

fn definition_lines(text: &str, word: &str) -> Vec<Line> {
    plain_text(text)
        .into_iter()
        .enumerate()
        .filter(|(index, line)| *index > 0 || !line.eq_ignore_ascii_case(word))
        .map(|(_, line)| Line {
            kind: 'D',
            text: line,
        })
        .collect()
}

fn stardict_fields(types: Option<&str>, data: &[u8]) -> Result<String, Failure> {
    let mut texts = Vec::new();
    let mut rest = data;
    let mut take = |kind: u8, last: bool, rest: &mut &[u8]| -> Result<(), Failure> {
        if kind.is_ascii_uppercase() {
            let size = match last {
                true => rest.len(),
                false => {
                    let size = rest
                        .get(..4)
                        .ok_or_else(|| broken("a field is cut short"))?;
                    *rest = &rest[4..];
                    u32::from_be_bytes([size[0], size[1], size[2], size[3]]) as usize
                }
            };
            *rest = rest.get(size..).unwrap_or_default();
            return Ok(());
        }
        let end = match last {
            true => rest.len(),
            false => rest
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(rest.len()),
        };
        if matches!(
            kind,
            b'm' | b'l' | b'g' | b'h' | b'x' | b't' | b'y' | b'k' | b'w'
        ) {
            texts.push(String::from_utf8_lossy(&rest[..end]).into_owned());
        }
        *rest = rest.get(end + 1..).unwrap_or_default();
        Ok(())
    };
    match types {
        Some(types) => {
            let kinds = types.as_bytes();
            for (index, kind) in kinds.iter().enumerate() {
                take(*kind, index + 1 == kinds.len(), &mut rest)?;
            }
        }
        None => {
            while let Some((kind, after)) = rest.split_first() {
                rest = after;
                take(*kind, false, &mut rest)?;
            }
        }
    }
    Ok(texts.join("\n"))
}

fn import_stardict(ifo: &Path) -> Result<(String, String, Vec<NewEntry>), Failure> {
    let info = fs::read_to_string(ifo)?;
    let mut lines = info.lines();
    if !lines.next().is_some_and(|first| {
        first
            .trim_start_matches('\u{feff}')
            .starts_with("StarDict's dict ifo file")
    }) {
        return Err(Failure::Unsupported);
    }
    let field = |key: &str| {
        info.lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
            .map(str::trim)
            .map(str::to_owned)
    };
    let name = field("bookname").ok_or_else(|| broken("no book name"))?;
    let wide = field("idxoffsetbits").as_deref() == Some("64");
    let types = field("sametypesequence");
    let attribution = field("copyright")
        .or_else(|| field("author"))
        .unwrap_or_else(|| name.clone());
    let base = ifo.with_extension("");
    let index_path =
        first_existing(&base, &[".idx", ".idx.gz"]).ok_or_else(|| broken("no .idx file"))?;
    let data_path =
        first_existing(&base, &[".dict.dz", ".dict"]).ok_or_else(|| broken("no .dict file"))?;
    let index = read_maybe_gzipped(&index_path)?;
    let data = read_maybe_gzipped(&data_path)?;
    let offset_size = if wide { 8 } else { 4 };
    let mut entries = Vec::new();
    let mut at = 0;
    while at < index.len() {
        let end = index[at..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|length| at + length)
            .ok_or_else(|| broken("the index is cut short"))?;
        let word = String::from_utf8_lossy(&index[at..end]).into_owned();
        let numbers = index
            .get(end + 1..end + 1 + offset_size + 4)
            .ok_or_else(|| broken("the index is cut short"))?;
        let (start, size) = match wide {
            true => (
                u64::from_be_bytes(numbers[..8].try_into().unwrap_or_default()) as usize,
                u32::from_be_bytes(numbers[8..12].try_into().unwrap_or_default()) as usize,
            ),
            false => (
                u32::from_be_bytes(numbers[..4].try_into().unwrap_or_default()) as usize,
                u32::from_be_bytes(numbers[4..8].try_into().unwrap_or_default()) as usize,
            ),
        };
        at = end + 1 + offset_size + 4;
        let article = data
            .get(start..start + size)
            .ok_or_else(|| broken("an entry points outside the data"))?;
        let text = stardict_fields(types.as_deref(), article)?;
        entries.push(NewEntry {
            keys: vec![format!("{ANY_LANGUAGE}:{}", word.to_lowercase())],
            lines: definition_lines(&text, &word),
            word,
            pos: String::new(),
        });
    }
    if let Some(synonyms) = first_existing(&base, &[".syn", ".syn.gz"]) {
        let synonyms = read_maybe_gzipped(&synonyms)?;
        let mut at = 0;
        while let Some(length) = synonyms[at..].iter().position(|byte| *byte == 0) {
            let end = at + length;
            let Some(number) = synonyms.get(end + 1..end + 5) else {
                break;
            };
            let index = u32::from_be_bytes([number[0], number[1], number[2], number[3]]) as usize;
            let word = String::from_utf8_lossy(&synonyms[at..end]).to_lowercase();
            if let Some(entry) = entries.get_mut(index) {
                entry.keys.push(format!("{ANY_LANGUAGE}:{word}"));
            }
            at = end + 5;
        }
    }
    Ok((name, attribution, entries))
}

fn base64_number(text: &str) -> Option<usize> {
    text.bytes().try_fold(0usize, |number, byte| {
        let digit = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        number.checked_mul(64)?.checked_add(usize::from(digit))
    })
}

fn header_value(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("00"))
        .map(str::to_owned)
}

fn import_dictd(index_path: &Path) -> Result<(String, String, Vec<NewEntry>), Failure> {
    let base = index_path.with_extension("");
    let data_path =
        first_existing(&base, &[".dict.dz", ".dict"]).ok_or_else(|| broken("no .dict file"))?;
    let data = read_maybe_gzipped(&data_path)?;
    let index = fs::read_to_string(index_path).map_err(|_| broken("the index is not text"))?;
    let mut name = None;
    let mut attribution = None;
    let mut entries = Vec::new();
    for line in index.lines() {
        let mut parts = line.split('\t');
        let (Some(word), Some(start), Some(size)) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(Failure::Unsupported);
        };
        let start = base64_number(start).ok_or_else(|| broken("a bad offset in the index"))?;
        let size = base64_number(size).ok_or_else(|| broken("a bad size in the index"))?;
        let text = data
            .get(start..start.saturating_add(size))
            .map(|text| String::from_utf8_lossy(text).into_owned())
            .ok_or_else(|| broken("an entry points outside the data"))?;
        match word {
            "00-database-short" | "00databaseshort" => {
                name = header_value(&text);
            }
            "00-database-info" | "00databaseinfo" => {
                attribution = header_value(&text);
            }
            other if other.starts_with("00-database") || other.starts_with("00database") => {}
            other => entries.push(NewEntry {
                keys: vec![format!("{ANY_LANGUAGE}:{}", other.to_lowercase())],
                lines: text
                    .lines()
                    .map(one_line)
                    .filter(|line| !line.is_empty())
                    .enumerate()
                    .filter(|(index, line)| *index > 0 || !line.eq_ignore_ascii_case(other))
                    .map(|(_, text)| Line { kind: 'D', text })
                    .collect(),
                word: other.to_owned(),
                pos: String::new(),
            }),
        }
    }
    let name = name
        .or_else(|| Some(base.file_name()?.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let attribution = attribution.unwrap_or_else(|| name.clone());
    Ok((name, attribution, entries))
}

fn unpacked(path: &Path) -> Result<PathBuf, Failure> {
    let folder = bande_xdg::cache_home()
        .ok_or_else(|| broken("no cache folder"))?
        .join(format!("bande/dictionary-import-{}", std::process::id()));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder)?;
    let status = Command::new("tar")
        .arg("-xf")
        .arg(path)
        .arg("-C")
        .arg(&folder)
        .arg("--no-same-owner")
        .arg("--no-same-permissions")
        .status()?;
    if !status.success() {
        return Err(broken("the archive cannot be unpacked"));
    }
    Ok(folder)
}

fn find_with(folder: &Path, wanted: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    for entry in fs::read_dir(folder).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_with(&path, wanted) {
                return Some(found);
            }
        } else if wanted(&path) {
            return Some(path);
        }
    }
    None
}

fn ends_with(path: &Path, ending: &str) -> bool {
    path.to_string_lossy().to_lowercase().ends_with(ending)
}

fn convert(path: &Path) -> Result<(String, String, Vec<NewEntry>), Failure> {
    if ends_with(path, ".ifo") {
        return import_stardict(path);
    }
    if ends_with(path, ".index") {
        return import_dictd(path);
    }
    if ends_with(path, ".dictionary")
        || find_with(path, &|found| ends_with(found, "body.data")).is_some()
    {
        return Err(Failure::Unsupported);
    }
    if TARBALLS.iter().any(|ending| ends_with(path, ending)) {
        let folder = unpacked(path)?;
        let found = find_with(&folder, &|found| ends_with(found, ".ifo"))
            .map(|ifo| import_stardict(&ifo))
            .or_else(|| {
                find_with(&folder, &|found| ends_with(found, ".index"))
                    .map(|index| import_dictd(&index))
            })
            .unwrap_or(Err(Failure::Unsupported));
        let _ = fs::remove_dir_all(&folder);
        return found;
    }
    Err(Failure::Unsupported)
}

pub fn import_file(path: &Path) -> Result<String, Failure> {
    let (name, attribution, entries) = convert(path)?;
    if entries.is_empty() {
        return Err(broken("it has no entries"));
    }
    let id = identifier(&name);
    let target = users_folder()?.join(format!("{id}.bdict"));
    bande_dict::write_pack(&target, &meta(&id, &name, &attribution, ""), &entries)?;
    Ok(name)
}

fn dict_command(stream: &mut BufReader<TcpStream>, command: &str) -> io::Result<String> {
    stream
        .get_mut()
        .write_all(format!("{command}\r\n").as_bytes())?;
    let mut line = String::new();
    stream.read_line(&mut line)?;
    Ok(line)
}

pub fn connect_dict(host: &str, port: u16) -> io::Result<BufReader<TcpStream>> {
    let address = (host, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::other("no address"))?;
    let stream = TcpStream::connect_timeout(&address, DICT_TIMEOUT)?;
    stream.set_read_timeout(Some(DICT_TIMEOUT))?;
    stream.set_write_timeout(Some(DICT_TIMEOUT))?;
    let mut stream = BufReader::new(stream);
    let mut banner = String::new();
    stream.read_line(&mut banner)?;
    match banner.starts_with("220") {
        true => Ok(stream),
        false => Err(io::Error::other("not a DICT server")),
    }
}

pub fn parse_dict_url(url: &str) -> Option<(String, u16, String)> {
    let rest = url.trim().strip_prefix("dict://")?;
    let (authority, database) = rest.split_once('/').unwrap_or((rest, "*"));
    let database = match database.trim_matches('/') {
        "" => "*",
        database => database,
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, port.parse().ok()?),
        None => (authority, DICT_PORT),
    };
    let valid = |text: &str| {
        !text.is_empty()
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '*' | '!'))
    };
    (valid(host) && valid(database)).then(|| (host.to_owned(), port, database.to_owned()))
}

pub fn add_server(url: &str) -> Result<String, Failure> {
    let (host, port, database) = parse_dict_url(url).ok_or(Failure::Unsupported)?;
    let mut stream = connect_dict(&host, port)?;
    let answer = dict_command(&mut stream, "QUIT")?;
    if !answer.starts_with("221") {
        return Err(broken("the server did not answer as a DICT server"));
    }
    let name = match database.as_str() {
        "*" | "!" => host.clone(),
        database => format!("{host} ({database})"),
    };
    let id = identifier(&format!("dict-{host}-{port}-{database}"));
    let address = format!("dict://{host}:{port}/{database}");
    let text = meta(
        &id,
        &name,
        &address,
        &format!("protocol=dict\nurl={address}\n"),
    );
    bande_xdg::replace_file(&users_folder()?.join(format!("{id}.source")), text)?;
    Ok(name)
}
#[cfg(test)]
mod tests {
    use super::*;

    fn stardict(folder: &Path) -> io::Result<PathBuf> {
        let article = b"<b>blorp</b><br>a made-up &amp; tested thing";
        let mut index = b"blorp\0".to_vec();
        index.extend(0u32.to_be_bytes());
        index.extend((article.len() as u32).to_be_bytes());
        fs::write(folder.join("test.idx"), &index)?;
        fs::write(folder.join("test.dict"), article)?;
        let ifo = folder.join("test.ifo");
        fs::write(
            &ifo,
            "StarDict's dict ifo file\nversion=2.4.2\nwordcount=1\nbookname=Test words\nsametypesequence=h\n",
        )?;
        Ok(ifo)
    }

    #[test]
    fn imported_stardict_and_dictd_dictionaries_are_looked_up() -> io::Result<()> {
        let folder = tempfile::tempdir()?;
        let dictd = folder.path().join("words.index");
        fs::write(
            folder.path().join("words.dict"),
            "00-database-short\n    Old words\nsnig\n   an old word\n",
        )?;
        fs::write(&dictd, "00-database-short\tA\tg\nsnig\tg\tU\n")?;
        let mut found = Vec::new();
        for (path, word) in [(stardict(folder.path())?, "Blorp"), (dictd, "snig")] {
            let (name, attribution, entries) =
                convert(&path).map_err(|failure| io::Error::other(format!("{failure:?}")))?;
            let pack = folder.path().join(format!("{}.bdict", identifier(&name)));
            bande_dict::write_pack(
                &pack,
                &meta(&identifier(&name), &name, &attribution, ""),
                &entries,
            )?;
            let source = bande_dict::Pack::open(&pack)?;
            let hits = bande_dict::look_up(&[source], word, &[])?;
            found.push((name, hits[0].entries[0].lines[0].text.clone()));
        }
        assert_eq!(
            found,
            [
                (
                    "Test words".to_owned(),
                    "a made-up & tested thing".to_owned()
                ),
                ("Old words".to_owned(), "an old word".to_owned()),
            ]
        );
        assert!(matches!(
            convert(&folder.path().join("test.dict")),
            Err(Failure::Unsupported)
        ));
        Ok(())
    }
}
