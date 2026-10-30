use std::collections::HashMap;
use std::path::{Path, PathBuf};

use unbound_search::{FileHit, FileResults, Query, Usage, calculate, fold};

pub const MAX_APPS: usize = 4;
pub const MAX_SETTINGS: usize = 3;
pub const MAX_FOLDERS: usize = 3;
pub const MAX_DOCUMENTS: usize = 5;
const DESCRIBED_PENALTY: u32 = 150;
const COMMENT_PENALTY: u32 = 300;
const SETTING_LABEL_PENALTY: u32 = 100;
const SETTING_KEYWORD_PENALTY: u32 = 200;
const CALCULATION_SCORE: u32 = u32::MAX;
const APPLICATION_BONUS: u32 = 25;

#[derive(Debug, Clone)]
pub struct AppInfo {
    pub app_id: String,
    pub name: String,
    pub generic_name: Option<String>,
    pub comment: Option<String>,
    pub keywords: Vec<String>,
}

struct IndexedApp {
    info: AppInfo,
    name: String,
    described: Vec<String>,
    comment: Option<String>,
}

impl IndexedApp {
    fn new(info: AppInfo) -> Self {
        Self {
            name: fold(&info.name),
            described: info
                .generic_name
                .iter()
                .chain(&info.keywords)
                .map(|text| fold(text))
                .collect(),
            comment: info.comment.as_deref().map(fold),
            info,
        }
    }

    fn score(&self, query: &Query) -> Option<u32> {
        let named = query.score(&self.name);
        let described = self
            .described
            .iter()
            .filter_map(|text| query.score(text))
            .filter(|score| *score >= unbound_search::WORDS)
            .max()
            .map(|score| score - DESCRIBED_PENALTY);
        let commented = self
            .comment
            .as_ref()
            .and_then(|comment| query.score(comment))
            .filter(|score| *score >= unbound_search::WORDS)
            .map(|score| score - COMMENT_PENALTY);
        named.max(described).max(commented)
    }
}

#[derive(Debug, Clone)]
pub struct SettingInfo {
    pub label: String,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PaneInfo {
    pub keyword: String,
    pub name: String,
    pub keywords: Vec<String>,
    pub settings: Vec<SettingInfo>,
}

struct IndexedPane {
    info: PaneInfo,
    name: String,
    keywords: Vec<String>,
    settings: Vec<(String, Vec<String>)>,
}

fn folded_all(texts: &[String]) -> Vec<String> {
    texts.iter().map(|text| fold(text)).collect()
}

fn word_match(query: &Query, texts: &[String]) -> Option<u32> {
    texts
        .iter()
        .filter_map(|text| query.score(text))
        .filter(|score| *score >= unbound_search::WORDS)
        .max()
}

impl IndexedPane {
    fn new(info: PaneInfo) -> Self {
        Self {
            name: fold(&info.name),
            keywords: folded_all(&info.keywords),
            settings: info
                .settings
                .iter()
                .map(|setting| (fold(&setting.label), folded_all(&setting.keywords)))
                .collect(),
            info,
        }
    }

