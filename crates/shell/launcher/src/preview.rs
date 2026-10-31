use accesskit::{Node, Role};
use bande_fs::{format_date, format_size};
use bande_preview::Preview;
use bande_toolkit::ui::{Context, Size, Widget, WidgetId};
use bande_toolkit::{
    Image, Item, Palette, Point, Radius, Rect, Shape, TextAlign, TextRole, TextStyle,
};

use crate::L10N;
use crate::model::{Clip, Hit, Kind, Target};
use crate::rows::{centered, fitted, icon_items};
use crate::view::{ICON_SIZE, Shared};

const PREVIEW_PADDING: f32 = 20.0;
const PREVIEW_VISUAL: f32 = 150.0;
const CODE_SIZE: f32 = 11.0;
const RESULT_SIZE: f32 = 34.0;
const INFO_LABEL_WIDTH: f32 = 72.0;
const INFO_LINE: f32 = 18.0;
const CLIP_VISUAL: f32 = 200.0;

fn ago(seconds: u64) -> String {
    let (id, count) = match seconds {
        0..60 => return L10N.tr("just-now"),
        60..3600 => ("minutes-ago", seconds / 60),
        3600..86400 => ("hours-ago", seconds / 3600),
        _ => ("days-ago", seconds / 86400),
    };
    L10N.tr_args(id, [("count", count.into())])
}

fn clip_description(clip: &Clip, content: Option<&Preview<Image>>) -> String {
    match clip.kind() {
        Kind::CopiedFiles => L10N.tr_args("file-count", [("count", clip.paths().len().into())]),
        Kind::CopiedImage => match content {
            Some(Preview::Image(image)) => L10N.tr_args(
                "image-size",
                [
                    ("width", image.width().into()),
                    ("height", image.height().into()),
                ],
            ),
            _ => L10N.tr("copied-image"),
        },
        _ => L10N.tr_args(
            "character-count",
            [("count", clip.text.chars().count().into())],
        ),
    }
}

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
        let label_style = TextStyle::role(TextRole::Callout, cx.palette.ink3);
        let value_style = TextStyle::role(TextRole::Callout, cx.palette.ink);
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
        content: Option<&Preview<Image>>,
        area: Rect,
        items: &mut Vec<Item>,
    ) {
        match content {
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
                    shape: Shape::rounded(rect, Radius::S),
                    items: vec![Item::Image {
                        rect,
                        image: image.clone(),
                    }],
                });
            }
            Some(Preview::Text(text)) => {
                let shape = Shape::rounded(area, Radius::M);
                let lines = (area.height / CODE_SIZE).ceil() as usize;
                let shown = text.lines().take(lines).collect::<Vec<_>>().join("\n");
                let paragraph = cx.text.paragraph(
                    &shown,
                    TextStyle::new(CODE_SIZE, cx.palette.ink).monospace(),
                    None,
                    TextAlign::Start,
                );
                items.push(Item::Fill {
                    shape,
                    paint: cx.palette.well.into(),
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
        let primary = cx.palette.ink;
        let muted = cx.palette.ink2;
        let loaded = state.preview.as_ref().filter(
            |loaded| matches!(&hit.target, Target::Path(path) if *path == loaded.entry.path),
        );
        let clip = match &hit.target {
            Target::Clip(id) => state.model.clip(*id).cloned(),
            _ => None,
        };
        let clip_text = clip
            .as_ref()
            .filter(|clip| clip.kind() == Kind::CopiedText)
            .map(|clip| Preview::Text(clip.text.clone()));
        let content = match &hit.target {
            Target::Path(_) => loaded.map(|loaded| &loaded.content),
            Target::Clip(id) => clip_text.as_ref().or_else(|| {
                state
                    .clip_picture
                    .as_ref()
                    .filter(|(pictured, _)| pictured == id)
                    .map(|(_, picture)| picture)
            }),
            _ => None,
        };
        if let (Kind::Calculation, Some(expression)) = (hit.kind, &hit.detail) {
            let expression = fitted(
                cx,
                expression,
                TextStyle::role(TextRole::WindowTitle, muted),
                inner,
            );
            let result = fitted(
                cx,
                &format!("= {}", hit.title),
                TextStyle::new(RESULT_SIZE, primary)
                    .weight(TextStyle::BOLD)
                    .tabular(),
                inner,
            );
            let block = expression.height() + 6.0 + result.height();
            y = frame.y + ((frame.height - block) / 2.0 - PREVIEW_PADDING).max(PREVIEW_PADDING);
            self.items.push(centered(&expression, frame, y));
            y += expression.height() + 6.0;
            self.items.push(centered(&result, frame, y));
        } else {
            let icon = state.catalog.icon(&hit);
            let visual = if hit.kind == Kind::CopiedText {
                CLIP_VISUAL
            } else {
                PREVIEW_VISUAL
            };
            let area = Rect::new(frame.x + PREVIEW_PADDING, y, inner, visual);
            Self::visual(cx, &hit, icon.as_ref(), content, area, &mut self.items);
            y += visual + 14.0;
            if hit.kind != Kind::CopiedText {
                let title = fitted(
                    cx,
                    &hit.title,
                    TextStyle::role(TextRole::Heading, primary),
                    inner,
                );
                self.items.push(centered(&title, frame, y));
                y += title.height() + 2.0;
            }
            let subtitle = match (&hit.target, &clip) {
                (Target::Path(path), _) => Some(state.catalog.kind(path, hit.kind == Kind::Folder)),
                (Target::Settings(_), _) => {
                    hit.detail.clone().or_else(|| Some(L10N.tr("settings")))
                }
                (Target::Clip(_), Some(clip)) => Some(clip_description(clip, content)),
                _ => hit.detail.clone(),
            };
            if let Some(subtitle) = subtitle.filter(|subtitle| *subtitle != hit.title) {
                let subtitle = fitted(
                    cx,
                    &subtitle,
                    TextStyle::role(TextRole::Callout, muted),
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
            if let Some(clip) = &clip {
                let lines = [(
                    L10N.tr("info-copied"),
                    ago(state.model.now().saturating_sub(clip.copied)),
                )];
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
                Kind::Command => L10N.tr("hint-command"),
                Kind::Emoji => L10N.tr("hint-emoji-copy"),
                Kind::CopiedText if state.model.inserts => L10N.tr("hint-clip-insert"),
                Kind::CopiedText | Kind::CopiedImage | Kind::CopiedFiles => {
                    L10N.tr("hint-clip-copy")
                }
                Kind::Folder | Kind::Document => L10N.tr("hint-file"),
            }
        };
        let color = if key.copied {
            cx.palette.accent_text
        } else {
            cx.palette.ink3
        };
        let hint = fitted(cx, &hint, TextStyle::role(TextRole::Callout, color), inner);
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
                loaded: state.preview.is_some() || state.clip_picture.is_some(),
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
