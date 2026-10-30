use unbound_fs::Sort;
use unbound_toolkit::Point;
use unbound_toolkit::ui::Menu;

use crate::browser::Location;
use crate::menus::{
    App, Command, Folder, Place, Selection, command, folder_menu, items_menu, place_menu,
};
use crate::sidebar::{PlaceKind, Row};
use crate::view::{FilesView, Hit, Request};
use crate::views::ViewMode;

impl FilesView {
    pub(crate) fn context_click(&mut self, hit: Hit, at: Point) -> bool {
        match hit {
            Hit::Row { column, row } => {
                if self.rename.as_ref().is_some_and(|rename| {
                    rename.path == self.browser.columns()[column].entries[row].path
                }) {
                    return false;
                }
                self.commit_rename();
                if column != self.browser.active() || !self.browser.columns()[column].is_marked(row)
                {
                    self.browser.select(column, row);
                }
                self.selection_changed();
                self.show_menu(self.selection_menu(), at);
            }
            Hit::Column(column) => {
                self.commit_rename();
                self.browser.focus_column(column);
                self.selection_changed();
                self.show_menu(self.folder_menu(), at);
            }
            Hit::Inspector => {
                let menu = if self.browser.selection().is_empty() {
                    self.folder_menu()
                } else {
                    self.selection_menu()
                };
                self.show_menu(menu, at);
            }
            Hit::Place(index) => {
                let Some(Row::Place(place)) = self.sidebar.get(index) else {
                    return false;
                };
                let menu = place_menu(match &place.location {
                    Location::Trash => Place::Trash {
                        empty: !self.trash_full,
                    },
                    Location::Folder(_) if place.kind == PlaceKind::Volume => Place::Volume,
                    _ => Place::Folder {
                        removable: place.kind == PlaceKind::Bookmark,
                    },
                });
                self.menu_place = Some(index);
                self.show_menu(menu, at);
            }
            _ => return false,
        }
        true
    }

    pub(crate) fn keyboard_menu(&mut self) {
        let active = self.browser.active();
        let selected = self.browser.active_column().selected;
        let anchor = |hit: &Hit| match (hit, selected) {
            (Hit::Row { column, row }, Some(selected)) => *column == active && *row == selected,
            (Hit::Column(column), None) => *column == active,
            _ => false,
        };
        let rect = match self.mode == ViewMode::List {
            true => selected
                .and_then(|row| self.table_row_bounds(row))
                .or_else(|| self.frame_of(self.table)),
            false => self
                .hits
                .iter()
                .rev()
                .find(|(_, hit)| anchor(hit))
                .map(|(rect, _)| *rect),
        };
        let Some(rect) = rect else {
            return;
        };
        let at = Point::new(rect.x + 24.0, rect.y + rect.height);
        let menu = if selected.is_some() {
            self.selection_menu()
        } else {
            self.folder_menu()
        };
        self.show_menu(menu, at);
    }

    pub(crate) fn show_menu(&mut self, menu: Menu, at: Point) {
        self.requests.push(Request::ShowMenu { menu, at });
    }

    pub(crate) fn selection_menu(&self) -> Menu {
        let selection = self.browser.selection();
        let trashed =
            !selection.is_empty() && selection.iter().all(|entry| entry.trashed.is_some());
        let files = selection
            .iter()
            .filter(|entry| !entry.is_folder())
            .collect::<Vec<_>>();
        let mimes = files
            .iter()
            .map(|entry| self.apps.mime_for(&entry.path))
            .collect::<Vec<_>>();
        let apps = match mimes.split_first() {
            Some((first, rest))
                if files.len() == selection.len() && rest.iter().all(|mime| mime == first) =>
            {
                self.apps
                    .apps_for(first)
                    .into_iter()
                    .map(|entry| App {
                        id: entry.id.clone(),
                        name: entry.name.clone(),
                    })
                    .collect()
            }
            _ => Vec::new(),
        };
        items_menu(&Selection {
            count: selection.len(),
            trashed,
            searching: self.searching.is_some(),
            folders: !selection.is_empty()
                && selection
                    .iter()
                    .all(|entry| entry.is_folder() && entry.trashed.is_none()),
            apps,
        })
    }

    pub(crate) fn folder_menu(&self) -> Menu {
        folder_menu(Folder {
            trash: self.browser.active_column().location == Location::Trash,
            trash_empty: !self.trash_full,
            sort: self.browser.sort().by,
            mode: self.mode,
            hidden_shown: self.browser.shows_hidden(),
            paste: self.clipboard_files,
        })
    }

    pub fn menu_action(&mut self, action: &str) -> bool {
        let Some(command) = command(action) else {
            return false;
        };
        let place = self.menu_place.take();
        match command {
            Command::Open => self.open(),
            Command::OpenInNewTab => self.open_selection_in_tabs(),
            Command::OpenWith(id) => self.open_with(&id),
            Command::QuickLook => {
                if self.quick_look.is_none() {
                    self.toggle_quick_look();
                }
            }
            Command::Info => self.show_inspector(),
            Command::Rename => self.start_rename(),
            Command::Duplicate => self.duplicate(),
            Command::Copy => self.copy_selection(false),
            Command::Cut => self.copy_selection(true),
            Command::ShowInEnclosingFolder => self.show_in_enclosing_folder(),
            Command::Trash | Command::PutBack => self.trash_or_put_back(),
            Command::DeleteImmediately => self.confirm_delete(),
            Command::NewFolder => self.new_folder(),
            Command::Paste => self.paste(false),
            Command::Sort(by) => {
                let current = self.browser.sort();
                if current.by != by {
                    self.browser.set_sort(Sort {
                        by,
                        descending: false,
                    });
                    self.selection_changed();
                }
            }
            Command::Icons => self.set_mode(ViewMode::Icons),
            Command::Columns => self.set_mode(ViewMode::Columns),
            Command::List => self.set_mode(ViewMode::List),
            Command::ToggleHidden => self.toggle_hidden(),
            Command::EmptyTrash => self.ask_to_empty_trash(),
            Command::OpenPlace
            | Command::OpenPlaceInNewTab
            | Command::RemoveFromSidebar
            | Command::Eject => {
                let Some(Row::Place(place)) = place.and_then(|index| self.sidebar.get(index))
                else {
                    return false;
                };
                let location = place.location.clone();
                match (command, location) {
                    (Command::OpenPlace, location) => self.go(location),
                    (Command::OpenPlaceInNewTab, location) => self.open_tab(location),
                    (Command::Eject, Location::Folder(path)) => {
                        self.requests.push(Request::Eject(path));
                    }
                    (_, Location::Folder(path)) => {
                        self.requests.push(Request::RemoveBookmark(path));
                    }
                    _ => {}
                }
            }
        }
        true
    }

    fn open_with(&mut self, id: &str) {
        let files = self.browser.selected_paths();
        let app = files.first().and_then(|path| {
            self.apps
                .apps_for_path(path)
                .into_iter()
                .find(|app| app.id == id)
                .cloned()
        });
        if let Some(app) = app {
            self.requests.push(Request::Open {
                app: Box::new(app),
                files,
            });
        }
    }

    pub fn menu_closed(&mut self) {
        self.menu_place = None;
    }
}
