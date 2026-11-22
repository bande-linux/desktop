use std::cell::RefCell;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;
use bande_app::{Dropping, FileDrop, View};
use bande_l10n::tr;
use bande_media::{FailureKind, MediaInfo, Opening, Playback, VideoInfo};
use bande_media_view::{Session, StageWidget};
use bande_toolkit::ui::{
    Cell, Column, Context, Event, FreeLayout, Key, Menu, MenuItem, Row, Table, Ui, Widget, WidgetId,
};
use bande_toolkit::{Appearance, Rect, Scene, Sizing, TextSystem, Window, WindowEvent};

use crate::mpris::{NowPlaying, Remote};
use crate::positions::Positions;

pub fn file_title(path: &Path) -> String {
    path.file_stem()
        .or(path.file_name())
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

const AUDIO_SIZE: (u32, u32) = (420, 360);
const QUEUE_WIDTH: f32 = 260.0;
const STATE_WIDTH: f32 = 72.0;
const PLAY: &str = "play";
const MOVE_UP: &str = "move-up";
const MOVE_DOWN: &str = "move-down";
const REMOVE: &str = "remove";

enum Choice {
    Selected,
    Play(usize),
    Menu(String, usize),
}

type Choices = Rc<RefCell<Vec<Choice>>>;

fn chosen_index(keys: &[String]) -> Option<usize> {
    keys.first()?.parse().ok()
}

fn queue_table(choices: &Choices) -> Table {
    let (selected, played, acted) = (Rc::clone(choices), Rc::clone(choices), Rc::clone(choices));
    Table::new(vec![
        Column::new(&tr!("Title"), QUEUE_WIDTH - STATE_WIDTH)
            .flexible()
            .unsortable(),
        Column::new(&tr!("State"), STATE_WIDTH).unsortable(),
    ])
    .without_header()
    .sidebar()
    .on_selection(move |_| selected.borrow_mut().push(Choice::Selected))
    .on_activate(move |keys| {
        if let Some(index) = chosen_index(keys) {
            played.borrow_mut().push(Choice::Play(index));
        }
    })
    .context_menu(
        |keys| {
            chosen_index(keys)?;
            Some(Menu::new(vec![
                MenuItem::new(&tr!("Play"), PLAY),
                MenuItem::separator(),
                MenuItem::new(&tr!("Move Up"), MOVE_UP),
                MenuItem::new(&tr!("Move Down"), MOVE_DOWN),
                MenuItem::separator(),
                MenuItem::new(&tr!("Remove from Queue"), REMOVE),
            ]))
        },
        move |action, keys| {
            if let Some(index) = chosen_index(keys) {
                acted
                    .borrow_mut()
                    .push(Choice::Menu(action.to_owned(), index));
            }
        },
    )
}
const LARGEST_FIT: (f64, f64) = (1120.0, 700.0);

fn fitted_size(window: &Window, video: VideoInfo) -> (u32, u32) {
    let scale = window.scale().max(1.0);
    let (width, height) = (
        f64::from(video.display_width) / scale,
        f64::from(video.height) / scale,
    );
    let shrink = (LARGEST_FIT.0 / width).min(LARGEST_FIT.1 / height).min(1.0);
    ((width * shrink) as u32, (height * shrink) as u32)
}

pub struct Pane {
    ui: Ui,
    host: WidgetId,
    stage: WidgetId,
    table: WidgetId,
    choices: Choices,
    queue_shown: bool,
    queue: Vec<PathBuf>,
    at: usize,
    looping: bool,
    remembered: Option<Playback>,
    fitted: bool,
    shown_title: Option<String>,
    menus_changed: bool,
    pending_load: bool,
    drop: FileDrop,
    positions: Rc<RefCell<Positions>>,
}

impl Pane {
    pub fn new(
        appearance: Appearance,
        queue: Vec<PathBuf>,
        positions: Rc<RefCell<Positions>>,
    ) -> Self {
        let choices: Choices = Rc::default();
        let mut host = FreeLayout::new();
        let stage = host.insert(StageWidget::new(), Rect::default());
        let host_id = Widget::id(&host);
        let mut ui = Ui::new(
            Box::new(host),
            Context::new(TextSystem::new(), appearance.palette()),
        );
        ui.set_appearance(appearance);
        ui.set_focus(stage);
        let mut pane = Self {
            ui,
            host: host_id,
            stage,
            table: WidgetId::next(),
            choices,
            queue_shown: false,
            queue,
            at: 0,
            looping: false,
            remembered: None,
            fitted: false,
            shown_title: None,
            menus_changed: false,
            drop: FileDrop::default(),
            pending_load: false,
            positions,
        };
        pane.load();
        pane
    }

    fn fill_queue(&mut self) {
        let playing = tr!("Playing");
        let rows = self
            .queue
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let state = match index == self.at {
                    true => Cell::new(playing.as_str()).muted(),
                    false => Cell::new(""),
                };
                Row::new(index.to_string(), vec![Cell::new(file_title(path)), state])
            })
            .collect();
        if let Some(table) = self.ui.widget_as::<Table>(self.table) {
            table.set_rows(rows);
        }
    }

    pub fn queue_shown(&self) -> bool {
        self.queue_shown
    }

    pub fn toggle_queue(&mut self) {
        self.queue_shown = !self.queue_shown;
        let Some(host) = self.ui.widget_as::<FreeLayout>(self.host) else {
            return;
        };
        let focus = match self.queue_shown {
            true => {
                self.table = host.insert(queue_table(&self.choices), Rect::default());
                self.table
            }
            false => {
                host.remove(self.table);
                self.stage
            }
        };
        self.fill_queue();
        self.ui.set_focus(focus);
    }

    fn selected(&self) -> Option<usize> {
        let table = self.ui.widget_ref::<Table>(self.table)?;
        table.selected_rows().first().copied()
    }

    pub fn can_edit_queue(&self) -> bool {
        self.queue_shown && self.selected().is_some()
    }

    pub fn can_remove_from_queue(&self) -> bool {
        self.can_edit_queue() && self.queue.len() > 1
    }

    pub fn remove_selected(&mut self) {
        if let Some(index) = self.selected() {
            self.remove_from_queue(index);
        }
    }

    pub fn move_selected(&mut self, down: bool) {
        if let Some(index) = self.selected() {
            self.move_in_queue(index, down);
        }
    }

    fn play_at(&mut self, index: usize) {
        if index < self.queue.len() {
            self.remember();
            self.at = index;
            self.pending_load = true;
        }
    }

    fn remove_from_queue(&mut self, index: usize) {
        if index >= self.queue.len() || self.queue.len() < 2 {
            return;
        }
        if index == self.at {
            self.remember();
            self.pending_load = true;
        }
        self.queue.remove(index);
        if index < self.at || self.at == self.queue.len() {
            self.at = self.at.saturating_sub(1);
        }
        self.queue_changed(index.min(self.queue.len() - 1));
    }

    fn move_in_queue(&mut self, index: usize, down: bool) {
        let other = match down {
            true => index + 1,
            false => index.wrapping_sub(1),
        };
        if index >= self.queue.len() || other >= self.queue.len() {
            return;
        }
        self.queue.swap(index, other);
        if self.at == index {
            self.at = other;
        } else if self.at == other {
            self.at = index;
        }
        self.queue_changed(other);
    }

    fn queue_changed(&mut self, selected: usize) {
        self.fill_queue();
        if let Some(table) = self.ui.widget_as::<Table>(self.table) {
            table.select_rows(&[selected]);
        }
        self.menus_changed = true;
    }

    fn take_choices(&mut self) {
        let choices = std::mem::take(&mut *self.choices.borrow_mut());
        for choice in choices {
            match choice {
                Choice::Selected => self.menus_changed = true,
                Choice::Play(index) => self.play_at(index),
                Choice::Menu(action, index) => match action.as_str() {
                    PLAY => self.play_at(index),
                    MOVE_UP => self.move_in_queue(index, false),
                    MOVE_DOWN => self.move_in_queue(index, true),
                    REMOVE => self.remove_from_queue(index),
                    _ => {}
                },
            }
        }
    }

    fn lay_out(&mut self, width: u32, height: u32) {
        let (width, height) = (width as f32, height as f32);
        let queue = match self.queue_shown {
            true => QUEUE_WIDTH.min(width / 2.0),
            false => 0.0,
        };
        let (stage, table) = (self.stage, self.table);
        if let Some(host) = self.ui.widget_as::<FreeLayout>(self.host) {
            host.set_frame(stage, Rect::new(0.0, 0.0, width - queue, height));
            host.set_frame(table, Rect::new(width - queue, 0.0, queue, height));
        }
    }

    fn load(&mut self) {
        let Some(path) = self.queue.get(self.at).cloned() else {
            return;
        };
        let start_ms = self.positions.borrow().start(&path);
        self.open(
            Opening {
                path,
                subtitles: None,
                start_ms,
            },
            true,
        );
        self.fitted = false;
        self.remembered = None;
        self.menus_changed = true;
        self.fill_queue();
    }

    fn open(&mut self, opening: Opening, play: bool) {
        let loaded = self.with_stage(|widget| widget.stage_mut().session_mut().load(opening, play));
        if let Some(Err(error)) = loaded {
            tracing::warn!("cannot start the media engine: {error}");
        }
    }

    pub fn add_subtitles(&mut self, subtitles: PathBuf) {
        let Some(path) = self.queue.get(self.at).cloned() else {
            return;
        };
        let play = self.is_playing();
        let start_ms = self.session().map_or(0, |session| {
            session.position(Instant::now()).as_millis() as u64
        });
        self.open(
            Opening {
                path,
                subtitles: Some(subtitles),
                start_ms,
            },
            play,
        );
        self.remembered = None;
        self.menus_changed = true;
    }

    pub fn has_video(&self) -> bool {
        self.session().is_some_and(Session::has_video)
    }

    fn widget(&self) -> Option<&StageWidget> {
        self.ui.widget_ref(self.stage)
    }

    fn with_stage<R>(&mut self, change: impl FnOnce(&mut StageWidget) -> R) -> Option<R> {
        self.ui.widget_as::<StageWidget>(self.stage).map(change)
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn remember(&self) {
        let (Some(path), Some(session)) = (self.queue.get(self.at), self.session()) else {
            return;
        };
        if session.playback() == Playback::Opening {
            return;
        }
        let length = session.info().duration_ms.map(Duration::from_millis);
        self.positions
            .borrow_mut()
            .remember(path, session.position(Instant::now()), length);
    }

    pub fn add(&mut self, paths: Vec<PathBuf>) {
        self.remember();
        self.queue = paths;
        self.at = 0;
        self.load();
    }

    fn session(&self) -> Option<&Session> {
        self.widget().map(|widget| widget.stage().session())
    }

    pub fn info(&self) -> Option<&MediaInfo> {
        self.session().map(Session::info)
    }

    pub fn is_loaded(&self) -> bool {
        self.session().is_some_and(Session::is_loaded)
    }

    pub fn is_playing(&self) -> bool {
        self.session().is_some_and(Session::is_playing)
    }

    pub fn has_queue(&self) -> bool {
        self.queue.len() > 1
    }

    pub fn loops(&self) -> bool {
        self.looping
    }

    pub fn rate(&self) -> f64 {
        self.session().map_or(1.0, Session::rate)
    }

    pub fn take_menus_changed(&mut self) -> bool {
        std::mem::take(&mut self.menus_changed)
    }

    pub fn toggle(&mut self) {
        self.with_stage(|widget| widget.stage_mut().session_mut().toggle());
    }

    pub fn set_rate(&mut self, rate: f64) {
        self.with_stage(|widget| widget.stage_mut().session_mut().set_rate(rate));
    }

    pub fn toggle_loop(&mut self) {
        self.looping = !self.looping;
    }

    pub fn select_audio(&mut self, id: &str) {
        let subtitles = self.info().and_then(|info| info.selected_subtitles.clone());
        let audio = Some(id.to_owned());
        self.with_stage(|widget| widget.stage().session().select_tracks(audio, subtitles));
    }

    pub fn select_subtitles(&mut self, id: Option<&str>) {
        let audio = self.info().and_then(|info| info.selected_audio.clone());
        let subtitles = id.map(str::to_owned);
        self.with_stage(|widget| widget.stage().session().select_tracks(audio, subtitles));
    }

    pub fn go(&mut self, forward: bool) {
        if self.queue.is_empty() {
            return;
        }
        self.remember();
        let last = self.queue.len() - 1;
        self.at = match (forward, self.at) {
            (true, at) if at == last => 0,
            (true, at) => at + 1,
            (false, 0) => last,
            (false, at) => at - 1,
        };
        self.pending_load = true;
    }

    pub fn remote(&mut self, remote: Remote) {
        match remote {
            Remote::Toggle => self.toggle(),
            Remote::Play => {
                self.with_stage(|widget| widget.stage_mut().session_mut().play());
            }
            Remote::Pause => {
                self.with_stage(|widget| widget.stage_mut().session_mut().pause());
            }
            Remote::Next => self.go(true),
            Remote::Previous => self.go(false),
            Remote::Seek(offset_us) => {
                let now = Instant::now();
                self.with_stage(|widget| {
                    let session = widget.stage_mut().session_mut();
                    let position = session.position(now).as_micros() as i64;
                    let target = position.saturating_add(offset_us).max(0) as u64;
                    session.seek(Duration::from_micros(target), true);
                });
            }
            Remote::SetPosition(position_us) => {
                self.with_stage(|widget| {
                    let position = Duration::from_micros(position_us.max(0) as u64);
                    widget.stage_mut().session_mut().seek(position, true);
                });
            }
            Remote::Raise | Remote::Quit => {}
        }
    }

    pub fn now_playing(&self) -> NowPlaying {
        let path = self.queue.get(self.at);
        let session = self.session();
        let info = session.map(Session::info);
        NowPlaying {
            playback: session.map(Session::playback),
            title: info
                .and_then(|info| info.title.clone())
                .or_else(|| path.map(|path| file_title(path))),
            artist: info.and_then(|info| info.artist.clone()),
            album: info.and_then(|info| info.album.clone()),
            path: path.cloned(),
            duration_ms: info.and_then(|info| info.duration_ms),
            position: session.map_or(Duration::ZERO, |session| session.position(Instant::now())),
            rate: session.map_or(1.0, Session::rate),
            volume: session.map_or(1.0, Session::volume),
            seekable: info.is_some_and(|info| info.seekable),
            has_queue: self.has_queue(),
        }
    }

    fn fit_window(&mut self, window: &mut Window) {
        if self.fitted {
            return;
        }
        let playback = self.session().map(Session::playback);
        let Some(info) = self.info().filter(|_| playback != Some(Playback::Opening)) else {
            return;
        };
        let size = match info.video {
            Some(video) => fitted_size(window, video),
            None if info.has_audio => AUDIO_SIZE,
            None => return,
        };
        self.fitted = true;
        window.resize_content(size, Sizing::Resizable);
    }

    fn follow(&mut self, window: &mut Window) {
        let playback = self.session().map(Session::playback);
        if playback != self.remembered {
            self.remembered = playback;
            self.menus_changed = true;
            if matches!(playback, Some(Playback::Paused | Playback::Ended)) {
                self.remember();
            }
            if playback == Some(Playback::Ended) {
                self.ended();
            }
        }
        if std::mem::take(&mut self.pending_load) {
            self.load();
        }
        if let Some(path) = self.queue.get(self.at) {
            let title = self
                .info()
                .and_then(|info| info.title.clone())
                .unwrap_or_else(|| file_title(path));
            if self.shown_title.as_ref() != Some(&title) {
                window.set_title(&title);
                self.shown_title = Some(title);
            }
        }
        self.fit_window(window);
    }

    fn ended(&mut self) {
        let failed = self
            .session()
            .and_then(Session::failure)
            .is_some_and(|kind| kind != FailureKind::Failed);
        if failed {
            return;
        }
        match (self.at + 1 < self.queue.len(), self.looping) {
            (true, _) => self.go(true),
            (false, true) if self.queue.len() > 1 => self.go(true),
            (false, true) => {
                self.with_stage(|widget| widget.stage_mut().session_mut().play());
            }
            (false, false) => {}
        }
    }
}

