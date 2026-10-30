use accesskit::{Node, Role};
use unbound_fs::{format_date, format_size};
use unbound_preview::Preview;
use unbound_toolkit::ui::{Context, Size, Widget, WidgetId};
use unbound_toolkit::{Image, Item, Palette, Point, Rect, Shape, TextAlign, TextStyle};

use crate::L10N;
use crate::model::{Hit, Kind, Target};
use crate::rows::{centered, fitted, icon_items};
use crate::view::{ICON_SIZE, Loaded, Shared};

const PREVIEW_PADDING: f32 = 20.0;
const PREVIEW_VISUAL: f32 = 150.0;
pub const PREVIEW_TITLE_SIZE: f32 = 16.0;
const PREVIEW_TEXT_SIZE: f32 = 12.0;
const CODE_SIZE: f32 = 11.0;
const RESULT_SIZE: f32 = 34.0;
const HINT_SIZE: f32 = 11.0;
const INFO_LABEL_WIDTH: f32 = 72.0;
const INFO_LINE: f32 = 18.0;

#[derive(Clone, Copy, PartialEq)]
struct PreviewKey {
    revision: u64,
    selected: usize,
    copied: bool,
    loaded: bool,
    palette: Palette,
    frame: Rect,
}

pub struct PreviewPane {
    id: WidgetId,
    state: Shared,
    items: Vec<Item>,
    painted: Option<PreviewKey>,
    frame: Rect,
}

impl PreviewPane {
    pub fn new(state: Shared) -> Self {
        Self {
            id: WidgetId::next(),
            state,
            items: Vec::new(),
            painted: None,
            frame: Rect::default(),
        }
    }
}

impl PreviewPane {
    fn info(
        cx: &mut Context,
        frame: Rect,
        y: &mut f32,
        lines: &[(String, String)],
        items: &mut Vec<Item>,
    ) {
        let label_style = TextStyle::new(PREVIEW_TEXT_SIZE, cx.palette.label_muted);
        let value_style = TextStyle::new(PREVIEW_TEXT_SIZE, cx.palette.label_primary);
        let value_x = frame.x + PREVIEW_PADDING + INFO_LABEL_WIDTH;
        let value_width = frame.x + frame.width - PREVIEW_PADDING - value_x;
        for (label, value) in lines {
            let label = fitted(cx, label, label_style, INFO_LABEL_WIDTH - 8.0);
            items.push(Item::Text {
                origin: Point::new(value_x - 8.0 - label.width(), *y),
                paragraph: label,
            });
            let value = fitted(cx, value, value_style, value_width);
            items.push(Item::Text {
                origin: Point::new(value_x, *y),
                paragraph: value,
            });
            *y += INFO_LINE;
        }
    }

    fn visual(
        cx: &mut Context,
        hit: &Hit,
        icon: Option<&Image>,
        loaded: Option<&Loaded>,
        area: Rect,
        items: &mut Vec<Item>,
    ) {
        match loaded.map(|loaded| &loaded.content) {
            Some(Preview::Image(image)) => {
                let scale = (area.width / image.width() as f32)
                    .min(area.height / image.height() as f32)
                    .min(1.0);
                let (width, height) = (
                    (image.width() as f32 * scale).round(),
                    (image.height() as f32 * scale).round(),
                );
                let rect = Rect::new(
                    (area.x + (area.width - width) / 2.0).round(),
                    (area.y + (area.height - height) / 2.0).round(),
                    width,
                    height,
                );
                items.push(Item::Clip {
                    shape: Shape::rounded(rect, 6.0),
                    items: vec![Item::Image {
                        rect,
                        image: image.clone(),
                    }],
                });
            }
            Some(Preview::Text(text)) => {
                let shape = Shape::rounded(area, 8.0);
                let lines = (area.height / CODE_SIZE).ceil() as usize;
                let shown = text.lines().take(lines).collect::<Vec<_>>().join("\n");
                let paragraph = cx.text.paragraph(
                    &shown,
                    TextStyle::new(CODE_SIZE, cx.palette.label_primary).monospace(),
                    None,
                    TextAlign::Start,
                );
                items.push(Item::Fill {
                    shape,
                    paint: cx.palette.surface_background.into(),
                });
                items.push(Item::Clip {
                    shape,
                    items: vec![Item::Text {
                        origin: Point::new(area.x + 10.0, area.y + 8.0),
                        paragraph,
                    }],
                });
            }
            _ => {
                let size = ICON_SIZE as f32;
                let rect = Rect::new(
                    (area.x + (area.width - size) / 2.0).round(),
                    (area.y + (area.height - size) / 2.0).round(),
                    size,
                    size,
                );
                icon_items(cx, hit, false, icon, rect, items);
            }
        }
    }

