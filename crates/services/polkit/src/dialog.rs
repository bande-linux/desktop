use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use accesskit::{Action, ActionData, ActionRequest, Node, NodeId, Role, TreeId};
use bande_toolkit::ui::{
    Animated, BUTTON_HEIGHT, Button, Context, Event, FreeLayout, Key, Modifiers, PopUpButton,
    SecureField, Size, Spring, Ui, Widget, WidgetId,
};
use bande_toolkit::{
    Appearance, Color, Item, Paragraph, Path, Point, Radius, Rect, Scene, Shape, TextAlign,
    TextRole, TextStyle, TextSystem,
};
use zeroize::Zeroizing;

use crate::L10N;
use crate::agent::{Answer, Identity, Notice};

const WIDTH: f32 = 400.0;
const PADDING: f32 = 24.0;
const ICON_SIZE: f32 = 36.0;
const ICON_GAP: f32 = 16.0;
const TITLE_GAP: f32 = 6.0;
const FIELD_GAP: f32 = 16.0;
const NOTE_GAP: f32 = 8.0;
const FOOTER_PADDING: f32 = 14.0;
const FOOTER_INSET: f32 = 16.0;
const BUTTON_SPACING: f32 = 8.0;
const BUTTON_MIN_WIDTH: f32 = 96.0;
const FIELD_FONT: f32 = 13.0;
const SHAKE_DISTANCE: f32 = 14.0;
const SHACKLE_WIDTH: f32 = 3.0;
const AVATAR_SIZE: f32 = 24.0;
const AVATAR_GAP: f32 = 8.0;
const AVATAR_FONT: f32 = 12.0;
const FIELD: usize = 0;
const AUTHENTICATE: usize = 2;
const PICKER: usize = 3;

pub type Password = Zeroizing<String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub message: String,
    pub identities: Vec<Identity>,
    pub user: String,
}

impl Prompt {
    fn name(&self) -> &str {
        self.identities
            .iter()
            .find(|identity| identity.login == self.user)
            .map_or(self.user.as_str(), |identity| identity.name.as_str())
    }

    fn asking(&self) -> String {
        L10N.tr_args("enter-password", [("user", self.name().into())])
    }
}

pub enum Message {
    Prompt {
        prompt: Prompt,
        reply: Sender<Answer>,
    },
    Notice(Notice),
    Close,
}

enum Choice {
    Authenticate(Password),
    SwitchTo(String),
    Cancel,
}

#[derive(Default)]
struct Shared {
    choices: Vec<Choice>,
    submit: bool,
    closed: bool,
}

#[derive(Clone, PartialEq, Eq)]
enum Note {
    Info(String),
    Error(String),
}

struct Authorization {
    id: WidgetId,
    prompt: Prompt,
    note: Option<Note>,
    shared: Rc<RefCell<Shared>>,
    children: Vec<Box<dyn Widget>>,
    shake: Animated,
    title: Option<Paragraph>,
    message: Option<Paragraph>,
    initial: Option<Paragraph>,
    picker_height: f32,
    note_text: Option<Paragraph>,
    note_height: f32,
    button_sizes: Vec<Size>,
    frame: Rect,
}

impl Authorization {
    fn new(prompt: Prompt, shared: Rc<RefCell<Shared>>) -> (Self, WidgetId) {
        let submitted = Rc::clone(&shared);
        let field = SecureField::new(&L10N.tr("password"), FIELD_FONT, move |secret| {
            submitted
                .borrow_mut()
                .choices
                .push(Choice::Authenticate(Zeroizing::new(secret.to_owned())));
        });
        let field_id = field.id();
        let cancelled = Rc::clone(&shared);
        let cancel = Button::new(&L10N.tr("cancel"), move || {
            let mut shared = cancelled.borrow_mut();
            shared.choices.push(Choice::Cancel);
            shared.closed = true;
        })
        .cancel();
        let requested = Rc::clone(&shared);
        let authenticate = Button::new(&L10N.tr("authenticate"), move || {
            requested.borrow_mut().submit = true;
        })
        .primary();
        let mut children: Vec<Box<dyn Widget>> =
            vec![Box::new(field), Box::new(cancel), Box::new(authenticate)];
        if prompt.identities.len() > 1 {
            let names: Vec<&str> = prompt
                .identities
                .iter()
                .map(|identity| identity.name.as_str())
                .collect();
            let logins: Vec<String> = prompt
                .identities
                .iter()
                .map(|identity| identity.login.clone())
                .collect();
            let selected = logins
                .iter()
                .position(|login| *login == prompt.user)
                .unwrap_or_default();
            let switched = Rc::clone(&shared);
            children.push(Box::new(PopUpButton::new(
                &L10N.tr("user"),
                &names,
                selected,
                move |index| {
                    if let Some(login) = logins.get(index) {
                        switched
                            .borrow_mut()
                            .choices
                            .push(Choice::SwitchTo(login.clone()));
                    }
                },
            )));
        }
        let dialog = Self {
            id: WidgetId::next(),
            prompt,
            note: None,
            shared,
            children,
            shake: Animated::new(0.0),
            title: None,
            message: None,
            initial: None,
            picker_height: 0.0,
            note_text: None,
            note_height: 0.0,
            button_sizes: Vec::new(),
            frame: Rect::default(),
        };
        (dialog, field_id)
    }

