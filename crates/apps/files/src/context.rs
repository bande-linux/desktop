use bande_fs::{Entry, GroupBy, Sort};
use bande_toolkit::Point;
use bande_toolkit::ui::Menu;
use bande_volumes::load_saved_servers;
use bande_xdg::{Apps, UserDir, config_dirs, data_dirs};

use crate::backups::backups_installed;
use crate::browser::Location;
use std::path::PathBuf;

use crate::controls::{COLUMN_ACTION, COLUMN_LEFT_ACTION, COLUMN_RIGHT_ACTION};
use crate::menus::{
    Command, Destination, Folder, Place, Selection, command, folder_menu, items_menu, network_menu,
    place_menu, server_menu, sidebar_menu,
};
use crate::sidebar::{BookmarkEdit, PlaceKind, Row, SidebarEdit, favourites, layout_text};
use crate::view::{FilesView, Hit, Request, user_folder};
use crate::views::{ListColumn, ViewMode};

impl FilesView {
    pub(crate) fn context_click(&mut self, hit: Hit, at: Point) -> bool {
        match hit {
            Hit::Row { column, row } => {
                let Some(entry) = self
                    .browser
                    .columns()
                    .get(column)
                    .and_then(|shown| shown.entries.get(row))
                else {
                    return false;
                };
                if self
                    .rename
                    .as_ref()
                    .is_some_and(|rename| rename.path == entry.path)
                {
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
                    _ => Place::Folder,
                });
                self.menu_place = Some(index);
                self.show_menu(menu, at);
            }
            Hit::Sidebar => self.show_menu(sidebar_menu(), at),
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
        if let Some(url) = self.browser.chosen_server().and_then(Entry::server_address) {
            return server_menu(load_saved_servers().iter().any(|server| server.url == url));
        }
        let selection = self.browser.selection();
        let trashed =
            !selection.is_empty() && selection.iter().all(|entry| entry.trashed.is_some());
        let only_files = selection.iter().all(|entry| !entry.is_folder());
        let apps = match only_files {
            true => self.apps.open_with_choices(&self.browser.selected_paths()),
            false => Vec::new(),
        };
        items_menu(&Selection {
            count: selection.len(),
            name: selection
                .first()
                .map(|entry| entry.name.clone())
                .unwrap_or_default(),
            trashed,
            in_backup: self.in_backup(),
            can_browse_backups: backups_installed() && !trashed && !self.in_backup(),
            away: self.searching.is_some() || *self.browser.root() == Location::Recents,
            files: !selection.is_empty() && only_files && !trashed,
            folders: !selection.is_empty()
                && selection
                    .iter()
                    .all(|entry| entry.is_folder() && entry.trashed.is_none()),
            terminal: self.terminal_installed(),
            apps,
            mail: only_files && !trashed && self.mail_app().is_some(),
            places: match trashed {
                true => Vec::new(),
                false => self
                    .mounted
                    .iter()
                    .filter(|place| !selection.iter().any(|entry| entry.path.starts_with(place)))
                    .cloned()
                    .collect(),
            },
        })
    }

    fn single_file(&self) -> Option<PathBuf> {
        match self.browser.selected_paths().as_slice() {
            [path] if !bande_volumes::on_connected_volume(path) && path.is_file() => {
                Some(path.clone())
            }
            _ => None,
        }
    }

    pub fn can_print(&self) -> bool {
        self.single_file()
            .is_some_and(|path| bande_app::can_print(&self.apps.mime_for(&path)))
    }

    pub fn printable(&self) -> Option<(String, bande_app::Document)> {
        let path = self.single_file()?;
        let document = bande_app::file_document(&path, &self.apps.mime_for(&path))?;
        let title = path.file_name()?.to_string_lossy().into_owned();
        Some((title, document))
    }

    pub fn terminal_installed(&self) -> bool {
        self.terminal
    }

    pub fn set_terminal_installed(&mut self, installed: bool) {
        self.terminal = installed;
    }

    pub fn reload_apps(&mut self) {
        self.apps = Apps::load(data_dirs(), &config_dirs());
    }

    pub(crate) fn folder_menu(&self) -> Menu {
        if self.browser.active_column().location == Location::Network {
            return network_menu();
        }
        folder_menu(Folder {
            templates: self.templates(),
            trash: self.browser.active_column().location == Location::Trash,
            trash_empty: !self.trash_full,
            paste: self.clipboard_files,
            terminal: self.terminal_installed(),
            can_browse_backups: backups_installed() && !self.in_backup(),
        })
    }