    fn compose(&mut self, cx: &mut Context, key: PreviewKey) {
        self.items.clear();
        let frame = self.frame;
        let mut state = self.state.borrow_mut();
        let state = &mut *state;
        let Some(hit) = state.model.selection().cloned() else {
            return;
        };
        let inner = frame.width - 2.0 * PREVIEW_PADDING;
        let mut y = frame.y + PREVIEW_PADDING;
        let primary = cx.palette.label_primary;
        let muted = cx.palette.label_muted;
        let loaded = state.preview.as_ref().filter(
            |loaded| matches!(&hit.target, Target::Path(path) if *path == loaded.entry.path),
        );
        if let (Kind::Calculation, Some(expression)) = (hit.kind, &hit.detail) {
            let expression = fitted(cx, expression, TextStyle::new(15.0, muted), inner);
            let result = fitted(
                cx,
                &format!("= {}", hit.title),
                TextStyle::new(RESULT_SIZE, primary).weight(TextStyle::SEMIBOLD),
                inner,
            );
            let block = expression.height() + 6.0 + result.height();
            y = frame.y + ((frame.height - block) / 2.0 - PREVIEW_PADDING).max(PREVIEW_PADDING);
            self.items.push(centered(&expression, frame, y));
            y += expression.height() + 6.0;
            self.items.push(centered(&result, frame, y));
        } else {
            let icon = state.catalog.icon(&hit);
            let area = Rect::new(frame.x + PREVIEW_PADDING, y, inner, PREVIEW_VISUAL);
            Self::visual(cx, &hit, icon.as_ref(), loaded, area, &mut self.items);
            y += PREVIEW_VISUAL + 14.0;
            let title = fitted(
                cx,
                &hit.title,
                TextStyle::new(PREVIEW_TITLE_SIZE, primary).weight(TextStyle::SEMIBOLD),
                inner,
            );
            self.items.push(centered(&title, frame, y));
            y += title.height() + 2.0;
            let subtitle = match &hit.target {
                Target::Path(path) => Some(state.catalog.kind(path, hit.kind == Kind::Folder)),
                Target::Settings(_) => hit.detail.clone().or_else(|| Some(L10N.tr("settings"))),
                _ => hit.detail.clone(),
            };
            if let Some(subtitle) = subtitle.filter(|subtitle| *subtitle != hit.title) {
                let subtitle = fitted(
                    cx,
                    &subtitle,
                    TextStyle::new(PREVIEW_TEXT_SIZE, muted),
                    inner,
                );
                self.items.push(centered(&subtitle, frame, y));
                y += subtitle.height();
            }
            y += 14.0;
            if let Target::Path(_) = &hit.target {
                let mut lines = Vec::new();
                if let Some(loaded) = loaded {
                    match loaded.content {
                        Preview::Folder { items } => lines.push((
                            L10N.tr("info-items"),
                            L10N.tr_args("item-count", [("count", items.into())]),
                        )),
                        _ => lines.push((L10N.tr("info-size"), format_size(loaded.entry.size))),
                    }
                    if let Some(modified) = loaded.entry.modified {
                        lines.push((L10N.tr("info-modified"), format_date(modified)));
                    }
                }
                if let Some(parent) = &hit.detail {
                    lines.push((L10N.tr("info-where"), parent.clone()));
                }
                Self::info(cx, frame, &mut y, &lines, &mut self.items);
            }
        }
        let hint = if key.copied {
            L10N.tr("hint-copied")
        } else {
            match hit.kind {
                Kind::Calculation => L10N.tr("hint-calculation"),
                Kind::Application => L10N.tr("hint-application"),
                Kind::Setting => L10N.tr("hint-setting"),
                Kind::Emoji => L10N.tr("hint-emoji-copy"),
                Kind::Folder | Kind::Document => L10N.tr("hint-file"),
            }
        };
        let color = if key.copied {
            cx.palette.accent_color
        } else {
            muted
        };
        let hint = fitted(cx, &hint, TextStyle::new(HINT_SIZE, color), inner);
        let bottom = frame.y + frame.height - PREVIEW_PADDING + 4.0 - hint.height();
        self.items.push(centered(&hint, frame, bottom.max(y)));
    }
}

impl Widget for PreviewPane {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        if frame.width <= 0.0 {
            self.painted = None;
            self.items.clear();
            return;
        }
        let key = {
            let state = self.state.borrow();
            PreviewKey {
                revision: state.model.revision,
                selected: state.model.selected,
                copied: state.model.copied,
                loaded: state.preview.is_some(),
                palette: cx.palette,
                frame,
            }
        };
        if self.painted != Some(key) {
            self.painted = Some(key);
            self.compose(cx, key);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, _cx: &Context, items: &mut Vec<Item>) {
        items.extend(self.items.iter().cloned());
    }

    fn access(&self, _cx: &Context) -> Node {
        let state = self.state.borrow();
        let mut node = Node::new(Role::Group);
        node.set_label(L10N.tr("preview"));
        if let Some(hit) = state.model.selection() {
            node.set_description(hit.title.clone());
        }
        node
    }
}
