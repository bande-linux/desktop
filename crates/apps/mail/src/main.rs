mod menus;
mod notices;
mod picker;

use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use accesskit::TreeUpdate;
use bande_app::{Cx, Info, Keys, Kind, Menus, Program, Saved, Settings, View, WindowId};
use bande_app_settings::{Stored, settings_folder};
use bande_inbox::Inbox;
use bande_l10n::tr;
use bande_mail::compose::{Draft, Outgoing, from_mailto, keep_unsent, left_unsent};
use bande_mail::message::Attachment;
use bande_mail::portal::choose_save_path;
use bande_mail::settings::{Change, MailSettings, listen};
use bande_mail::ui::{
    COMPOSE_SIZE, ComposeRequest, ComposeView, MailView, Mode, Place, Request, SETTINGS_SIZE,
    SIDEBAR_WIDTH, SettingsView,
};
use bande_toolkit::{
    Appearance, FrameRegions, MenuPlacement, Rect, Scene, TextSystem, Window, WindowError,
    WindowEvent,
};
use bande_xdg::{Apps, config_dirs, data_dirs, data_home, replace_file};

use crate::notices::{Announcer, Clicked, Notices};
use crate::picker::{MAX_ATTACHMENT, Picked, Picker};

const ATTACH_FLAG: &str = "--attach";
const SHARED_FILES: u64 = u64::MAX;
const APP_ID: &str = "org.bande.Mail";
const WINDOW_SIZE: (u32, u32) = (1180, 720);

static INFO: Info = Info {
    id: APP_ID,
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || tr!("Mail"),
    keys: Keys::Standard,
    settings: Some(Settings {
        title: || tr!("Mail Settings"),
        size: SETTINGS_SIZE,
        view: settings_window,
    }),
};

fn settings_window(appearance: Appearance) -> Box<dyn View> {
    Box::new(SettingsView::new(
        store_path(),
        appearance,
        TextSystem::new(),
    ))
}

fn load_settings() -> MailSettings {
    settings_folder()
        .as_deref()
        .map(MailSettings::load)
        .unwrap_or_default()
}

enum Pane {
    Main(Box<MailView>),
    Compose(Box<ComposeView>),
}

impl View for Pane {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let full_screen = window.header_height() == 0;
        let (width, height) = window.size();
        match self {
            Self::Main(view) => {
                window.set_frame_regions(FrameRegions {
                    sidebar: SIDEBAR_WIDTH,
                    toolbar: 0.0,
                });
                view.set_full_screen(full_screen);
                let scene = view.scene(width, height);
                window.set_header(view.header());
                scene
            }
            Self::Compose(view) => {
                window.set_frame_regions(FrameRegions {
                    sidebar: 0.0,
                    toolbar: 0.0,
                });
                let scene = view.scene(width, height);
                window.set_header(view.header());
                scene
            }
        }
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        match (self, event) {
            (Self::Compose(view), event @ WindowEvent::Drag(_)) => view.dragged(window, &event),
            (Self::Main(view), WindowEvent::Input(input)) => view.handle(&input),
            (Self::Compose(view), WindowEvent::Input(input)) => view.handle(&input),
            (Self::Main(view), WindowEvent::Accessibility(request)) => view.perform(&request),
            (Self::Compose(view), WindowEvent::Accessibility(request)) => view.perform(&request),
            (Self::Main(view), WindowEvent::Pasted(text)) => view.pasted(&text),
            (Self::Compose(view), WindowEvent::Pasted(text)) => view.pasted(&text),
            (Self::Main(view), WindowEvent::MenuAction(action)) => view.menu_action(&action),
            (Self::Compose(view), WindowEvent::MenuAction(action)) => view.menu_action(&action),
            (Self::Main(view), WindowEvent::MenuClosed) => view.menu_closed(),
            (Self::Compose(view), WindowEvent::MenuClosed) => view.menu_closed(),
            _ => false,
        }
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        match self {
            Self::Main(view) => view.set_appearance(appearance),
            Self::Compose(view) => view.set_appearance(appearance),
        }
    }

    fn set_active(&mut self, active: bool) {
        if let Self::Main(view) = self {
            view.set_window_active(active);
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        match self {
            Self::Main(view) => view.fds(),
            Self::Compose(_) => Vec::new(),
        }
    }

    fn source_ready(&mut self, _window: &mut Window, _index: usize) -> bool {
        match self {
            Self::Main(view) => view.engine_ready(),
            Self::Compose(_) => false,
        }
    }

    fn deadline(&self) -> Option<Instant> {
        match self {
            Self::Main(view) => view.deadline(),
            Self::Compose(view) => view.deadline(),
        }
    }

    fn tick(&mut self, _window: &mut Window, now: Instant) -> bool {
        if let Self::Compose(view) = self {
            view.autosave(now);
        }
        true
    }

    fn animating(&self) -> bool {
        match self {
            Self::Main(view) => view.animating(),
            Self::Compose(view) => view.animating(),
        }
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        match self {
            Self::Main(view) => Some(view.accessibility()),
            Self::Compose(view) => Some(view.accessibility()),
        }
    }

    fn text_input_area(&self) -> Option<Rect> {
        match self {
            Self::Main(view) => view.text_input_area(),
            Self::Compose(view) => view.text_input_area(),
        }
    }
}

