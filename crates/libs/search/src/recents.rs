use std::collections::HashMap;
use std::path::{Path, PathBuf};

use unbound_xdg::path_from_file_uri;

use crate::usage::Usage;

pub const FILE_KEY: &str = "file:";

pub fn recently_used_path(data_home: Option<&Path>, home: &Path) -> PathBuf {
    data_home
        .filter(|path| path.is_absolute())
        .map_or_else(|| home.join(".local/share"), Path::to_path_buf)
        .join("recently-used.xbel")
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let start = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let length = tag[start..].find('"')?;
    Some(&tag[start..start + length])
}

fn unescape(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn seconds(stamp: &str) -> Option<u64> {
    let stamp = stamp.parse::<jiff::Timestamp>().ok()?;
    u64::try_from(stamp.as_second()).ok()
}

pub fn recently_used(xbel: &str) -> Vec<(PathBuf, u64)> {
    xbel.split("<bookmark ")
        .skip(1)
        .filter_map(|rest| {
            let tag = format!(" {}", &rest[..rest.find('>')?]);
            let tag = tag.as_str();
            let path = path_from_file_uri(&unescape(attribute(tag, "href")?))?;
            let used = ["visited", "modified", "added"]
                .iter()
                .filter_map(|name| attribute(tag, name).and_then(seconds))
                .max()?;
            Some((path, used))
        })
        .collect()
}

pub fn recent_files(xbel: Option<&str>, usage: &Usage, limit: usize) -> Vec<PathBuf> {
    let mut latest: HashMap<PathBuf, u64> = HashMap::new();
    let used = usage
        .last_used(FILE_KEY)
        .chain(xbel.map(recently_used).unwrap_or_default());
    for (path, at) in used {
        let known = latest.entry(path).or_insert(at);
        *known = (*known).max(at);
    }
    let mut ordered = latest.into_iter().collect::<Vec<_>>();
    ordered.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ordered
        .into_iter()
        .map(|(path, _)| path)
        .filter(|path| path.is_file())
        .take(limit)
        .collect()
}
