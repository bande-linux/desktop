use std::path::{Path, PathBuf};

use bande_fs::Recency;
use bande_search::{FUZZY, FileIndex};
use bande_toolkit::ui::{BUTTON_HEIGHT, PopUpButton, SEGMENTED_HEIGHT, SegmentedControl, WidgetId};
use bande_toolkit::{Item, Point, Rect, TextAlign, TextStyle};

use crate::L10N;
use crate::browser::Location;
use crate::controls::{Signal, sender};
use crate::tabs::location_title;
use crate::view::{FilesView, home};
use crate::views::ViewMode;

pub const SEARCH_HEIGHT: f32 = 36.0;
const MAX_FOUND: usize = 2000;
const FILTER_WIDTH: f32 = 130.0;
const FILTER_GAP: f32 = 8.0;
const ARCHIVES: &[&str] = &[
    "application/zip",
    "application/gzip",
    "application/x-tar",
    "application/x-7z-compressed",
    "application/x-rar",
    "application/vnd.rar",
    "application/x-xz",
    "application/x-bzip2",
    "application/zstd",
    "application/x-compressed-tar",
    "application/x-xz-compressed-tar",
    "application/x-bzip2-compressed-tar",
    "application/x-zstd-compressed-tar",
];
const DOCUMENTS: &[&str] = &[
    "application/pdf",
    "application/rtf",
    "application/msword",
    "application/epub+zip",
    "application/vnd.oasis.opendocument.",
    "application/vnd.openxmlformats-officedocument.",
    "application/vnd.ms-",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    Any,
    Folder,
    Document,
    Image,
    Audio,
    Video,
    Archive,
}

impl SearchKind {
    pub const ALL: [Self; 7] = [
        Self::Any,
        Self::Folder,
        Self::Document,
        Self::Image,
        Self::Audio,
        Self::Video,
        Self::Archive,
    ];

    fn title(self) -> String {
        L10N.tr(match self {
            Self::Any => "kind-any",
            Self::Folder => "kind-folder",
            Self::Document => "kind-document",
            Self::Image => "kind-image",
            Self::Audio => "kind-audio",
            Self::Video => "kind-video",
            Self::Archive => "kind-archive",
        })
    }

    fn matches(self, mime: &str, folder: bool) -> bool {
        match self {
            Self::Any => true,
            Self::Folder => folder,
            _ if folder => false,
            Self::Document => {
                mime.starts_with("text/") || DOCUMENTS.iter().any(|known| mime.starts_with(known))
            }
            Self::Image => mime.starts_with("image/"),
            Self::Audio => mime.starts_with("audio/"),
            Self::Video => mime.starts_with("video/"),
            Self::Archive => ARCHIVES.contains(&mime),
        }
    }
}

fn recency_title(recency: Recency) -> String {
    L10N.tr(match recency {
        Recency::AnyTime => "date-any-time",
        Recency::Today => "date-today-filter",
        Recency::LastWeek => "date-last-week",
        Recency::LastMonth => "date-last-month",
        Recency::ThisYear => "date-this-year",
    })
}

pub(crate) struct Searching {
    pub text: String,
    pub folder: PathBuf,
    pub everywhere: bool,
    before: Location,
    before_mode: ViewMode,
    pub kind: SearchKind,
    pub recency: Recency,
    hits: Vec<PathBuf>,
    pub(crate) scope_control: WidgetId,
    pub(crate) kind_control: WidgetId,
    pub(crate) date_control: WidgetId,
}

pub(crate) struct Index {
    root: PathBuf,
    index: FileIndex,
    generation: u64,
}

fn covered(root: &Path, folder: &Path) -> bool {
    folder.strip_prefix(root).is_ok_and(|inside| {
        inside
            .components()
            .all(|part| !part.as_os_str().to_string_lossy().starts_with('.'))
    })
}

impl FilesView {
    pub fn searching(&self) -> Option<&str> {
        self.searching
            .as_ref()
            .map(|searching| searching.text.as_str())
    }