struct Mail {
    main: Option<WindowId>,
    parked: Option<Box<MailView>>,
    apps: Option<Apps>,
    picker: Picker,
    changes: Inbox<Change>,
    notices: Notices,
}

fn store_path() -> PathBuf {
    data_home()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("bande-mail/mail.sqlite3")
}

fn save_attachment(name: String, data: Vec<u8>, announcer: Announcer) {
    let spawned = std::thread::Builder::new()
        .name("save-attachment".to_owned())
        .spawn(
            move || match choose_save_path(&tr!("Save Attachment"), &name) {
                Ok(Some(path)) => {
                    if let Err(error) = replace_file(&path, &data) {
                        tracing::warn!("cannot save {}: {error}", path.display());

                        announcer.announce(
                            tr!(
                                "The attachment couldn’t be saved: {error}",
                                error = error.to_string()
                            ),
                            name,
                            false,
                            None,
                        );
                    }
                }
                Ok(None) => {}
                Err(error) => tracing::warn!("cannot choose where to save {name}: {error}"),
            },
        );
    if let Err(error) = spawned {
        tracing::warn!("cannot save an attachment: {error}");
    }
}

fn keep_locally(outgoing: &Outgoing) {
    if let Err(error) = keep_unsent(outgoing) {
        tracing::warn!("cannot keep the unsent message: {error}");
    }
}

fn main_view<'a>(
    cx: &'a mut Cx<Mail>,
    main: Option<WindowId>,
    parked: &'a mut Option<Box<MailView>>,
) -> Option<&'a mut MailView> {
    match main.and_then(|main| cx.window(main)) {
        Some((_, Pane::Main(view))) => Some(view),
        _ => parked.as_deref_mut(),
    }
}

fn show_main(cx: &mut Cx<Mail>, mut view: Box<MailView>) -> Result<WindowId, WindowError> {
    let title = view.title();
    cx.open(Kind::Main, &title, WINDOW_SIZE, |appearance| {
        view.set_appearance(appearance);
        Pane::Main(view)
    })
}

impl Mail {
    fn apps(&mut self) -> &Apps {
        self.apps
            .get_or_insert_with(|| Apps::load(data_dirs(), &config_dirs()))
    }

    fn open_compose(&mut self, cx: &mut Cx<Self>, outgoing: Outgoing) -> Option<WindowId> {
        let (senders, bcc_field, known) = main_view(cx, self.main, &mut self.parked)
            .map(|view| {
                (
                    view.accounts(),
                    view.settings().bcc_field,
                    view.correspondents(),
                )
            })
            .unwrap_or_default();
        let mut view = ComposeView::new(
            outgoing,
            senders,
            bcc_field,
            known,
            Appearance::default().palette(),
            TextSystem::new(),
        );
        let title = view.title();
        let opened = cx.open(
            Kind::Secondary("compose"),
            &title,
            COMPOSE_SIZE,
            |appearance| {
                view.set_appearance(appearance);
                Pane::Compose(Box::new(view))
            },
        );
        opened
            .inspect_err(|error| tracing::warn!("cannot open a message window: {error}"))
            .ok()
    }