    fn score(&self, query: &Query) -> Option<(u32, Option<String>)> {
        let named = query.score(&self.name).map(|score| (score, None));
        let described =
            word_match(query, &self.keywords).map(|score| (score - DESCRIBED_PENALTY, None));
        let settings = self
            .settings
            .iter()
            .zip(&self.info.settings)
            .filter_map(|((label, keywords), setting)| {
                let labelled = query
                    .score(label)
                    .filter(|score| *score >= unbound_search::WORDS)
                    .map(|score| score - SETTING_LABEL_PENALTY);
                let keyworded =
                    word_match(query, keywords).map(|score| score - SETTING_KEYWORD_PENALTY);
                labelled
                    .max(keyworded)
                    .map(|score| (score, Some(setting.label.clone())))
            })
            .max_by_key(|(score, _)| *score);
        [named, described, settings]
            .into_iter()
            .flatten()
            .max_by_key(|(score, _)| *score)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Calculation,
    Application,
    Setting,
    Folder,
    Document,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    App(String),
    Settings(String),
    Path(PathBuf),
    Value(String),
}

impl Target {
    pub fn usage_key(&self) -> Option<String> {
        match self {
            Self::App(app_id) => Some(format!("app:{app_id}")),
            Self::Settings(keyword) => Some(format!("settings:{keyword}")),
            Self::Path(path) => path.to_str().map(|path| format!("file:{path}")),
            Self::Value(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub kind: Kind,
    pub title: String,
    pub detail: Option<String>,
    pub target: Target,
    score: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heading {
    TopHit,
    Applications,
    Settings,
    Folders,
    Documents,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Open(Target),
    Reveal(PathBuf),
    Copy(String),
    Close,
}

pub struct Model {
    apps: Vec<IndexedApp>,
    panes: Vec<IndexedPane>,
    home: PathBuf,
    usage: Usage,
    now: u64,
    pub query: String,
    pub generation: u64,
    files: Vec<FileHit>,
    pub files_complete: bool,
    pub hits: Vec<Hit>,
    pub headings: Vec<(usize, Heading)>,
    pub selected: usize,
    pub copied: bool,
    pub actions: Vec<Action>,
    pub revision: u64,
}

fn display_path(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(relative) if relative.as_os_str().is_empty() => "~".to_owned(),
        Ok(relative) => format!("~/{}", relative.display()),
        Err(_) => path.display().to_string(),
    }
}

impl Model {
    pub fn new(
        apps: Vec<AppInfo>,
        panes: Vec<PaneInfo>,
        home: PathBuf,
        usage: Usage,
        now: u64,
    ) -> Self {
        Self {
            apps: apps.into_iter().map(IndexedApp::new).collect(),
            panes: panes.into_iter().map(IndexedPane::new).collect(),
            home,
            usage,
            now,
            query: String::new(),
            generation: 0,
            files: Vec::new(),
            files_complete: false,
            hits: Vec::new(),
            headings: Vec::new(),
            selected: 0,
            copied: false,
            actions: Vec::new(),
            revision: 0,
        }
    }

    pub fn file_boosts(&self) -> HashMap<PathBuf, u32> {
        self.usage
            .keys()
            .filter_map(|key| {
                let path = key.strip_prefix("file:")?;
                Some((PathBuf::from(path), self.usage.boost(key, self.now)))
            })
            .collect()
    }

    pub fn set_now(&mut self, now: u64) {
        self.now = now;
    }

    pub fn set_query(&mut self, query: &str) {
        if query == self.query {
            return;
        }
        self.query = query.to_owned();
        self.generation += 1;
        self.files_complete = Query::new(query).is_none();
        self.selected = 0;
        self.rebuild(None);
    }

    pub fn set_files(&mut self, results: FileResults) {
        if results.generation != self.generation {
            return;
        }
        let keep = (self.selected > 0)
            .then(|| self.selection().map(|hit| hit.target.clone()))
            .flatten();
        self.files = results.hits;
        self.files_complete = true;
        self.rebuild(keep);
    }

    pub fn record_use(&mut self, target: &Target) -> std::io::Result<()> {
        match target.usage_key() {
            Some(key) => self.usage.record(&key, self.now),
            None => Ok(()),
        }
    }

    fn file_hit(&self, file: &FileHit, query: &Query) -> Option<Hit> {
        let name = file.path.file_name()?.to_string_lossy().into_owned();
        let score = if self.files_complete {
            file.score
        } else {
            query.score(&fold(&name))?
        };
        Some(Hit {
            kind: if file.folder {
                Kind::Folder
            } else {
                Kind::Document
            },
            detail: file
                .path
                .parent()
                .map(|parent| display_path(parent, &self.home)),
            title: name,
            target: Target::Path(file.path.clone()),
            score,
        })
    }

    fn rebuild(&mut self, keep: Option<Target>) {
        self.revision += 1;
        self.copied = false;
        self.hits.clear();
        self.headings.clear();
        let Some(query) = Query::new(&self.query) else {
            self.files.clear();
            return;
        };
        let calculation = calculate(&self.query).map(|value| Hit {
            kind: Kind::Calculation,
            title: value.clone(),
            detail: Some(self.query.trim().to_owned()),
            target: Target::Value(value),
            score: CALCULATION_SCORE,
        });
        let mut apps: Vec<Hit> = self
            .apps
            .iter()
            .filter_map(|app| {
                let target = Target::App(app.info.app_id.clone());
                let boost = target
                    .usage_key()
                    .map_or(0, |key| self.usage.boost(&key, self.now));
                app.score(&query).map(|score| Hit {
                    kind: Kind::Application,
                    title: app.info.name.clone(),
                    detail: app
                        .info
                        .generic_name
                        .clone()
                        .or_else(|| app.info.comment.clone()),
                    target,
                    score: score + boost + APPLICATION_BONUS,
                })
            })
            .collect();
        let by_score = |a: &Hit, b: &Hit| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.title.len().cmp(&b.title.len()))
                .then_with(|| a.title.cmp(&b.title))
        };
        apps.sort_by(by_score);
        let mut settings: Vec<Hit> = self
            .panes
            .iter()
            .filter_map(|pane| {
                let (score, setting) = pane.score(&query)?;
                let target = Target::Settings(pane.info.keyword.clone());
                let boost = target
                    .usage_key()
                    .map_or(0, |key| self.usage.boost(&key, self.now));
                Some(Hit {
                    kind: Kind::Setting,
                    title: pane.info.name.clone(),
                    detail: setting,
                    target,
                    score: score + boost,
                })
            })
            .collect();
        settings.sort_by(by_score);
        let files: Vec<Hit> = self
            .files
            .iter()
            .filter_map(|file| self.file_hit(file, &query))
            .collect();
        let (mut folders, mut documents): (Vec<Hit>, Vec<Hit>) =
            files.into_iter().partition(|hit| hit.kind == Kind::Folder);
        folders.sort_by(by_score);
        documents.sort_by(by_score);

        let top = match calculation {
            Some(calculation) => Some(calculation),
            None => [&mut apps, &mut settings, &mut folders, &mut documents]
                .into_iter()
                .filter(|group| !group.is_empty())
                .min_by(|a, b| by_score(&a[0], &b[0]))
                .map(|group| group.remove(0)),
        };
        if let Some(top) = top {
            self.headings.push((0, Heading::TopHit));
            self.hits.push(top);
        }
        for (heading, group, limit) in [
            (Heading::Applications, apps, MAX_APPS),
            (Heading::Settings, settings, MAX_SETTINGS),
            (Heading::Folders, folders, MAX_FOLDERS),
            (Heading::Documents, documents, MAX_DOCUMENTS),
        ] {
            if group.is_empty() {
                continue;
            }
            self.headings.push((self.hits.len(), heading));
            self.hits.extend(group.into_iter().take(limit));
        }
        self.selected = keep
            .and_then(|target| self.hits.iter().position(|hit| hit.target == target))
            .unwrap_or(0);
    }

    pub fn selection(&self) -> Option<&Hit> {
        self.hits.get(self.selected)
    }

    pub fn select(&mut self, index: usize) {
        if index < self.hits.len() && index != self.selected {
            self.selected = index;
            self.copied = false;
        }
    }

    pub fn select_next(&mut self) {
        self.select((self.selected + 1).min(self.hits.len().saturating_sub(1)));
    }

    pub fn select_previous(&mut self) {
        self.select(self.selected.saturating_sub(1));
    }

    pub fn select_next_group(&mut self, forward: bool) {
        let mut starts = self.headings.iter().map(|(start, _)| *start);
        let target = if forward {
            starts.find(|start| *start > self.selected)
        } else {
            starts.rev().find(|start| *start < self.selected)
        };
        if let Some(target) = target {
            self.select(target);
        }
    }

    pub fn activate(&mut self) {
        let Some(hit) = self.selection() else {
            return;
        };
        let action = match &hit.target {
            Target::Value(value) => Action::Copy(value.clone()),
            target => Action::Open(target.clone()),
        };
        let closes = matches!(action, Action::Copy(_));
        self.actions.push(action);
        if closes {
            self.actions.push(Action::Close);
        }
    }

    pub fn reveal(&mut self) {
        if let Some(Hit {
            target: Target::Path(path),
            ..
        }) = self.selection()
        {
            self.actions.push(Action::Reveal(path.clone()));
        }
    }

    pub fn copy(&mut self) -> bool {
        let text = match self.selection().map(|hit| &hit.target) {
            Some(Target::Value(value)) => value.clone(),
            Some(Target::Path(path)) => path.display().to_string(),
            Some(Target::App(_) | Target::Settings(_)) | None => return false,
        };
        self.actions.push(Action::Copy(text));
        self.copied = true;
        true
    }
}
