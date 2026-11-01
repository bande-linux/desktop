use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bande_config::{Search, SearchCategory};
use bande_search::{FileHit, FileResults, Query, Usage, calculate, fold, recent_files};
use bande_xdg::path_from_file_uri;

use crate::emoji::EMOJI;

pub const GRID_COLUMNS: usize = 8;
pub const EMOJI_COLUMNS: usize = 12;
const FREQUENT_EMOJI_ROWS: usize = 2;
const MAX_EMOJI: usize = 240;
const EMOJI_KEY: &str = "emoji:";
pub const MAX_APPS: usize = 4;
pub const MAX_SETTINGS: usize = 3;
pub const MAX_FOLDERS: usize = 3;
pub const MAX_DOCUMENTS: usize = 5;
pub const MAX_COMMANDS: usize = 2;
const MIN_COMMAND_QUERY: usize = 2;
const DESCRIBED_PENALTY: u32 = 150;
const COMMENT_PENALTY: u32 = 300;
const SETTING_LABEL_PENALTY: u32 = 100;
const SETTING_KEYWORD_PENALTY: u32 = 200;
const CALCULATION_SCORE: u32 = u32::MAX;
const APPLICATION_BONUS: u32 = 25;
const FILE_PENALTY: u32 = 200;

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
            .filter(|score| *score >= bande_search::WORDS)
            .max()
            .map(|score| score - DESCRIBED_PENALTY);
        let commented = self
            .comment
            .as_ref()
            .and_then(|comment| query.score(comment))
            .filter(|score| *score >= bande_search::WORDS)
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
        .filter(|score| *score >= bande_search::WORDS)
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
                    .filter(|score| *score >= bande_search::WORDS)
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
pub enum Mode {
    Search,
    Emoji,
    Clipboard,
}

const URI_LIST: &str = "text/uri-list";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clip {
    pub id: u64,
    pub copied: u64,
    pub text: String,
    pub mimes: Vec<String>,
}

impl Clip {
    pub fn kind(&self) -> Kind {
        if self.mimes.iter().any(|mime| mime == URI_LIST) {
            Kind::CopiedFiles
        } else if self.text.is_empty() && self.picture().is_some() {
            Kind::CopiedImage
        } else {
            Kind::CopiedText
        }
    }

    pub fn picture(&self) -> Option<&str> {
        self.mimes
            .iter()
            .find(|mime| *mime == "image/png")
            .or_else(|| self.mimes.iter().find(|mime| mime.starts_with("image/")))
            .map(String::as_str)
    }