    fn open_arguments(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>) {
        if arguments.first().is_some_and(|flag| flag == ATTACH_FLAG) {
            let paths = arguments.into_iter().skip(1).map(PathBuf::from).collect();
            self.picker.read(SHARED_FILES, paths);
            return;
        }
        for argument in arguments {
            self.open_argument(cx, &argument);
        }
    }

    fn open_argument(&mut self, cx: &mut Cx<Self>, argument: &str) {
        if let Some(message) = argument
            .strip_prefix(bande_mail::offer::MESSAGE_ARGUMENT)
            .and_then(|message| message.parse().ok())
        {
            if let Some(view) = main_view(cx, self.main, &mut self.parked) {
                view.show_message(message);
            }
            self.serve_main(cx);
            return;
        }
        if let Some(draft) = from_mailto(argument) {
            if let Some(view) = main_view(cx, self.main, &mut self.parked) {
                view.compose(draft);
            }
            self.serve_main(cx);
        }
    }

    fn open_url(&mut self, cx: &mut Cx<Self>, window: WindowId, url: &str) {
        if from_mailto(url).is_some() {
            self.open_argument(cx, url);
            return;
        }
        self.open_target(cx, window, url);
    }

    fn open_target(&mut self, cx: &mut Cx<Self>, window: WindowId, target: &str) {
        let token = cx.window(window).and_then(|(window, _)| {
            window
                .activation_token(APP_ID)
                .map_err(|error| tracing::warn!("cannot get an activation token: {error}"))
                .ok()
        });
        if let Err(error) = bande_xdg::open_with(self.apps(), target, token.as_deref()) {
            tracing::warn!("cannot open {target}: {error}");
        }
    }