    pub(crate) fn search_index_fd(&self) -> Option<std::os::fd::BorrowedFd<'_>> {
        self.index.as_ref().map(|index| index.index.fd())
    }

    fn scope_widget(&self, folder: &Path, everywhere: bool) -> SegmentedControl {
        let send = sender(&self.signals);
        let titles = [
            L10N.tr_args(
                "search-scope-folder",
                [(
                    "folder",
                    location_title(&Location::Folder(folder.to_path_buf())).into(),
                )],
            ),
            L10N.tr("search-scope-home"),
        ];
        SegmentedControl::new(
            &L10N.tr("search-scope"),
            &titles,
            Some(usize::from(everywhere)),
            move |index| send(Signal::Scope(index == 1)),
        )
    }

    pub(crate) fn search_changed(&mut self, text: &str) {
        if text.trim().is_empty() {
            self.end_search();
            return;
        }
        if let Some(searching) = &mut self.searching {
            searching.text = text.to_owned();
        } else {
            self.begin_search(text);
        }
        self.query();
    }

    fn filter_widgets(&self) -> (PopUpButton, PopUpButton) {
        let (kind, date) = (sender(&self.signals), sender(&self.signals));
        let kinds = SearchKind::ALL.map(SearchKind::title);
        let dates = Recency::ALL.map(recency_title);
        (
            PopUpButton::new(&L10N.tr("search-kind"), &kinds, 0, move |index| {
                kind(Signal::SearchKind(index));
            }),
            PopUpButton::new(&L10N.tr("search-date"), &dates, 0, move |index| {
                date(Signal::SearchDate(index));
            }),
        )
    }

    fn begin_search(&mut self, text: &str) {
        self.commit_rename();
        self.quick_look = None;
        let before = self.browser.root().clone();
        let folder = match &before {
            Location::Folder(folder) => folder.clone(),
            _ => home(),
        };
        let everywhere = self.search_home || folder == home();
        let scope_control = self.insert(self.scope_widget(&folder, everywhere));
        let (kind, date) = self.filter_widgets();
        let (kind_control, date_control) = (self.insert(kind), self.insert(date));
        self.searching = Some(Searching {
            text: text.to_owned(),
            before,
            before_mode: self.mode,
            folder: folder.clone(),
            everywhere,
            kind: SearchKind::Any,
            recency: Recency::AnyTime,
            hits: Vec::new(),
            scope_control,
            kind_control,
            date_control,
        });
        self.mode = ViewMode::List;
        self.browser.set_single(true);
        self.browser.set_found(Vec::new());
        self.browser.replace(Location::Search(folder));
        self.shown_root = Some(self.browser.root().clone());
        self.refresh_size_slider();
        self.refresh_view_control();
        self.selection_changed();
    }

    fn scope(&self) -> Option<PathBuf> {
        let searching = self.searching.as_ref()?;
        Some(match searching.everywhere {
            true => home(),
            false => searching.folder.clone(),
        })
    }

    fn query(&mut self) {
        let (Some(scope), Some(text)) = (
            self.scope(),
            self.searching
                .as_ref()
                .map(|searching| searching.text.clone()),
        ) else {
            return;
        };
        let home = home();
        let root = match covered(&home, &scope) {
            true => home,
            false => scope.clone(),
        };
        if !self
            .index
            .as_ref()
            .is_some_and(|index| covered(&index.root, &scope))
        {
            match FileIndex::spawn(root.clone()) {
                Ok(index) => {
                    self.index = Some(Index {
                        root,
                        index,
                        generation: 0,
                    });
                }
                Err(error) => {
                    tracing::warn!("cannot search {}: {error}", root.display());
                    return;
                }
            }
        }
        if let Some(index) = &mut self.index {
            index.generation += 1;
            index
                .index
                .query_within(index.generation, &text, &scope, MAX_FOUND);
        }
    }

    pub(crate) fn search_results(&mut self) -> bool {
        let Some(index) = &mut self.index else {
            return false;
        };
        let Some(results) = index.index.take() else {
            return false;
        };
        if results.generation != index.generation || self.searching.is_none() {
            return false;
        }
        let found = results
            .hits
            .into_iter()
            .filter(|hit| hit.score > FUZZY)
            .map(|hit| hit.path)
            .collect();
        if let Some(searching) = &mut self.searching {
            searching.hits = found;
        }
        self.show_filtered();
        true
    }

    fn show_filtered(&mut self) {
        let Some(searching) = &self.searching else {
            return;
        };
        let (kind, recency) = (searching.kind, searching.recency);
        let found = searching
            .hits
            .iter()
            .filter(|path| {
                if kind == SearchKind::Any && recency == Recency::AnyTime {
                    return true;
                }
                let Ok(metadata) = std::fs::metadata(path) else {
                    return false;
                };
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_default();
                kind.matches(self.apps.mime_for_name(&name), metadata.is_dir())
                    && recency.includes(metadata.modified().ok())
            })
            .cloned()
            .collect();
        self.browser.set_found(found);
        self.selection_changed();
    }

    pub(crate) fn set_search_kind(&mut self, index: usize) {
        if let Some(searching) = &mut self.searching {
            searching.kind = SearchKind::ALL
                .get(index)
                .copied()
                .unwrap_or(SearchKind::Any);
        }
        self.show_filtered();
    }

    pub(crate) fn set_search_date(&mut self, index: usize) {
        if let Some(searching) = &mut self.searching {
            searching.recency = Recency::ALL.get(index).copied().unwrap_or(Recency::AnyTime);
        }
        self.show_filtered();
    }

    pub(crate) fn set_scope(&mut self, everywhere: bool) {
        let Some(searching) = &mut self.searching else {
            return;
        };
        if searching.everywhere == everywhere {
            return;
        }
        searching.everywhere = everywhere;
        let location = Location::Search(match everywhere {
            true => home(),
            false => searching.folder.clone(),
        });
        self.browser.replace(location);
        self.shown_root = Some(self.browser.root().clone());
        self.query();
        self.selection_changed();
    }

    pub(crate) fn end_search(&mut self) {
        let Some(searching) = self.searching.take() else {
            return;
        };
        for control in [
            searching.scope_control,
            searching.kind_control,
            searching.date_control,
        ] {
            self.remove(control);
        }
        self.reset_search_field();
        self.mode = searching.before_mode;
        self.browser
            .set_single(searching.before_mode != ViewMode::Columns);
        self.browser.replace(searching.before);
        self.shown_root = Some(self.browser.root().clone());
        self.refresh_size_slider();
        self.refresh_view_control();
        self.selection_changed();
    }

    pub(crate) fn focus_search(&mut self) {
        self.commit_rename();
        self.focus(self.toolbar.search);
    }

    pub(crate) fn show_in_enclosing_folder(&mut self) {
        let Some(path) = self.browser.selected().map(|entry| entry.path.clone()) else {
            return;
        };
        self.end_search();
        self.browser.reveal(&path);
        self.selection_changed();
        self.focus_content();
    }

    pub(crate) fn scope_frames(&self) -> Vec<(WidgetId, Rect)> {
        let Some(searching) = self.searching.as_ref() else {
            return Vec::new();
        };
        let content = self.content_frame();
        let top = content.y - SEARCH_HEIGHT;
        let filters = 2.0 * FILTER_WIDTH + FILTER_GAP + 12.0;
        let date_x = content.x + content.width - 12.0 - FILTER_WIDTH;
        let pop_up_y = top + ((SEARCH_HEIGHT - BUTTON_HEIGHT) / 2.0).round();
        vec![
            (
                searching.scope_control,
                Rect::new(
                    content.x + 64.0,
                    top + ((SEARCH_HEIGHT - SEGMENTED_HEIGHT) / 2.0).round(),
                    280.0_f32.min(content.width - 80.0 - filters).max(0.0),
                    SEGMENTED_HEIGHT,
                ),
            ),
            (
                searching.kind_control,
                Rect::new(
                    date_x - FILTER_GAP - FILTER_WIDTH,
                    pop_up_y,
                    FILTER_WIDTH,
                    BUTTON_HEIGHT,
                ),
            ),
            (
                searching.date_control,
                Rect::new(date_x, pop_up_y, FILTER_WIDTH, BUTTON_HEIGHT),
            ),
        ]
    }

    pub(crate) fn paint_scope_bar(&mut self, items: &mut Vec<Item>) {
        if self.searching.is_none() {
            return;
        }
        let content = self.content_frame();
        let bar = Rect::new(
            content.x,
            content.y - SEARCH_HEIGHT,
            content.width,
            SEARCH_HEIGHT,
        );
        let muted = self.palette.ink2;
        let label = self.text().paragraph(
            &L10N.tr("search-scope"),
            TextStyle::new(12.0, muted).weight(TextStyle::SEMIBOLD),
            None,
            TextAlign::Start,
        );
        items.push(Item::Text {
            origin: Point::new(
                bar.x + 12.0,
                bar.y + ((SEARCH_HEIGHT - label.height()) / 2.0).round(),
            ),
            paragraph: label,
        });
    }
}