    fn text_x(&self) -> f32 {
        PADDING + ICON_SIZE + ICON_GAP
    }

    fn footer_height() -> f32 {
        BUTTON_HEIGHT + 2.0 * FOOTER_PADDING
    }

    fn shake(&mut self, cx: &Context) {
        self.shake.jump_to(SHAKE_DISTANCE);
        cx.animate(&mut self.shake, 0.0, Spring::SHAKE);
    }

    fn paint_lock(&self, cx: &Context, items: &mut Vec<Item>) {
        let x = self.frame.x + PADDING;
        let y = self.frame.y + PADDING;
        let at = |dx: f32, dy: f32| Point::new(x + dx, y + dy);
        items.push(Item::StrokePath {
            path: Path::new()
                .move_to(at(10.0, 17.0))
                .line_to(at(10.0, 11.0))
                .cubic_to(at(10.0, 1.5), at(26.0, 1.5), at(26.0, 11.0))
                .line_to(at(26.0, 17.0)),
            width: SHACKLE_WIDTH,
            paint: cx.palette.ink2.into(),
        });
        items.push(Item::Fill {
            shape: Shape::rounded(Rect::new(x + 4.0, y + 15.0, 28.0, 21.0), Radius::S),
            paint: cx.palette.accent.into(),
        });
        items.push(Item::Fill {
            shape: Shape::rounded(Rect::new(x + 16.5, y + 21.5, 3.0, 8.0), 1.5),
            paint: cx.palette.on_accent.into(),
        });
    }
}