    fn attach(&mut self, cx: &mut Cx<Self>, token: u64, picked: Vec<Picked>) {
        let mut attachments = Vec::new();
        let mut refused = Vec::new();
        for item in picked {
            match item {
                Picked::File(path, data) => {
                    let mime = self.apps().mime_for(&path);
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "attachment".to_owned());
                    attachments.push(Attachment { name, mime, data });
                }
                Picked::TooLarge(path) => refused.push(path),
            }
        }
        if token == SHARED_FILES {
            if let Some(path) = refused.first() {
                tracing::warn!("{} is too large to attach", path.display());
            }
            if let Some(view) = main_view(cx, self.main, &mut self.parked) {
                view.compose(Draft {
                    attachments,
                    ..Draft::default()
                });
            }
            self.serve_main(cx);
            return;
        }
        let windows: Vec<WindowId> = cx.windows().collect();
        let Some(window) = windows.into_iter().find(
            |window| matches!(cx.view(*window), Some(Pane::Compose(view)) if view.token() == token),
        ) else {
            return;
        };
        if let Some((_, Pane::Compose(view))) = cx.window(window) {
            view.add_attachments(attachments);
            if let Some(path) = refused.first() {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                view.show_error(&tr!(
                    "{name} is larger than {limit} and was not attached.",
                    name = name,
                    limit = bande_fs::format_size(MAX_ATTACHMENT)
                ));
            }
        }
        refresh(cx, window);
    }

    fn serve_compose(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        let Some((shown, Pane::Compose(view))) = cx.window(window) else {
            return;
        };
        let requests = view.take_requests();
        let sent = view.take_sent();
        let discarded = view.take_discarded();
        let finished = view.finished();
        let token = view.token();
        for request in requests {
            let result = match request {
                ComposeRequest::ChooseFiles => {
                    self.picker.pick(token, tr!("Attach Files"));
                    Ok(())
                }
                ComposeRequest::Attach(paths) => {
                    self.picker.read(token, paths);
                    Ok(())
                }
                ComposeRequest::ShowMenu(menu, at) => {
                    shown.show_menu(menu, MenuPlacement::AtPoint(at))
                }
                ComposeRequest::Clipboard(request) => shown.serve_clipboard(request),
            };
            if let Err(error) = result {
                tracing::warn!("cannot serve the message window: {error}");
            }
        }
        if let Some(outgoing) = sent {
            keep_locally(&outgoing);
            let delivered =
                main_view(cx, self.main, &mut self.parked).is_some_and(|main| main.send(outgoing));
            if !delivered {
                tracing::warn!("no account sends from this address");
            }
        }
        if let Some(draft) = discarded
            && let Some(main) = main_view(cx, self.main, &mut self.parked)
        {
            main.discard_draft(&draft);
        }
        if finished {
            self.keep_draft(cx, window);
            cx.close(window);
        } else {
            refresh(cx, window);
        }
    }

    fn serve_main(&mut self, cx: &mut Cx<Self>) {
        if self.parked.as_ref().is_some_and(|view| view.has_sheet()) {
            self.show_viewer(cx);
        }
        if let Some(main) = self.main {
            return self.serve(cx, main);
        }
        let requests = self
            .parked
            .as_mut()
            .map(|view| view.take_requests())
            .unwrap_or_default();
        for request in requests {
            match request {
                Request::Compose(outgoing) => {
                    self.open_compose(cx, *outgoing);
                }
                Request::Notify {
                    summary,
                    body,
                    sound,
                    message,
                } => self
                    .notices
                    .announcer()
                    .announce(summary, body, sound, message),
                Request::NotSent { error, outgoing } => {
                    if let Some(opened) = self.open_compose(cx, *outgoing)
                        && let Some((_, Pane::Compose(view))) = cx.window(opened)
                    {
                        view.show_error(&error);
                    }
                }
                _ => {}
            }
        }
        self.show_unread(cx);
    }

    fn show_unread(&mut self, cx: &mut Cx<Self>) {
        let unread =
            main_view(cx, self.main, &mut self.parked).map_or(0, |view| view.unread_inbox());
        self.notices
            .set_badge(u32::try_from(unread).unwrap_or(u32::MAX));
    }

    fn open_clicked(&mut self, cx: &mut Cx<Self>, clicked: Clicked) {
        if let Some(message) = clicked.message
            && let Some(view) = main_view(cx, self.main, &mut self.parked)
        {
            view.show_message(message);
        }
        match self.main {
            Some(main) => {
                cx.raise(main, clicked.token);
                self.serve(cx, main);
            }
            None => self.reopen(cx),
        }
    }

    fn serve(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        let requests = match cx.window(window) {
            Some((_, Pane::Main(view))) => view.take_requests(),
            Some((_, Pane::Compose(_))) => return self.serve_compose(cx, window),
            None => return,
        };
        for request in requests {
            match request {
                Request::Compose(outgoing) => {
                    self.open_compose(cx, *outgoing);
                }
                Request::OpenSettings => cx.open_settings(),
                Request::OpenLink(url) => self.open_url(cx, window, &url),
                Request::OpenAttachment(path) => {
                    self.open_target(cx, window, &path.to_string_lossy())
                }
                Request::SaveAttachment(name, data) => {
                    save_attachment(name, data, self.notices.announcer().clone())
                }
                Request::Notify {
                    summary,
                    body,
                    sound,
                    message,
                } => self
                    .notices
                    .announcer()
                    .announce(summary, body, sound, message),
                Request::NotSent { error, outgoing } => {
                    if let Some(opened) = self.open_compose(cx, *outgoing)
                        && let Some((_, Pane::Compose(view))) = cx.window(opened)
                    {
                        view.show_error(&error);
                    }
                }
                Request::ShowMenu(menu, at) => {
                    if let Some((shown, _)) = cx.window(window)
                        && let Err(error) = shown.show_menu(menu, MenuPlacement::AtPoint(at))
                    {
                        tracing::warn!("cannot show the menu: {error}");
                    }
                }
                Request::Clipboard(request) => {
                    if let Some((shown, _)) = cx.window(window)
                        && let Err(error) = shown.serve_clipboard(request)
                    {
                        tracing::warn!("cannot use the clipboard: {error}");
                    }
                }
            }
        }
        refresh(cx, window);
        self.show_unread(cx);
    }

    fn keep_draft(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        let unsent = match cx.window(window) {
            Some((_, Pane::Compose(view))) => {
                let unsent = view.unsent();
                if unsent.is_none() {
                    view.discard_unsent();
                }
                unsent
            }
            _ => None,
        };
        let Some(outgoing) = unsent else {
            return;
        };
        keep_locally(&outgoing);
        if !main_view(cx, self.main, &mut self.parked).is_some_and(|main| main.save_draft(outgoing))
        {
            tracing::warn!("no account keeps drafts for this address");
        }
    }

    fn show_viewer(&mut self, cx: &mut Cx<Self>) {
        if self.main.is_none() {
            return self.reopen(cx);
        }
        if let Some((window, _)) = self.main.and_then(|main| cx.window(main))
            && let Err(error) = window
                .activation_token(APP_ID)
                .and_then(|token| window.activate(&token))
        {
            tracing::warn!("cannot raise the viewer: {error}");
        }
    }

    fn apply(&mut self, cx: &mut Cx<Self>, change: Change) {
        if change == Change::AddAccount {
            self.show_viewer(cx);
        }
        let Some(view) = main_view(cx, self.main, &mut self.parked) else {
            return;
        };
        match change {
            Change::Settings => view.set_settings(load_settings()),
            Change::Accounts => view.accounts_changed(),
            Change::Remove(id) => view.remove_account(id),
            Change::AddAccount => view.present_setup(Mode::Kind),
        }
        if let Some(main) = self.main {
            self.serve(cx, main);
        }
    }

    fn finish(&mut self, cx: &mut Cx<Self>) {
        let windows: Vec<WindowId> = cx.windows().collect();
        for window in windows {
            self.keep_draft(cx, window);
        }
        if let Some(view) = main_view(cx, self.main, &mut self.parked) {
            view.finish();
        }
    }
}