    pub fn paths(&self) -> Vec<PathBuf> {
        self.text
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| path_from_file_uri(line.trim()))
            .collect()
    }

    fn title(&self) -> String {
        match self.kind() {
            Kind::CopiedFiles => self
                .paths()
                .iter()
                .filter_map(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(", "),
            Kind::CopiedImage => crate::L10N.tr("copied-image"),
            _ => self
                .text
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or_default()
                .to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemCommand {
    LockScreen,
    Sleep,
    LogOut,
    Restart,
    ShutDown,
}

impl SystemCommand {
    const ALL: [Self; 5] = [
        Self::LockScreen,
        Self::Sleep,
        Self::LogOut,
        Self::Restart,
        Self::ShutDown,
    ];

    fn key(self) -> &'static str {
        match self {
            Self::LockScreen => "lock-screen",
            Self::Sleep => "sleep",
            Self::LogOut => "log-out",
            Self::Restart => "restart",
            Self::ShutDown => "shut-down",
        }
    }

    pub fn title(self) -> String {
        crate::L10N.tr(match self {
            Self::LockScreen => "command-lock-screen",
            Self::Sleep => "command-sleep",
            Self::LogOut => "command-log-out",
            Self::Restart => "command-restart",
            Self::ShutDown => "command-shut-down",
        })
    }

    fn keywords(self) -> String {
        crate::L10N.tr(match self {
            Self::LockScreen => "command-lock-screen-keywords",
            Self::Sleep => "command-sleep-keywords",
            Self::LogOut => "command-log-out-keywords",
            Self::Restart => "command-restart-keywords",
            Self::ShutDown => "command-shut-down-keywords",
        })
    }
}

struct IndexedCommand {
    command: SystemCommand,
    title: String,
    name: String,
    keywords: Vec<String>,
}

impl IndexedCommand {
    fn new(command: SystemCommand) -> Self {
        let title = command.title();
        Self {
            name: fold(title.trim_end_matches('…')),
            keywords: command.keywords().split_whitespace().map(fold).collect(),
            title,
            command,
        }
    }

    fn score(&self, query: &Query) -> Option<u32> {
        let named = query
            .score(&self.name)
            .filter(|score| *score >= bande_search::WORDS);
        let described = word_match(query, &self.keywords).map(|score| score - DESCRIBED_PENALTY);
        named.max(described)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    CopiedText,
    CopiedImage,
    CopiedFiles,
    Emoji,
    Calculation,
    Application,
    Setting,
    Command,
    Folder,
    Document,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    App(String),
    Settings(String),
    Path(PathBuf),
    Value(String),
    Emoji(String),
    Clip(u64),
    Command(SystemCommand),
}

impl Target {
    pub fn usage_key(&self) -> Option<String> {
        match self {
            Self::App(app_id) => Some(format!("app:{app_id}")),
            Self::Settings(keyword) => Some(format!("settings:{keyword}")),
            Self::Path(path) => path.to_str().map(|path| format!("file:{path}")),
            Self::Emoji(character) => Some(format!("{EMOJI_KEY}{character}")),
            Self::Command(command) => Some(format!("command:{}", command.key())),
            Self::Value(_) | Self::Clip(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub kind: Kind,
    pub title: String,
    pub detail: Option<String>,
    pub target: Target,
    pub matched: Vec<usize>,
    score: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heading {
    TopHit,
    Applications,
    Settings,
    System,
    Folders,
    Documents,
    RecentApps,
    RecentDocuments,
    AllApps,
    FrequentlyUsed,
    EmojiGroup(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Open(Target),
    Reveal(PathBuf),
    Copy(String),
    Insert(String),
    SkinTone(Option<usize>),
    PasteClip(u64),
    RemoveClip(u64),
    ClearClips,
    Close,
}

pub struct Model {
    apps: Vec<IndexedApp>,
    panes: Vec<IndexedPane>,
    commands: Vec<IndexedCommand>,
    home: PathBuf,
    usage: Usage,
    now: u64,
    pub query: String,
    pub generation: u64,
    files: Vec<FileHit>,
    recent_documents: Vec<PathBuf>,
    search: Search,
    excluded: Vec<PathBuf>,
    pub clips: Vec<Clip>,
    pub files_complete: bool,
    pub hits: Vec<Hit>,
    pub headings: Vec<(usize, Heading)>,
    pub columns: usize,
    pub mode: Mode,
    pub inserts: bool,
    pub skin_tone: Option<usize>,
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
        let mut model = Self {
            apps: apps.into_iter().map(IndexedApp::new).collect(),
            panes: panes.into_iter().map(IndexedPane::new).collect(),
            commands: SystemCommand::ALL
                .into_iter()
                .map(IndexedCommand::new)
                .collect(),
            home,
            usage,
            now,
            query: String::new(),
            generation: 0,
            files: Vec::new(),
            recent_documents: Vec::new(),
            search: Search::default(),
            excluded: Vec::new(),
            clips: Vec::new(),
            files_complete: false,
            hits: Vec::new(),
            headings: Vec::new(),
            columns: 0,
            mode: Mode::Search,
            inserts: false,
            skin_tone: None,
            selected: 0,
            copied: false,
            actions: Vec::new(),
            revision: 0,
        };
        model.rebuild(None);
        model
    }

    pub fn set_recently_used(&mut self, xbel: Option<&str>) {
        let documents = match self.shows(SearchCategory::RecentDocuments) {
            true => recent_files(xbel, &self.usage, &self.excluded, GRID_COLUMNS),
            false => Vec::new(),
        };
        if documents != self.recent_documents {
            self.recent_documents = documents;
            if Query::new(&self.query).is_none() {
                self.rebuild(None);
            }
        }
    }

    pub fn set_search(&mut self, search: Search, excluded: Vec<PathBuf>) {
        if search != self.search || excluded != self.excluded {
            self.search = search;
            self.excluded = excluded;
            self.files_complete |= !self.searches_files();
            self.rebuild(None);
        }
    }

    fn shows(&self, category: SearchCategory) -> bool {
        self.search.shows(category)
    }

    fn searches_files(&self) -> bool {
        self.shows(SearchCategory::Documents) || self.shows(SearchCategory::Folders)
    }

    fn is_excluded(&self, path: &Path) -> bool {
        self.excluded.iter().any(|folder| path.starts_with(folder))
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

    pub fn now(&self) -> u64 {
        self.now
    }

    pub fn set_now(&mut self, now: u64) {
        self.now = now;
    }

    pub fn set_mode(&mut self, mode: Mode, inserts: bool) {
        self.inserts = inserts;
        if mode != self.mode {
            self.mode = mode;
            self.generation += 1;
            self.rebuild(None);
        }
    }

    pub fn set_skin_tone(&mut self, tone: Option<usize>) {
        if tone != self.skin_tone {
            self.skin_tone = tone;
            self.actions.push(Action::SkinTone(tone));
        }
    }

    pub fn emoji_text(&self, character: &str) -> String {
        EMOJI.with_tone(character, self.skin_tone)
    }

    pub fn set_query(&mut self, query: &str) {
        if query == self.query {
            return;
        }
        self.query = query.to_owned();
        self.generation += 1;
        self.files_complete = Query::new(query).is_none() || !self.searches_files();
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
        let category = match file.folder {
            true => SearchCategory::Folders,
            false => SearchCategory::Documents,
        };
        if !self.shows(category) || self.is_excluded(&file.path) {
            return None;
        }
        let name = file.path.file_name()?.to_string_lossy().into_owned();
        let score = if self.files_complete {
            file.score
        } else {
            query.score(&fold(&name))?
        };
        Some(Hit {
            matched: query.matched(&fold(&name)),
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
            score: score.saturating_sub(FILE_PENALTY),
        })
    }

    fn rebuild(&mut self, keep: Option<Target>) {
        self.revision += 1;
        self.copied = false;
        self.hits.clear();
        self.headings.clear();
        if self.mode != Mode::Search {
            match self.mode {
                Mode::Emoji => self.rebuild_emoji(),
                _ => self.rebuild_clipboard(),
            }
            self.selected = keep
                .and_then(|target| self.hits.iter().position(|hit| hit.target == target))
                .unwrap_or(0);
            return;
        }
        let Some(query) = Query::new(&self.query) else {
            self.files.clear();
            self.browse();
            return;
        };
        self.columns = 0;
        let calculation = calculate(&self.query)
            .filter(|calculation| {
                self.shows(match calculation.conversion {
                    true => SearchCategory::Conversion,
                    false => SearchCategory::Calculator,
                })
            })
            .map(|calculation| Hit {
                kind: Kind::Calculation,
                title: calculation.value.clone(),
                detail: Some(self.query.trim().to_owned()),
                target: Target::Value(calculation.value),
                matched: Vec::new(),
                score: CALCULATION_SCORE,
            });
        let mut apps: Vec<Hit> = self
            .apps
            .iter()
            .filter(|_| self.shows(SearchCategory::Applications))
            .filter_map(|app| {
                let target = Target::App(app.info.app_id.clone());
                let boost = target
                    .usage_key()
                    .map_or(0, |key| self.usage.boost(&key, self.now));
                app.score(&query).map(|score| Hit {
                    matched: query.matched(&app.name),
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
            .filter(|_| self.shows(SearchCategory::Settings))
            .filter_map(|pane| {
                let (score, setting) = pane.score(&query)?;
                let target = Target::Settings(pane.info.keyword.clone());
                let boost = target
                    .usage_key()
                    .map_or(0, |key| self.usage.boost(&key, self.now));
                Some(Hit {
                    matched: query.matched(&pane.name),
                    kind: Kind::Setting,
                    title: pane.info.name.clone(),
                    detail: setting,
                    target,
                    score: score + boost,
                })
            })
            .collect();
        settings.sort_by(by_score);
        let typed_enough = self.query.trim().chars().count() >= MIN_COMMAND_QUERY
            && self.shows(SearchCategory::System);
        let mut commands: Vec<Hit> = self
            .commands
            .iter()
            .filter(|_| typed_enough)
            .filter_map(|command| {
                let target = Target::Command(command.command);
                let boost = target
                    .usage_key()
                    .map_or(0, |key| self.usage.boost(&key, self.now));
                Some(Hit {
                    matched: query.matched(&command.name),
                    kind: Kind::Command,
                    title: command.title.clone(),
                    detail: None,
                    target,
                    score: command.score(&query)? + boost,
                })
            })
            .collect();
        commands.sort_by(by_score);
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
            (Heading::System, commands, MAX_COMMANDS),
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

    fn app_hit(&self, app: &IndexedApp) -> Hit {
        Hit {
            kind: Kind::Application,
            title: app.info.name.clone(),
            detail: app
                .info
                .generic_name
                .clone()
                .or_else(|| app.info.comment.clone()),
            target: Target::App(app.info.app_id.clone()),
            matched: Vec::new(),
            score: 0,
        }
    }

    fn browse(&mut self) {
        self.columns = GRID_COLUMNS;
        let mut used: Vec<(PathBuf, u64)> = self.usage.last_used("app:").collect();
        used.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let recent_apps: Vec<Hit> = used
            .iter()
            .filter(|_| self.shows(SearchCategory::RecentApps))
            .filter_map(|(app_id, _)| {
                self.apps
                    .iter()
                    .find(|app| Path::new(&app.info.app_id) == app_id)
            })
            .take(GRID_COLUMNS)
            .map(|app| self.app_hit(app))
            .collect();
        let recent_documents: Vec<Hit> = self
            .recent_documents
            .iter()
            .filter(|_| self.shows(SearchCategory::RecentDocuments))
            .filter_map(|path| {
                Some(Hit {
                    kind: Kind::Document,
                    title: path.file_name()?.to_string_lossy().into_owned(),
                    detail: path.parent().map(|parent| display_path(parent, &self.home)),
                    target: Target::Path(path.clone()),
                    matched: Vec::new(),
                    score: 0,
                })
            })
            .collect();
        let mut all: Vec<&IndexedApp> = self.apps.iter().collect();
        all.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| a.info.name.cmp(&b.info.name))
        });
        let all_apps: Vec<Hit> = all.into_iter().map(|app| self.app_hit(app)).collect();
        for (heading, group) in [
            (Heading::RecentApps, recent_apps),
            (Heading::RecentDocuments, recent_documents),
            (Heading::AllApps, all_apps),
        ] {
            if !group.is_empty() {
                self.headings.push((self.hits.len(), heading));
                self.hits.extend(group);
            }
        }
        self.selected = 0;
    }

    fn emoji_hit(character: &str, name: &str, matched: Vec<usize>) -> Hit {
        let mut characters = name.chars();
        let title = characters
            .next()
            .map(|first| first.to_uppercase().chain(characters).collect())
            .unwrap_or_default();
        Hit {
            kind: Kind::Emoji,
            title,
            detail: None,
            target: Target::Emoji(character.to_owned()),
            matched,
            score: 0,
        }
    }

    pub fn set_clips(&mut self, clips: Vec<Clip>) {
        self.clips = clips;
        if self.mode == Mode::Clipboard {
            let keep = self.selection().map(|hit| hit.target.clone());
            self.rebuild(keep);
        }
    }

    pub fn clip(&self, id: u64) -> Option<&Clip> {
        self.clips.iter().find(|clip| clip.id == id)
    }

    fn rebuild_clipboard(&mut self) {
        self.columns = 0;
        self.files_complete = true;
        let query = Query::new(&self.query);
        self.hits = self
            .clips
            .iter()
            .filter_map(|clip| {
                let title = clip.title();
                let matched = match &query {
                    Some(query) => {
                        let folded = fold(&clip.text);
                        if !folded.contains(query.text()) && query.score(&fold(&title)).is_none() {
                            return None;
                        }
                        query.matched(&fold(&title))
                    }
                    None => Vec::new(),
                };
                Some(Hit {
                    kind: clip.kind(),
                    title,
                    detail: None,
                    target: Target::Clip(clip.id),
                    matched,
                    score: 0,
                })
            })
            .collect();
    }

    pub fn remove_clip(&mut self) {
        let Some(Target::Clip(id)) = self.selection().map(|hit| hit.target.clone()) else {
            return;
        };
        let selected = self.selected;
        self.clips.retain(|clip| clip.id != id);
        self.actions.push(Action::RemoveClip(id));
        self.rebuild(None);
        self.selected = selected.min(self.hits.len().saturating_sub(1));
    }

    pub fn clear_clips(&mut self) {
        self.clips.clear();
        self.actions.push(Action::ClearClips);
        self.rebuild(None);
    }

    fn rebuild_emoji(&mut self) {
        self.columns = EMOJI_COLUMNS;
        self.files_complete = true;
        if let Some(query) = Query::new(&self.query) {
            self.hits = EMOJI
                .search(&query)
                .into_iter()
                .take(MAX_EMOJI)
                .map(|(emoji, _, matched)| Self::emoji_hit(emoji.character, emoji.name, matched))
                .collect();
            return;
        }
        let mut used: Vec<(u32, u64, String)> = self
            .usage
            .last_used(EMOJI_KEY)
            .map(|(character, last)| {
                let character = character.to_string_lossy().into_owned();
                let key = format!("{EMOJI_KEY}{character}");
                (self.usage.boost(&key, self.now), last, character)
            })
            .collect();
        used.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        let frequent: Vec<Hit> = used
            .iter()
            .filter_map(|(_, _, character)| EMOJI.find(character))
            .take(FREQUENT_EMOJI_ROWS * EMOJI_COLUMNS)
            .map(|emoji| Self::emoji_hit(emoji.character, emoji.name, Vec::new()))
            .collect();
        if !frequent.is_empty() {
            self.headings.push((0, Heading::FrequentlyUsed));
            self.hits.extend(frequent);
        }
        for emoji in &EMOJI.emoji {
            if self.headings.last().map(|(_, heading)| *heading)
                != Some(Heading::EmojiGroup(emoji.group))
            {
                self.headings
                    .push((self.hits.len(), Heading::EmojiGroup(emoji.group)));
            }
            self.hits
                .push(Self::emoji_hit(emoji.character, emoji.name, Vec::new()));
        }
    }

    fn sections(&self) -> Vec<(usize, usize)> {
        let mut starts: Vec<usize> = self.headings.iter().map(|(start, _)| *start).collect();
        if starts.first() != Some(&0) {
            starts.insert(0, 0);
        }
        starts
            .iter()
            .enumerate()
            .map(|(index, start)| {
                (
                    *start,
                    starts.get(index + 1).copied().unwrap_or(self.hits.len()),
                )
            })
            .collect()
    }

    pub fn select_vertically(&mut self, down: bool) {
        if self.columns == 0 {
            if down {
                self.select_next();
            } else {
                self.select_previous();
            }
            return;
        }
        let columns = self.columns;
        let sections = self.sections();
        let Some(section) = sections
            .iter()
            .position(|(start, end)| (*start..*end).contains(&self.selected))
        else {
            return;
        };
        let (start, end) = sections[section];
        let offset = self.selected - start;
        let column = offset % columns;
        let row = offset / columns;
        let last_row = (end - start - 1) / columns;
        let target = if down && row < last_row {
            Some((start + (row + 1) * columns + column).min(end - 1))
        } else if !down && row > 0 {
            Some(start + (row - 1) * columns + column)
        } else if down {
            sections
                .get(section + 1)
                .map(|(next, next_end)| (next + column).min(next_end - 1))
        } else {
            section.checked_sub(1).map(|previous| {
                let (previous_start, previous_end) = sections[previous];
                let last = (previous_end - previous_start - 1) / columns * columns;
                (previous_start + last + column).min(previous_end - 1)
            })
        };
        if let Some(target) = target {
            self.select(target);
        }
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
        match hit.target.clone() {
            Target::Value(value) => {
                self.actions.push(Action::Copy(value));
                self.actions.push(Action::Close);
            }
            Target::Emoji(character) => {
                let text = self.emoji_text(&character);
                if let Err(error) = self.record_use(&Target::Emoji(character)) {
                    tracing::warn!("cannot remember the emoji: {error}");
                }
                if self.inserts {
                    self.actions.push(Action::Close);
                    self.actions.push(Action::Insert(text));
                } else {
                    self.actions.push(Action::Copy(text));
                    self.actions.push(Action::Close);
                }
            }
            Target::Clip(id) => {
                let text = self
                    .clip(id)
                    .filter(|clip| self.inserts && clip.kind() == Kind::CopiedText)
                    .map(|clip| clip.text.clone());
                self.actions.push(Action::PasteClip(id));
                self.actions.push(Action::Close);
                if let Some(text) = text {
                    self.actions.push(Action::Insert(text));
                }
            }
            target => self.actions.push(Action::Open(target)),
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
            Some(Target::Emoji(character)) => self.emoji_text(character),
            Some(Target::Clip(id)) => {
                self.actions.push(Action::PasteClip(*id));
                self.copied = true;
                return true;
            }
            Some(Target::App(_) | Target::Settings(_) | Target::Command(_)) | None => return false,
        };
        self.actions.push(Action::Copy(text));
        self.copied = true;
        true
    }
}