    pub fn menu_action(&mut self, action: &str) -> bool {
        if let Some(column) = action.strip_prefix(COLUMN_ACTION).and_then(|name| {
            ListColumn::ALL
                .into_iter()
                .find(|known| known.name() == name)
        }) {
            self.toggle_list_column(column);
            return true;
        }
        let moved = |prefix: &str| {
            let name = action.strip_prefix(prefix)?;
            ListColumn::ALL
                .into_iter()
                .find(|known| known.name() == name)
        };
        if let Some(column) = moved(COLUMN_LEFT_ACTION) {
            self.move_list_column(column, -1);
            return true;
        }
        if let Some(column) = moved(COLUMN_RIGHT_ACTION) {
            self.move_list_column(column, 1);
            return true;
        }
        let Some(command) = command(action) else {
            let handled = self.ui.menu_action(action);
            self.after_ui();
            return handled;
        };
        let place = self.menu_place.take();
        match command {
            Command::Open => self.open(),
            Command::OpenInNewTab => self.open_selection_in_tabs(),
            Command::OpenInTerminal => self.open_in_terminal(),
            Command::NewDocument(name) => self.new_document(&name),
            Command::OpenWith(id) => self.open_with(&id),
            Command::OpenWithOther => {
                let files = self.browser.selected_paths();
                self.ask_open_with(files);
            }
            Command::LookUp => {
                let stem = self
                    .browser
                    .selected_paths()
                    .first()
                    .and_then(|path| path.file_stem()?.to_str().map(str::to_owned));
                if let Some(stem) = stem {
                    self.requests.push(Request::LookUp(stem));
                }
            }
            Command::QuickLook => {
                if !self.quick_look_shown() {
                    self.open_quick_look(false);
                }
            }
            Command::Info => self.show_inspector(),
            Command::Rename => self.start_rename(),
            Command::Duplicate => self.duplicate(),
            Command::MakeAlias => self.make_alias(),
            Command::Compress => self.compress(),
            Command::ShareMail => self.share_by_mail(),
            Command::ShareTo(place) => self.share_to(place),
            Command::Copy => self.copy_selection(false),
            Command::CopyPath => self.copy_paths(),
            Command::AddToSidebar => self.add_to_sidebar(),
            Command::Cut => self.copy_selection(true),
            Command::ShowInEnclosingFolder => self.show_in_enclosing_folder(),
            Command::BrowseBackups => self.browse_backups(),
            Command::RestoreFromBackup => self.restore_from_backup(),
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
                        ..current
                    });
                    self.remember_view();
                    self.selection_changed();
                }
            }
            Command::UseGroups => {
                let group = match self.browser.groups() {
                    Some(_) => None,
                    None => Some(GroupBy::Kind),
                };
                self.set_group(group);
            }
            Command::Group(group) => self.set_group(group),
            Command::Icons => self.set_mode(ViewMode::Icons),
            Command::Columns => self.set_mode(ViewMode::Columns),
            Command::List => self.set_mode(ViewMode::List),
            Command::Gallery => self.set_mode(ViewMode::Gallery),
            Command::ToggleHidden => self.toggle_hidden(),
            Command::ToggleStatusBar => self.toggle_status_bar(),
            Command::QuickLookFullScreen => self.toggle_quick_look_full_screen(),
            Command::MoveHere => self.paste(true),
            Command::EjectCurrent => self.eject_current(),
            Command::GoTo(place) => self.go(match place {
                Destination::Desktop => Location::Folder(user_folder(UserDir::Desktop)),
                Destination::Documents => Location::Folder(user_folder(UserDir::Documents)),
                Destination::Downloads => Location::Folder(user_folder(UserDir::Downloads)),
                Destination::Recents => Location::Recents,
            }),
            Command::ConnectToServer => self.ask_connect(),
            Command::ForgetServer => {
                if let Some(url) = self.browser.chosen_server().and_then(Entry::server_address) {
                    self.requests.push(Request::ForgetServer(url.to_owned()));
                }
            }
            Command::EmptyTrash => self.ask_to_empty_trash(),
            Command::RestoreSidebar => self.requests.push(Request::EditSidebar(SidebarEdit {
                layout: None,
                bookmarks: None,
            })),
            Command::OpenPlaceInNewTab | Command::RemoveFromSidebar | Command::Eject => {
                let Some((index, Row::Place(place))) =
                    place.and_then(|index| Some((index, self.sidebar.get(index)?)))
                else {
                    return false;
                };
                let location = place.location.clone();
                match (command, location) {
                    (Command::OpenPlaceInNewTab, location) => self.open_tab(location),
                    (Command::Eject, Location::Folder(path)) => {
                        self.requests.push(Request::Eject(path));
                    }
                    (Command::RemoveFromSidebar, _) => self.remove_from_sidebar(index),
                    _ => {}
                }
            }
        }
        true
    }

    fn open_with(&mut self, id: &str) {
        let files = self.browser.selected_paths();
        let app = self
            .apps
            .apps_for_files(&files)
            .into_iter()
            .find(|app| app.id == id)
            .cloned();
        if let Some(app) = app {
            self.requests.push(Request::Open {
                app: Box::new(app),
                files,
            });
        }
    }

    fn save_sidebar(
        &mut self,
        places: &[crate::sidebar::Place],
        hidden: &[Location],
        bookmarks: Option<BookmarkEdit>,
    ) {
        let places = places.iter().collect::<Vec<_>>();
        self.requests.push(Request::EditSidebar(SidebarEdit {
            layout: Some(layout_text(&places, hidden)),
            bookmarks,
        }));
    }

    fn favourite_places(&self) -> Vec<crate::sidebar::Place> {
        favourites(&self.favourites).into_iter().cloned().collect()
    }

    pub(crate) fn remove_from_sidebar(&mut self, index: usize) {
        let Some(Row::Place(place)) = self.sidebar.get(index).cloned() else {
            return;
        };
        let mut places = self.favourite_places();
        places.retain(|known| known.location != place.location);
        let mut hidden = self.hidden_places.clone();
        let bookmarks = match (place.kind, &place.location) {
            (PlaceKind::Bookmark, Location::Folder(path)) => {
                Some(BookmarkEdit::Remove(path.clone()))
            }
            _ => {
                if !hidden.contains(&place.location) {
                    hidden.push(place.location.clone());
                }
                None
            }
        };
        self.save_sidebar(&places, &hidden, bookmarks);
    }

    pub(crate) fn place_in_sidebar(&mut self, locations: Vec<Location>, before: Option<Location>) {
        let mut places = self.favourite_places();
        let mut hidden = self.hidden_places.clone();
        let mut added: Vec<PathBuf> = Vec::new();
        let moved = locations
            .into_iter()
            .filter(|location| Some(location) != before.as_ref())
            .filter_map(|location| {
                let was_hidden = hidden.contains(&location);
                hidden.retain(|known| *known != location);
                if let Some(at) = places.iter().position(|known| known.location == location) {
                    return Some(places.remove(at));
                }
                let Location::Folder(path) = &location else {
                    return None;
                };
                if !was_hidden {
                    added.push(path.clone());
                }
                Some(crate::sidebar::Place {
                    name: path.file_name().map_or_else(
                        || path.display().to_string(),
                        |name| name.to_string_lossy().into_owned(),
                    ),
                    location,
                    icon: "folder",
                    kind: PlaceKind::Bookmark,
                })
            })
            .collect::<Vec<_>>();
        if moved.is_empty() {
            return;
        }
        let at = before
            .and_then(|before| places.iter().position(|known| known.location == before))
            .unwrap_or(places.len());
        let moved_bookmark = match moved.as_slice() {
            [place] if place.kind == PlaceKind::Bookmark => match &place.location {
                Location::Folder(path) => Some(path.clone()),
                _ => None,
            },
            _ => None,
        };
        places.splice(at..at, moved);
        let bookmarks = if !added.is_empty() {
            Some(BookmarkEdit::Add(added))
        } else {
            moved_bookmark.map(|folder| {
                let before = places
                    .iter()
                    .skip_while(|place| place.location != Location::Folder(folder.clone()))
                    .skip(1)
                    .find_map(|place| match (&place.kind, &place.location) {
                        (PlaceKind::Bookmark, Location::Folder(path)) => Some(path.clone()),
                        _ => None,
                    });
                BookmarkEdit::Move { folder, before }
            })
        };
        self.save_sidebar(&places, &hidden, bookmarks);
    }

    pub fn menu_closed(&mut self) {
        self.menu_place = None;
    }
}