fn refresh(cx: &mut Cx<Mail>, window: WindowId) {
    if let Some((shown, pane)) = cx.window(window) {
        let (cursor, title) = match pane {
            Pane::Main(view) => (view.cursor(), view.title()),
            Pane::Compose(view) => (view.cursor(), view.title()),
        };
        shown.set_cursor(cursor);
        shown.set_title(&title);
        shown.request_redraw();
    }
    cx.refresh_menus();
}

impl Program for Mail {
    type View = Pane;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let mut view = MailView::new(
            store_path(),
            load_settings(),
            Appearance::default().palette(),
            TextSystem::new(),
        )?;
        let changes = Inbox::new()?;
        listen(changes.outbox());
        view.start();
        let (main, parked) = match arguments.is_empty() {
            true => (None, Some(Box::new(view))),
            false => (Some(show_main(cx, Box::new(view))?), None),
        };
        let mut mail = Self {
            main,
            parked,
            apps: None,
            picker: Picker::new()?,
            changes,
            notices: Notices::start(APP_ID, cx.session())?,
        };
        mail.open_arguments(cx, arguments);
        if let Some(main) = main {
            mail.serve(cx, main);
        }
        for outgoing in left_unsent() {
            let outgoing = match main_view(cx, mail.main, &mut mail.parked) {
                Some(view) => view.recover(outgoing),
                None => outgoing,
            };
            mail.open_compose(cx, outgoing);
        }
        mail.show_unread(cx);
        Ok(mail)
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        let view = match self.main.and_then(|main| cx.view(main)) {
            Some(Pane::Main(view)) => Some(view.as_ref()),
            _ => self.parked.as_deref(),
        };
        let windowed = cx.windows().next().is_some();
        let composing = cx
            .focused()
            .is_some_and(|window| matches!(cx.view(window), Some(Pane::Compose(_))));
        Menus {
            menus: view
                .map(|view| menus::app_menus(view, self.main.is_some(), windowed, composing))
                .unwrap_or_default(),
            window: menus::window_menu(),
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, window: Option<WindowId>, command: &str) {
        if command == menus::VIEWER {
            return self.show_viewer(cx);
        }
        if command == bande_app_menu::PRINT {
            let printable =
                main_view(cx, self.main, &mut self.parked).and_then(|view| view.printable());
            if let Some((title, text)) = printable {
                cx.print(&title, bande_app::text_document(text, false));
            }
            return;
        }
        let handled = match window.and_then(|window| cx.window(window)) {
            Some((_, Pane::Main(view))) => view.menu_action(command),
            Some((_, Pane::Compose(view))) => view.menu_action(command),
            None => false,
        };
        if !handled && (window.is_none() || window != self.main) {
            if let Some(view) = main_view(cx, self.main, &mut self.parked) {
                view.menu_action(command);
            }
            self.serve_main(cx);
        }
        if command == "bcc-field" {
            let shown = main_view(cx, self.main, &mut self.parked)
                .is_some_and(|view| view.settings().bcc_field);
            let windows: Vec<WindowId> = cx.windows().collect();
            for window in windows {
                if let Some((_, Pane::Compose(view))) = cx.window(window) {
                    view.set_bcc_field(shown);
                    refresh(cx, window);
                }
            }
        }
        if let Some(window) = window {
            self.serve(cx, window);
        }
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, _directory: &Path) {
        self.open_arguments(cx, arguments);
    }

