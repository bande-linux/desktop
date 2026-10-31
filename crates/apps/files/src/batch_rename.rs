use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use accesskit::{Node, Role};
use bande_fs::{
    Action, BatchItem, BatchProblem, Change, Numbering, RenameRule, new_name, plan_renames,
    rename_all,
};
use bande_toolkit::ui::{
    BUTTON_HEIGHT, Button, Context, PopUpButton, Size, TextField, Widget, WidgetId,
};
use bande_toolkit::{Item, Paragraph, Point, Rect, TextAlign, TextRole, TextStyle};

use crate::L10N;
use crate::controls::{Signal, sender};
use crate::view::{FilesView, Sheet};

const WIDTH: f32 = 460.0;
const PADDING: f32 = 20.0;
const LABEL_WIDTH: f32 = 128.0;
const ROW_GAP: f32 = 10.0;
const SECTION_GAP: f32 = 16.0;
const BUTTON_SPACING: f32 = 8.0;
const BUTTON_MIN_WIDTH: f32 = 76.0;
const PREVIEWED: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleKind {
    Replace,
    Add,
    Format,
}

impl RuleKind {
    const ALL: [Self; 3] = [Self::Replace, Self::Add, Self::Format];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchRename {
    pub items: Vec<BatchItem>,
    pub kind: RuleKind,
    pub find: String,
    pub replace: String,
    pub text: String,
    pub add_after: bool,
    pub numbering: Numbering,
    pub format_after: bool,
    pub name: String,
    pub start: String,
    pub(crate) widget: WidgetId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenameField {
    Find,
    Replace,
    Text,
    Name,
    Start,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenameChoice {
    Kind,
    AddWhere,
    Numbering,
    FormatWhere,
}

impl BatchRename {
    pub fn rule(&self) -> RenameRule {
        match self.kind {
            RuleKind::Replace => RenameRule::Replace {
                find: self.find.clone(),
                replace: self.replace.clone(),
            },
            RuleKind::Add => RenameRule::Add {
                text: self.text.clone(),
                after: self.add_after,
            },
            RuleKind::Format => RenameRule::Format {
                name: self.name.clone(),
                numbering: self.numbering,
                start: self.start.trim().parse().unwrap_or(1),
                after: self.format_after,
            },
        }
    }

    fn problem(&self) -> Option<String> {
        if self.kind == RuleKind::Format && self.start.trim().parse::<u64>().is_err() {
            return Some(L10N.tr("batch-start-number"));
        }
        let problem = plan_renames(&self.items, &self.rule()).err()?;
        Some(match problem {
            BatchProblem::Empty => L10N.tr("rename-empty"),
            BatchProblem::ContainsSlash => L10N.tr("rename-slash"),
            BatchProblem::Reserved => L10N.tr("rename-reserved"),
            BatchProblem::Twice(name) => L10N.tr_args("batch-twice", [("name", name.into())]),
            BatchProblem::Exists(name) => L10N.tr_args("rename-exists", [("name", name.into())]),
        })
    }

    pub fn preview(&self) -> Vec<String> {
        let rule = self.rule();
        self.items
            .iter()
            .take(PREVIEWED)
            .enumerate()
            .map(|(position, item)| {
                let old = name_of(&item.path);
                let new = new_name(&old, item.folder, &rule, position);
                L10N.tr_args("batch-example", [("old", old.into()), ("new", new.into())])
            })
            .collect()
    }
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn where_titles() -> [String; 2] {
    [L10N.tr("batch-after-name"), L10N.tr("batch-before-name")]
}

pub(crate) struct RenameSheet {
    id: WidgetId,
    title: String,
    labels: Vec<String>,
    children: Vec<Box<dyn Widget>>,
    preview: Vec<String>,
    problem: Option<String>,
    chosen: Rc<Cell<bool>>,
    rename: WidgetId,
    title_text: Option<Paragraph>,
    label_texts: Vec<Paragraph>,
    preview_texts: Vec<Paragraph>,
    problem_text: Option<Paragraph>,
    sizes: Vec<Size>,
    frame: Rect,
}

impl RenameSheet {
    fn buttons(&self) -> usize {
        self.children.len() - 2
    }

    fn set_rows(&mut self, rows: Vec<(String, Box<dyn Widget>)>) {
        let buttons = self.children.split_off(self.buttons());
        self.children.truncate(1);
        self.labels.truncate(1);
        for (label, widget) in rows {
            self.labels.push(label);
            self.children.push(widget);
        }
        self.children.extend(buttons);
    }

    fn set_outcome(&mut self, preview: Vec<String>, problem: Option<String>) {
        self.preview = preview;
        self.problem = problem;
    }
}

impl Widget for RenameSheet {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let width = WIDTH.min(available.width - 2.0 * PADDING);
        let text_width = width - 2.0 * PADDING;
        let field_width = text_width - LABEL_WIDTH;
        let palette = cx.palette;
        let title = cx.text.paragraph(
            &self.title,
            TextStyle::role(TextRole::BodyStrong, palette.ink),
            Some(text_width),
            TextAlign::Start,
        );
        self.label_texts = self
            .labels
            .iter()
            .map(|label| {
                cx.text.paragraph(
                    label,
                    TextStyle::role(TextRole::Body, palette.ink2),
                    Some(LABEL_WIDTH - 10.0),
                    TextAlign::End,
                )
            })
            .collect();
        self.preview_texts = self
            .preview
            .iter()
            .map(|line| {
                cx.text.paragraph(
                    line,
                    TextStyle::role(TextRole::Callout, palette.ink2),
                    Some(text_width),
                    TextAlign::Start,
                )
            })
            .collect();
        self.problem_text = self.problem.as_ref().map(|problem| {
            cx.text.paragraph(
                problem,
                TextStyle::role(TextRole::Callout, palette.danger),
                Some(text_width),
                TextAlign::Start,
            )
        });
        let buttons = self.buttons();
        self.sizes = self
            .children
            .iter_mut()
            .enumerate()
            .map(|(index, child)| {
                let size = child.measure(cx, Size::new(field_width, available.height));
                match index < buttons {
                    true => Size::new(field_width, size.height),
                    false => Size::new(size.width.max(BUTTON_MIN_WIDTH), BUTTON_HEIGHT),
                }
            })
            .collect();
        let rows = self.sizes[..buttons]
            .iter()
            .map(|size| size.height.max(BUTTON_HEIGHT) + ROW_GAP)
            .sum::<f32>();
        let preview = self
            .preview_texts
            .iter()
            .map(|text| text.height() + 2.0)
            .sum::<f32>();
        let problem = self
            .problem_text
            .as_ref()
            .map_or(0.0, |text| text.height() + 6.0);
        let height = PADDING
            + title.height()
            + SECTION_GAP
            + rows
            + SECTION_GAP
            + preview
            + problem
            + SECTION_GAP
            + BUTTON_HEIGHT
            + PADDING;
        self.title_text = Some(title);
        Size::new(width, height.ceil())
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let buttons = self.buttons();
        let mut y = frame.y
            + PADDING
            + self.title_text.as_ref().map_or(0.0, Paragraph::height)
            + SECTION_GAP;
        let x = frame.x + PADDING + LABEL_WIDTH;
        for index in 0..buttons {
            let size = self.sizes[index];
            let height = size.height.max(BUTTON_HEIGHT);
            self.children[index].place(
                cx,
                Rect::new(
                    x,
                    y + ((height - size.height) / 2.0).round(),
                    size.width,
                    size.height,
                ),
            );
            y += height + ROW_GAP;
        }
        let y = frame.y + frame.height - PADDING - BUTTON_HEIGHT;
        let mut x = frame.x + frame.width - PADDING;
        for index in (buttons..self.children.len()).rev() {
            let size = self.sizes[index];
            x -= size.width;
            self.children[index].place(cx, Rect::new(x, y, size.width, size.height));
            x -= BUTTON_SPACING;
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let left = self.frame.x + PADDING;
        let mut y = self.frame.y + PADDING;
        if let Some(title) = &self.title_text {
            items.push(Item::Text {
                origin: Point::new(left, y),
                paragraph: title.clone(),
            });
        }
        for (index, label) in self.label_texts.iter().enumerate() {
            if let Some(child) = self.children.get(index) {
                let frame = child.frame();
                items.push(Item::Text {
                    origin: Point::new(
                        left,
                        frame.y + ((frame.height - label.height()) / 2.0).round(),
                    ),
                    paragraph: label.clone(),
                });
                y = frame.y + frame.height.max(BUTTON_HEIGHT);
            }
        }
        y += SECTION_GAP;
        for line in self.preview_texts.iter().chain(&self.problem_text) {
            items.push(Item::Text {
                origin: Point::new(left, y),
                paragraph: line.clone(),
            });
            y += line.height() + 2.0;
        }
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Dialog);
        node.set_label(self.title.clone());
        if let Some(problem) = &self.problem {
            node.set_description(problem.clone());
        }
        node.set_modal();
        node
    }

    fn dismissed(&self) -> bool {
        self.chosen.get()
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

impl FilesView {
    fn rename_field(&self, field: RenameField, text: &str) -> Box<dyn Widget> {
        let send = sender(&self.signals);
        Box::new(TextField::new("", text, move |text| {
            send(Signal::RenameField(field, text.to_owned()));
        }))
    }

    fn rename_choice(
        &self,
        choice: RenameChoice,
        titles: &[String],
        selected: usize,
    ) -> Box<dyn Widget> {
        let send = sender(&self.signals);
        let label = titles.first().cloned().unwrap_or_default();
        Box::new(PopUpButton::new(&label, titles, selected, move |index| {
            send(Signal::RenameChoice(choice, index));
        }))
    }

    fn rename_rows(&self, state: &BatchRename) -> Vec<(String, Box<dyn Widget>)> {
        let after = |after: bool| usize::from(!after);
        match state.kind {
            RuleKind::Replace => vec![
                (
                    L10N.tr("batch-find"),
                    self.rename_field(RenameField::Find, &state.find),
                ),
                (
                    L10N.tr("batch-replace-with"),
                    self.rename_field(RenameField::Replace, &state.replace),
                ),
            ],
            RuleKind::Add => vec![
                (
                    L10N.tr("batch-text"),
                    self.rename_field(RenameField::Text, &state.text),
                ),
                (
                    L10N.tr("batch-where"),
                    self.rename_choice(
                        RenameChoice::AddWhere,
                        &where_titles(),
                        after(state.add_after),
                    ),
                ),
            ],
            RuleKind::Format => vec![
                (
                    L10N.tr("batch-format"),
                    self.rename_choice(
                        RenameChoice::Numbering,
                        &[L10N.tr("batch-name-index"), L10N.tr("batch-name-counter")],
                        usize::from(state.numbering == Numbering::Counter),
                    ),
                ),
                (
                    L10N.tr("batch-where"),
                    self.rename_choice(
                        RenameChoice::FormatWhere,
                        &where_titles(),
                        after(state.format_after),
                    ),
                ),
                (
                    L10N.tr("batch-custom-format"),
                    self.rename_field(RenameField::Name, &state.name),
                ),
                (
                    L10N.tr("batch-start-at"),
                    self.rename_field(RenameField::Start, &state.start),
                ),
            ],
        }
    }

    pub(crate) fn ask_batch_rename(&mut self) {
        let selection = self.browser.selection();
        if selection.len() < 2 || selection.iter().any(|entry| entry.trashed.is_some()) {
            return;
        }
        let items = selection
            .iter()
            .map(|entry| BatchItem {
                path: entry.path.clone(),
                folder: entry.is_folder(),
            })
            .collect::<Vec<_>>();
        let name = items
            .first()
            .map(|item| {
                let name = name_of(&item.path);
                let (stem, _) = name
                    .rsplit_once('.')
                    .filter(|_| !item.folder)
                    .unwrap_or((&name, ""));
                stem.trim_end_matches(|c: char| {
                    c.is_ascii_digit() || c == ' ' || c == '_' || c == '-'
                })
                .to_owned()
            })
            .unwrap_or_default();
        let chosen = Rc::new(Cell::new(false));
        let choose = Rc::new(self.choice());
        let button = |title: &str, action: &'static str| {
            let (chosen, choose) = (chosen.clone(), choose.clone());
            Button::new(title, move || {
                if !chosen.replace(true) {
                    choose(action);
                }
            })
        };
        let cancel = button(&L10N.tr("cancel"), "cancel").cancel();
        let rename = button(&L10N.tr("menu-rename"), "rename").primary();
        let (cancel_id, rename_id) = (cancel.id(), rename.id());
        let mut state = BatchRename {
            items,
            kind: RuleKind::Replace,
            find: String::new(),
            replace: String::new(),
            text: String::new(),
            add_after: true,
            numbering: Numbering::Index,
            format_after: true,
            name,
            start: "1".to_owned(),
            widget: WidgetId::next(),
        };
        let kinds = [
            L10N.tr("batch-replace-text"),
            L10N.tr("batch-add-text"),
            L10N.tr("batch-format-names"),
        ];
        let mut sheet = RenameSheet {
            id: WidgetId::next(),
            title: L10N.tr_args("batch-title", [("count", state.items.len().into())]),
            labels: vec![String::new()],
            children: vec![
                self.rename_choice(RenameChoice::Kind, &kinds, 0),
                Box::new(cancel),
                Box::new(rename),
            ],
            preview: Vec::new(),
            problem: None,
            chosen,
            rename: rename_id,
            title_text: None,
            label_texts: Vec::new(),
            preview_texts: Vec::new(),
            problem_text: None,
            sizes: Vec::new(),
            frame: Rect::default(),
        };
        sheet.set_rows(self.rename_rows(&state));
        state.widget = sheet.id;
        let first_field = sheet.children.get(1).map(|child| child.id());
        self.show_sheet(sheet, Sheet::Rename(state), Some(cancel_id));
        self.refresh_batch_rename(false);
        if let Some(field) = first_field {
            self.focus(field);
        }
    }

    pub(crate) fn refresh_batch_rename(&mut self, rows: bool) {
        let Some(Sheet::Rename(state)) = &self.sheet else {
            return;
        };
        let state = state.clone();
        let new_rows = rows.then(|| self.rename_rows(&state));
        let (preview, problem) = (state.preview(), state.problem());
        let enabled = problem.is_none();
        let mut rename = None;
        let mut focus = None;
        if let Some(sheet) = self.ui.widget_as::<RenameSheet>(state.widget) {
            if let Some(new_rows) = new_rows {
                sheet.set_rows(new_rows);
                focus = sheet.children.get(1).map(|child| child.id());
            }
            sheet.set_outcome(preview, problem);
            rename = Some(sheet.rename);
        }
        if let Some(button) = rename.and_then(|id| self.ui.widget_as::<Button>(id)) {
            button.set_enabled(enabled);
        }
        self.ui
            .layout(Size::new(self.size.0.max(1.0), self.size.1.max(1.0)));
        if let Some(field) = focus {
            self.focus(field);
        }
    }

    pub(crate) fn batch_field(&mut self, field: RenameField, text: String) {
        if let Some(Sheet::Rename(state)) = &mut self.sheet {
            match field {
                RenameField::Find => state.find = text,
                RenameField::Replace => state.replace = text,
                RenameField::Text => state.text = text,
                RenameField::Name => state.name = text,
                RenameField::Start => state.start = text,
            }
        }
        self.refresh_batch_rename(false);
    }

    pub(crate) fn batch_choice(&mut self, choice: RenameChoice, index: usize) {
        let mut rows = false;
        if let Some(Sheet::Rename(state)) = &mut self.sheet {
            match choice {
                RenameChoice::Kind => {
                    let kind = RuleKind::ALL
                        .get(index)
                        .copied()
                        .unwrap_or(RuleKind::Replace);
                    rows = kind != state.kind;
                    state.kind = kind;
                }
                RenameChoice::AddWhere => state.add_after = index == 0,
                RenameChoice::Numbering => {
                    state.numbering = match index {
                        0 => Numbering::Index,
                        _ => Numbering::Counter,
                    };
                }
                RenameChoice::FormatWhere => state.format_after = index == 0,
            }
        }
        self.refresh_batch_rename(rows);
    }

    pub(crate) fn apply_batch_rename(&mut self, state: BatchRename) {
        let plan = match plan_renames(&state.items, &state.rule()) {
            Ok(plan) => plan,
            Err(_) => return,
        };
        let renamed = plan
            .iter()
            .map(|(_, to)| to.clone())
            .collect::<Vec<PathBuf>>();
        let (changes, error) = rename_all(&plan);
        let done = changes
            .iter()
            .filter_map(|change| match change {
                Change::Moved { to, .. } => Some(to.clone()),
                _ => None,
            })
            .filter(|path| renamed.contains(path))
            .collect::<Vec<_>>();
        self.operations.record(Action::Rename, changes);
        if let Some(error) = error {
            self.show_notice(
                L10N.tr_args("error-operation", [("error", error.to_string().into())]),
            );
        }
        let kept = state
            .items
            .iter()
            .map(|item| item.path.clone())
            .filter(|path| !plan.iter().any(|(from, _)| from == path) && path.exists());
        let selected = done.into_iter().chain(kept).collect::<Vec<_>>();
        self.reveal_after_change(&selected);
    }
}