impl View for Pane {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.lay_out(width, height);
        Ui::scene(&mut self.ui, width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        match self.drop.event(window, &event, Path::is_file) {
            Dropping::Over => return true,
            Dropping::Dropped(paths) => {
                self.add(paths);
                window.request_redraw();
                return true;
            }
            Dropping::Other => {}
        }
        let double_click = matches!(
            &event,
            WindowEvent::Input(Event::PointerDown(pointer))
                if pointer.count == 2
                    && self.widget().is_some_and(|widget| {
                        widget.frame().contains(pointer.position) && !widget.stage().in_bar(pointer.position)
                    })
        );
        if double_click && self.session().is_some_and(Session::has_video) {
            if let Err(error) = window.toggle_full_screen() {
                tracing::warn!("cannot go full screen: {error}");
            }
            return true;
        }
        let escape = matches!(
            &event,
            WindowEvent::Input(Event::Key {
                key: Key::Escape,
                ..
            })
        );
        if let WindowEvent::Input(Event::Key {
            key: key @ (Key::Up | Key::Down),
            modifiers,
        }) = &event
            && modifiers.alt
            && self.ui.focus() == Some(self.table)
        {
            self.move_selected(*key == Key::Down);
            window.request_redraw();
            return true;
        }
        if escape && window.toplevel_state().fullscreen {
            if let Err(error) = window.toggle_full_screen() {
                tracing::warn!("cannot leave full screen: {error}");
            }
            return true;
        }
        let handled = match window.drive(&mut self.ui, event) {
            Ok(unhandled) => unhandled.is_none(),
            Err(error) => {
                tracing::warn!("{error}");
                false
            }
        };
        self.take_choices();
        let now = Instant::now();
        self.with_stage(|widget| widget.stage_mut().settle(now));
        self.follow(window);
        handled
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.ui.set_window_active(active);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.session().and_then(Session::fd).into_iter().collect()
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) -> bool {
        let changed = self
            .with_stage(|widget| widget.stage_mut().session_mut().ready(window))
            .unwrap_or(false);
        let now = Instant::now();
        self.with_stage(|widget| widget.stage_mut().settle(now));
        self.follow(window);
        changed
    }

    fn deadline(&self) -> Option<Instant> {
        let stage = self.widget().and_then(|widget| widget.stage().deadline());
        [stage, self.ui.deadline()].into_iter().flatten().min()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) -> bool {
        let changed = self
            .with_stage(|widget| widget.stage_mut().tick(now))
            .unwrap_or(false);
        self.follow(window);
        changed
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }
}