    fn changed(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        self.serve(cx, window);
    }

    fn should_close(&mut self, cx: &mut Cx<Self>, window: WindowId) -> bool {
        if Some(window) != self.main {
            self.keep_draft(cx, window);
        }
        true
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        let Some(view) = self.parked.take() else {
            return;
        };
        match show_main(cx, view) {
            Ok(main) => {
                self.main = Some(main);
                self.serve(cx, main);
            }
            Err(error) => tracing::warn!("cannot open the viewer: {error}"),
        }
    }

    fn save_window(&self, cx: &Cx<Self>, window: WindowId) -> Option<Saved> {
        match cx.view(window)? {
            Pane::Main(view) => Saved::of(&view.place().key()),
            Pane::Compose(_) => None,
        }
    }

    fn restore_window(&mut self, cx: &mut Cx<Self>, saved: Saved) {
        if let Some(place) = saved.read::<String>().and_then(|key| Place::from_key(&key))
            && let Some(view) = self.parked.as_deref_mut()
        {
            view.choose_place(place);
        }
        self.reopen(cx);
    }

    fn window_closed(&mut self, _cx: &mut Cx<Self>, window: WindowId, view: Pane) {
        if let Pane::Main(view) = view
            && self.main == Some(window)
        {
            self.main = None;
            self.parked = Some(view);
        }
    }

    fn should_quit(&mut self, cx: &mut Cx<Self>) -> bool {
        self.finish(cx);
        true
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        let mut sources = vec![self.changes.fd(), self.picker.fd(), self.notices.fd()];
        if let Some(view) = &self.parked {
            sources.extend(view.fds());
        }
        sources
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, index: usize) {
        if index == 0 {
            for change in self.changes.drain() {
                self.apply(cx, change);
            }
            return;
        }
        if index == 2 {
            for clicked in self.notices.clicked() {
                self.open_clicked(cx, clicked);
            }
            return;
        }
        if index > 2 {
            if let Some(view) = &mut self.parked {
                view.engine_ready();
            }
            return self.serve_main(cx);
        }
        for (token, picked) in self.picker.take() {
            self.attach(cx, token, picked);
        }
    }
}

fn main() -> ExitCode {
    bande_app::run::<Mail>(&INFO)
}