impl Widget for Authorization {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let width = WIDTH.min(available.width);
        let text_width = width - self.text_x() - PADDING;
        let title = cx.text.paragraph(
            &self.prompt.message,
            TextStyle::role(TextRole::Heading, cx.palette.ink),
            Some(text_width),
            TextAlign::Start,
        );
        let message = cx.text.paragraph(
            &self.prompt.asking(),
            TextStyle::role(TextRole::Body, cx.palette.ink2),
            Some(text_width),
            TextAlign::Start,
        );
        let initial: String = self
            .prompt
            .name()
            .chars()
            .next()
            .map(|first| first.to_uppercase().collect())
            .unwrap_or_default();
        self.initial = Some(cx.text.paragraph(
            &initial,
            TextStyle::new(AVATAR_FONT, cx.palette.on_accent).weight(TextStyle::SEMIBOLD),
            Some(AVATAR_SIZE),
            TextAlign::Center,
        ));
        self.picker_height = self.children.get_mut(PICKER).map_or(0.0, |picker| {
            let room = Size::new(text_width - AVATAR_SIZE - AVATAR_GAP, available.height);
            picker.measure(cx, room).height.max(AVATAR_SIZE) + FIELD_GAP
        });
        let field = self.children.get_mut(FIELD).map_or(0.0, |field| {
            field
                .measure(cx, Size::new(text_width, available.height))
                .height
        });
        let reserved = cx
            .text
            .paragraph(
                " ",
                TextStyle::role(TextRole::Callout, cx.palette.ink2),
                Some(text_width),
                TextAlign::Start,
            )
            .height();
        let note = self.note.as_ref().map(|note| {
            let (text, color) = match note {
                Note::Info(text) => (text.as_str(), cx.palette.ink2),
                Note::Error(text) => (text.as_str(), cx.palette.danger_text),
            };
            cx.text.paragraph(
                text,
                TextStyle::role(TextRole::Callout, color),
                Some(text_width),
                TextAlign::Start,
            )
        });
        self.note_height = note
            .as_ref()
            .map_or(reserved, Paragraph::height)
            .max(reserved);
        self.button_sizes = self
            .children
            .iter_mut()
            .take(PICKER)
            .skip(1)
            .map(|button| {
                let size = button.measure(cx, available);
                Size::new(size.width.max(BUTTON_MIN_WIDTH), BUTTON_HEIGHT)
            })
            .collect();
        let body = title.height()
            + TITLE_GAP
            + message.height()
            + FIELD_GAP
            + self.picker_height
            + field
            + NOTE_GAP
            + self.note_height;
        self.title = Some(title);
        self.message = Some(message);
        self.note_text = note;
        let height = PADDING + body.max(ICON_SIZE) + PADDING + Self::footer_height();
        Size::new(width, height.ceil())
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        let offset = cx.sample(&self.shake).round();
        self.frame = Rect::new(frame.x + offset, frame.y, frame.width, frame.height);
        let frame = self.frame;
        let x = frame.x + self.text_x();
        let width = frame.width - self.text_x() - PADDING;
        let y = frame.y
            + PADDING
            + self.title.as_ref().map_or(0.0, Paragraph::height)
            + TITLE_GAP
            + self.message.as_ref().map_or(0.0, Paragraph::height)
            + FIELD_GAP;
        if let Some(picker) = self.children.get_mut(PICKER) {
            let row = self.picker_height - FIELD_GAP;
            let top = y + ((row - BUTTON_HEIGHT) / 2.0).round();
            let left = x + AVATAR_SIZE + AVATAR_GAP;
            picker.place(
                cx,
                Rect::new(left, top, width - AVATAR_SIZE - AVATAR_GAP, 0.0),
            );
        }
        if let Some(field) = self.children.get_mut(FIELD) {
            field.place(cx, Rect::new(x, y + self.picker_height, width, 0.0));
        }
        let top = frame.y + frame.height - FOOTER_PADDING - BUTTON_HEIGHT;
        let mut right = frame.x + frame.width - FOOTER_INSET;
        for (button, size) in self
            .children
            .iter_mut()
            .take(PICKER)
            .skip(1)
            .zip(&self.button_sizes)
            .rev()
        {
            right -= size.width;
            button.place(cx, Rect::new(right, top, size.width, size.height));
            right -= BUTTON_SPACING;
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let footer = Self::footer_height();
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(
                self.frame.x,
                self.frame.y + self.frame.height - footer,
                self.frame.width,
                footer,
            )),
            paint: cx.palette.chassis.into(),
        });
        self.paint_lock(cx, items);
        let x = self.frame.x + self.text_x();
        let mut y = self.frame.y + PADDING;
        for paragraph in [&self.title, &self.message].into_iter().flatten() {
            items.push(Item::Text {
                origin: Point::new(x, y),
                paragraph: paragraph.clone(),
            });
            y += paragraph.height() + TITLE_GAP;
        }
        if let (Some(picker), Some(initial)) = (self.children.get(PICKER), &self.initial) {
            let row = picker.frame();
            let top = row.y + ((row.height - AVATAR_SIZE) / 2.0).round();
            items.push(Item::Fill {
                shape: Shape::rounded(
                    Rect::new(x, top, AVATAR_SIZE, AVATAR_SIZE),
                    AVATAR_SIZE / 2.0,
                ),
                paint: cx.palette.accent.into(),
            });
            items.push(Item::Text {
                origin: Point::new(x, top + ((AVATAR_SIZE - initial.height()) / 2.0).round()),
                paragraph: initial.clone(),
            });
        }
        if let (Some(note), Some(field)) = (&self.note_text, self.children.get(FIELD)) {
            let field = field.frame();
            items.push(Item::Text {
                origin: Point::new(x, field.y + field.height + NOTE_GAP),
                paragraph: note.clone(),
            });
        }
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::AlertDialog);
        node.set_label(self.prompt.message.clone());
        let description = match &self.note {
            Some(Note::Info(text) | Note::Error(text)) => text.clone(),
            None => self.prompt.asking(),
        };
        node.set_description(description);
        node.set_modal();
        node
    }

    fn dismissed(&self) -> bool {
        self.shared.borrow().closed
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

pub struct Dialog {
    ui: Ui,
    shared: Rc<RefCell<Shared>>,
    dialog: WidgetId,
    field: WidgetId,
    reply: Option<Sender<Answer>>,
    switch_to: Option<String>,
    declined: bool,
}

impl Dialog {
    pub fn new(
        prompt: Prompt,
        reply: Sender<Answer>,
        text: TextSystem,
        appearance: Appearance,
    ) -> Self {
        let mut ui = Ui::new(
            Box::new(FreeLayout::new()),
            Context::new(text, appearance.palette()),
        );
        ui.set_appearance(appearance);
        let shared = Rc::default();
        let (authorization, field) = Authorization::new(prompt, Rc::clone(&shared));
        let dialog = authorization.id();
        ui.present_dialog(authorization);
        Self {
            ui,
            shared,
            dialog,
            field,
            reply: Some(reply),
            switch_to: None,
            declined: false,
        }
    }

    pub fn ui(&self) -> &Ui {
        &self.ui
    }

    pub fn ui_mut(&mut self) -> &mut Ui {
        &mut self.ui
    }

    pub fn closing(&self) -> bool {
        self.shared.borrow().closed
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
        let mut scene = self.ui.scene(width, height);
        scene.background = Color::TRANSPARENT;
        scene
    }

    pub fn finished(&self) -> bool {
        !self.ui.has_sheet() && !self.ui.is_animating()
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        let changed = self.ui.handle(event);
        self.settle_choices() || changed
    }

    pub fn perform(&mut self, request: &ActionRequest) -> bool {
        let changed = self.ui.perform(request);
        self.settle_choices() || changed
    }

    pub fn settle_choices(&mut self) -> bool {
        let submit = std::mem::take(&mut self.shared.borrow_mut().submit);
        let changed = submit && {
            self.ui.set_focus(self.field);
            self.ui.handle(&Event::Key {
                key: Key::Enter,
                modifiers: Modifiers::default(),
            })
        };
        self.answer();
        changed
    }

    pub fn receive(&mut self, message: Message) {
        match message {
            Message::Prompt { prompt, reply } => {
                if let Some(login) = self.switch_to.take()
                    && login != prompt.user
                {
                    let _ = reply.send(Answer::SwitchTo(login));
                    return;
                }
                self.ui
                    .update_widget::<Authorization>(self.dialog, |dialog, _| {
                        dialog.prompt = prompt
                    });
                self.reply = Some(reply);
                self.set_waiting(false);
                self.answer();
            }
            Message::Notice(Notice::WrongPassword) => {
                self.set_note(Some(Note::Error(L10N.tr("wrong-password"))));
                self.clear_field();
                self.ui
                    .update_widget::<Authorization>(self.dialog, |dialog, cx| dialog.shake(cx));
            }
            Message::Notice(Notice::Info(text)) => self.set_note(Some(Note::Info(text))),
            Message::Notice(Notice::Error(text)) => self.set_note(Some(Note::Error(text))),
            Message::Close => {
                self.shared.borrow_mut().closed = true;
                self.ui.perform(&ActionRequest {
                    action: Action::Click,
                    target_tree: TreeId::ROOT,
                    target_node: NodeId::from(self.dialog),
                    data: None,
                });
            }
        }
    }

    pub fn declined(&self) -> bool {
        self.declined
    }

    pub fn cancel(&mut self) {
        self.declined = true;
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(Answer::Cancel);
        }
    }

    fn answer(&mut self) {
        let choices = std::mem::take(&mut self.shared.borrow_mut().choices);
        for choice in choices {
            match choice {
                Choice::Authenticate(password) => {
                    if let Some(reply) = self.reply.take() {
                        let _ = reply.send(Answer::Password(password));
                        self.set_waiting(true);
                    }
                }
                Choice::SwitchTo(login) => self.switch_to(login),
                Choice::Cancel => self.cancel(),
            }
        }
    }

    fn switch_to(&mut self, login: String) {
        self.ui
            .update_widget::<Authorization>(self.dialog, |dialog, _| {
                dialog.prompt.user = login.clone();
                dialog.note = None;
            });
        self.clear_field();
        match self.reply.take() {
            Some(reply) => {
                let _ = reply.send(Answer::SwitchTo(login));
                self.set_waiting(true);
            }
            None => self.switch_to = Some(login),
        }
    }

    fn set_waiting(&mut self, waiting: bool) {
        self.ui
            .update_widget::<Button>(self.authenticate_button(), |button, _| {
                button.set_enabled(!waiting)
            });
    }

    fn authenticate_button(&self) -> WidgetId {
        self.ui
            .widget_ref::<Authorization>(self.dialog)
            .and_then(|dialog| dialog.children.get(AUTHENTICATE))
            .map_or(self.dialog, |button| button.id())
    }

    fn set_note(&mut self, note: Option<Note>) {
        self.ui
            .update_widget::<Authorization>(self.dialog, |dialog, _| dialog.note = note);
    }

    fn clear_field(&mut self) {
        self.ui.perform(&ActionRequest {
            action: Action::SetValue,
            target_tree: TreeId::ROOT,
            target_node: NodeId::from(self.field),
            data: Some(ActionData::Value(String::new().into())),
        });
        self.ui.set_focus(self.field);
    }
}

impl Drop for Dialog {
    fn drop(&mut self) {
        self.cancel();
    }
}
