use std::path::{Path, PathBuf};

use unbound_search::{FUZZY, FileIndex};
use unbound_toolkit::ui::{SEGMENTED_HEIGHT, SegmentedControl, WidgetId};
use unbound_toolkit::{Item, Point, Rect, TextAlign, TextStyle};

use crate::L10N;
use crate::browser::Location;
use crate::controls::{Signal, sender};
use crate::tabs::location_title;
use crate::view::{FilesView, home};
use crate::views::ViewMode;

pub const SEARCH_HEIGHT: f32 = 36.0;
const MAX_FOUND: usize = 2000;

pub(crate) struct Searching {
    pub text: String,
    pub folder: PathBuf,
    pub everywhere: bool,
    before: Location,
    before_mode: ViewMode,
    pub(crate) scope_control: WidgetId,
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

    fn begin_search(&mut self, text: &str) {
        self.commit_rename();
        self.quick_look = None;
        let before = self.browser.root().clone();
        let folder = match &before {
            Location::Folder(folder) => folder.clone(),
            _ => home(),
        };
        let everywhere = folder == home();
        let scope_control = self.insert(self.scope_widget(&folder, everywhere));
        self.searching = Some(Searching {
            text: text.to_owned(),
            before,
            before_mode: self.mode,
            folder: folder.clone(),
            everywhere,
            scope_control,
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
        self.browser.set_found(found);
        self.selection_changed();
        true
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
        self.remove(searching.scope_control);
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

    pub(crate) fn scope_frame(&self) -> Option<(WidgetId, Rect)> {
        let searching = self.searching.as_ref()?;
        let content = self.content_frame();
        Some((
            searching.scope_control,
            Rect::new(
                content.x + 64.0,
                content.y - SEARCH_HEIGHT + ((SEARCH_HEIGHT - SEGMENTED_HEIGHT) / 2.0).round(),
                280.0_f32.min(content.width - 80.0),
                SEGMENTED_HEIGHT,
            ),
        ))
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
