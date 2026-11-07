use std::fs;
use std::io;
use std::path::PathBuf;

use crate::file::replace_file;
use crate::mimeapps::config_home;

fn path() -> Option<PathBuf> {
    config_home().map(|home| home.join("bande/app-languages"))
}

fn parsed(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let (app, code) = line.split_once('=')?;
            let (app, code) = (app.trim(), code.trim());
            (!app.is_empty() && !code.is_empty()).then(|| (app.to_owned(), code.to_owned()))
        })
        .collect()
}

pub fn app_languages() -> Vec<(String, String)> {
    path()
        .and_then(|path| fs::read_to_string(path).ok())
        .map(|text| parsed(&text))
        .unwrap_or_default()
}

pub fn app_language(app_id: &str) -> Option<String> {
    app_languages()
        .into_iter()
        .find_map(|(app, code)| (app == app_id).then_some(code))
}

pub fn set_app_language(app_id: &str, code: Option<&str>) -> io::Result<()> {
    let path = path().ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    let mut languages = app_languages();
    languages.retain(|(app, _)| app != app_id);
    if let Some(code) = code {
        languages.push((app_id.to_owned(), code.to_owned()));
    }
    let text: String = languages
        .iter()
        .map(|(app, code)| format!("{app}={code}\n"))
        .collect();
    replace_file(&path, text)
}
