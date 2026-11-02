use std::sync::Arc;

use web_css::properties::{self as keywords, CssWideKeyword, LonghandId, PropertyValue};
use web_css::values::LengthPercentage as Specified;
use web_css::values::calc::{CalcLeaf, CalcNode};
use web_css::values::color::{Color as CssColor, Rgba};
use web_css::values::specified::{
    self, FamilyName, FontSize, GenericFamily, LengthPercentageOrAuto, MaxSize, Size,
};
use web_css::values::{Length, LengthUnit};

use crate::stylist::Viewport;
use crate::{
    Color, ComputedStyle, Direction, Display, FontFamily, InheritedStyle, LengthPercentage,
    LineHeight, ListStyleType, MEDIUM, ResetStyle, TextAlign, TextDecorations, VerticalAlign,
    WhiteSpace,
};

pub(crate) enum Value<'a> {
    Declared(&'a PropertyValue),
    Keyword(CssWideKeyword),
}

pub(crate) struct Context {
    pub(crate) root_font_size: f32,
    pub(crate) viewport: Viewport,
    pub(crate) root: bool,
}

struct Computer<'a> {
    parent: &'a ComputedStyle,
    inherited: InheritedStyle,
    reset: ResetStyle,
    context: &'a Context,
    decoration_color: Option<Color>,
    border_styles: [bool; 4],
}

fn priority(id: LonghandId) -> u8 {
    match id {
        LonghandId::FontSize => 0,
        LonghandId::FontFamily => 1,
        LonghandId::Color => 2,
        LonghandId::LineHeight => 3,
        _ => 4,
    }
}

pub(crate) fn compute(
    parent: &ComputedStyle,
    initial_reset: &Arc<ResetStyle>,
    mut winners: Vec<(LonghandId, Value<'_>)>,
    custom: Arc<[(Arc<str>, Arc<str>)]>,
    context: &Context,
) -> ComputedStyle {
    winners.sort_by_key(|(id, _)| priority(*id));
    let mut inherited = (*parent.inherited).clone();
    inherited.custom = custom;
    let mut computer = Computer {
        parent,
        inherited,
        reset: (**initial_reset).clone(),
        context,
        decoration_color: None,
        border_styles: [false; 4],
    };
    for (id, value) in &winners {
        match value {
            Value::Declared(value) => computer.apply(*id, value),
            Value::Keyword(keyword) => computer.keyword(*id, *keyword),
        }
        if *id == LonghandId::FontFamily || *id == LonghandId::FontSize {
            computer.fix_monospace();
        }
    }
    computer.finish();
    let Computer {
        inherited, reset, ..
    } = computer;
    let inherited = if inherited == *parent.inherited {
        parent.inherited.clone()
    } else {
        Arc::new(inherited)
    };
    let reset = if reset == **initial_reset {
        initial_reset.clone()
    } else {
        Arc::new(reset)
    };
    ComputedStyle { inherited, reset }
}

fn color(rgba: Rgba) -> Color {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color {
        r: channel(rgba.r),
        g: channel(rgba.g),
        b: channel(rgba.b),
        a: channel(rgba.a),
    }
}

fn rgba(color: Color) -> Rgba {
    Rgba::new(
        f32::from(color.r) / 255.0,
        f32::from(color.g) / 255.0,
        f32::from(color.b) / 255.0,
        f32::from(color.a) / 255.0,
    )
}

const KEYWORD_SIZES: [f32; 8] = [9.0, 10.0, 13.0, 16.0, 18.0, 24.0, 32.0, 48.0];

impl Computer<'_> {
    fn em(&self) -> f32 {
        self.inherited.font_size
    }

    fn length_px(&self, length: Length, em: f32) -> f32 {
        if let Some(factor) = length.unit.px_per_unit() {
            return length.value * factor;
        }
        let viewport = self.context.viewport;
        let rem = self.context.root_font_size;
        let unit = match length.unit {
            LengthUnit::Em => em,
            LengthUnit::Rem => rem,
            LengthUnit::Ex | LengthUnit::Ch => em / 2.0,
            LengthUnit::Rex | LengthUnit::Rch => rem / 2.0,
            LengthUnit::Cap => em * 0.7,
            LengthUnit::Rcap => rem * 0.7,
            LengthUnit::Ic => em,
            LengthUnit::Ric => rem,
            LengthUnit::Lh => em * 1.2,
            LengthUnit::Rlh => rem * 1.2,
            LengthUnit::Vw
            | LengthUnit::Svw
            | LengthUnit::Lvw
            | LengthUnit::Dvw
            | LengthUnit::Vi
            | LengthUnit::Svi
            | LengthUnit::Lvi
            | LengthUnit::Dvi
            | LengthUnit::Cqw
            | LengthUnit::Cqi => viewport.width / 100.0,
            LengthUnit::Vh
            | LengthUnit::Svh
            | LengthUnit::Lvh
            | LengthUnit::Dvh
            | LengthUnit::Vb
            | LengthUnit::Svb
            | LengthUnit::Lvb
            | LengthUnit::Dvb
            | LengthUnit::Cqh
            | LengthUnit::Cqb => viewport.height / 100.0,
            LengthUnit::Vmin
            | LengthUnit::Svmin
            | LengthUnit::Lvmin
            | LengthUnit::Dvmin
            | LengthUnit::Cqmin => viewport.width.min(viewport.height) / 100.0,
            _ => viewport.width.max(viewport.height) / 100.0,
        };
        length.value * unit
    }

    fn calc(&self, node: &CalcNode, em: f32, basis: f32) -> Option<f32> {
        node.evaluate(&|leaf| match leaf {
            CalcLeaf::Number(value) => Some(*value),
            CalcLeaf::Percentage(percent) => Some(percent / 100.0 * basis),
            CalcLeaf::Length(length) => Some(self.length_px(*length, em)),
            _ => None,
        })
    }

    fn length_percentage_em(&self, value: &Specified, em: f32) -> LengthPercentage {
        match value {
            Specified::Length(length) => LengthPercentage::px(self.length_px(*length, em)),
            Specified::Percentage(percent) => LengthPercentage {
                px: 0.0,
                percent: *percent,
            },
            Specified::Calc(node) => {
                let px = self.calc(node, em, 0.0).unwrap_or(0.0);
                let full = self.calc(node, em, 100.0).unwrap_or(px);
                LengthPercentage {
                    px,
                    percent: full - px,
                }
            }
        }
    }

    fn length_percentage(&self, value: &Specified) -> LengthPercentage {
        self.length_percentage_em(value, self.em())
    }

    fn auto(&self, value: &LengthPercentageOrAuto) -> Option<LengthPercentage> {
        match value {
            LengthPercentageOrAuto::Auto => None,
            LengthPercentageOrAuto::LengthPercentage(value) => Some(self.length_percentage(value)),
        }
    }

    fn size(&self, value: &Size) -> Option<LengthPercentage> {
        match value {
            Size::LengthPercentage(value) => Some(self.length_percentage(value)),
            _ => None,
        }
    }

    fn color(&self, value: &CssColor) -> Color {
        color(value.resolve(rgba(self.inherited.color), false))
    }

    fn fix_monospace(&mut self) {
        if let Some(factor) = self.inherited.font_size_keyword {
            let base = if self.inherited.font_family == FontFamily::Monospace {
                13.0
            } else {
                MEDIUM
            };
            self.inherited.font_size = base * factor;
        }
    }

    fn font_size(&mut self, value: &FontSize) {
        let parent = &self.parent.inherited;
        let keyword = |index: usize| KEYWORD_SIZES.get(index).copied().unwrap_or(MEDIUM) / MEDIUM;
        let (size, factor) = match value {
            FontSize::XxSmall => (0.0, Some(keyword(0))),
            FontSize::XSmall => (0.0, Some(keyword(1))),
            FontSize::Small => (0.0, Some(keyword(2))),
            FontSize::Medium => (0.0, Some(keyword(3))),
            FontSize::Large => (0.0, Some(keyword(4))),
            FontSize::XLarge => (0.0, Some(keyword(5))),
            FontSize::XxLarge => (0.0, Some(keyword(6))),
            FontSize::XxxLarge => (0.0, Some(keyword(7))),
            FontSize::Larger => (
                parent.font_size * 1.2,
                parent.font_size_keyword.map(|factor| factor * 1.2),
            ),
            FontSize::Smaller => (
                parent.font_size / 1.2,
                parent.font_size_keyword.map(|factor| factor / 1.2),
            ),
            FontSize::Math => (parent.font_size, parent.font_size_keyword),
            FontSize::LengthPercentage(value) => {
                let relative = match value {
                    Specified::Length(Length {
                        unit: LengthUnit::Em,
                        value,
                    }) => Some(*value),
                    Specified::Percentage(percent) => Some(percent / 100.0),
                    _ => None,
                };
                let size = self
                    .length_percentage_em(value, parent.font_size)
                    .resolve(parent.font_size);
                (
                    size,
                    relative
                        .and_then(|ratio| parent.font_size_keyword.map(|factor| factor * ratio)),
                )
            }
        };
        self.inherited.font_size = size.max(0.0);
        self.inherited.font_size_keyword = factor;
        if let Some(factor) = factor {
            self.inherited.font_size = MEDIUM * factor;
        }
    }

    fn font_family(&mut self, families: &specified::FontFamily) {
        let generic = |generic: GenericFamily| match generic {
            GenericFamily::Monospace | GenericFamily::UiMonospace => FontFamily::Monospace,
            GenericFamily::SansSerif
            | GenericFamily::SystemUi
            | GenericFamily::UiSansSerif
            | GenericFamily::UiRounded => FontFamily::SansSerif,
            _ => FontFamily::Serif,
        };
        let named = |name: &str| {
            let name = name.to_ascii_lowercase();
            if ["mono", "courier", "consolas", "menlo"]
                .iter()
                .any(|hint| name.contains(hint))
            {
                Some(FontFamily::Monospace)
            } else if [
                "sans",
                "arial",
                "helvetica",
                "verdana",
                "segoe",
                "roboto",
                "inter",
                "lato",
            ]
            .iter()
            .any(|hint| name.contains(hint))
            {
                Some(FontFamily::SansSerif)
            } else if ["serif", "times", "georgia", "libertine", "garamond"]
                .iter()
                .any(|hint| name.contains(hint))
            {
                Some(FontFamily::Serif)
            } else {
                None
            }
        };
        if let Some(family) = families.0.iter().find_map(|family| match family {
            FamilyName::Generic(family) => Some(generic(family.clone())),
            FamilyName::Named(name) => named(name),
        }) {
            self.inherited.font_family = family;
        }
    }

    fn white_space(&mut self, collapse: Option<keywords::WhiteSpaceCollapse>, wrap: Option<bool>) {
        use keywords::WhiteSpaceCollapse as Collapse;
        let current = self.inherited.white_space;
        let wraps = wrap.unwrap_or(current.wraps());
        let mode = collapse.map_or(
            match current {
                WhiteSpace::Normal | WhiteSpace::Nowrap => 0,
                WhiteSpace::Pre | WhiteSpace::PreWrap => 1,
                WhiteSpace::PreLine => 2,
            },
            |collapse| match collapse {
                Collapse::Collapse | Collapse::Discard => 0,
                Collapse::Preserve | Collapse::BreakSpaces | Collapse::PreserveSpaces => 1,
                Collapse::PreserveBreaks => 2,
            },
        );
        self.inherited.white_space = match (mode, wraps) {
            (0, true) => WhiteSpace::Normal,
            (0, false) | (2, false) => WhiteSpace::Nowrap,
            (1, false) => WhiteSpace::Pre,
            (1, true) => WhiteSpace::PreWrap,
            _ => WhiteSpace::PreLine,
        };
    }

    fn text_align(&self, value: keywords::TextAlign) -> TextAlign {
        use keywords::TextAlign as Align;
        match value {
            Align::Start => TextAlign::Start,
            Align::End => TextAlign::End,
            Align::Left | Align::WebkitLeft => TextAlign::Left,
            Align::Right | Align::WebkitRight => TextAlign::Right,
            Align::Center | Align::WebkitCenter => TextAlign::Center,
            Align::Justify => TextAlign::Justify,
            Align::MatchParent => {
                let parent = &self.parent.inherited;
                match (parent.text_align, parent.direction) {
                    (TextAlign::Start, Direction::Ltr) | (TextAlign::End, Direction::Rtl) => {
                        TextAlign::Left
                    }
                    (TextAlign::Start, Direction::Rtl) | (TextAlign::End, Direction::Ltr) => {
                        TextAlign::Right
                    }
                    (align, _) => align,
                }
            }
        }
    }

    fn apply(&mut self, id: LonghandId, value: &PropertyValue) {
        use LonghandId as Id;
        use PropertyValue as V;
        match (id, value) {
            (Id::Display, V::Display(display)) => {
                use specified::Display as D;
                self.reset.display = match display {
                    D::None | D::TableColumn | D::TableColumnGroup => Display::None,
                    D::Contents => Display::Contents,
                    D::Inline | D::InlineListItem | D::Ruby | D::RubyText => Display::Inline,
                    D::InlineBlock | D::InlineFlex | D::InlineGrid | D::InlineTable => {
                        Display::InlineBlock
                    }
                    D::ListItem => Display::ListItem,
                    _ => Display::Block,
                };
            }
            (Id::Color, V::Color(value)) => {
                self.inherited.color =
                    color(value.resolve(rgba(self.parent.inherited.color), false))
            }
            (Id::BackgroundColor, V::Color(value)) => {
                self.reset.background_color =
                    color(value.resolve(rgba(self.inherited.color), false))
            }
            (Id::FontSize, V::FontSize(value)) => self.font_size(value),
            (Id::FontFamily, V::FontFamily(value)) => self.font_family(value),
            (Id::FontWeight, V::FontWeight(value)) => {
                let parent = self.parent.inherited.font_weight;
                self.inherited.font_weight = match value {
                    specified::FontWeight::Absolute(weight) => weight.clamp(1.0, 1000.0) as u16,
                    specified::FontWeight::Bolder => match parent {
                        0..350 => 400,
                        350..550 => 700,
                        _ => 900,
                    },
                    specified::FontWeight::Lighter => match parent {
                        0..550 => 100,
                        550..750 => 400,
                        _ => 700,
                    },
                };
            }
            (Id::FontStyle, V::FontStyle(style)) => {
                self.inherited.italic = !matches!(style, keywords::FontStyle::Normal)
            }
            (Id::LineHeight, V::LineHeight(value)) => {
                self.inherited.line_height = match value {
                    specified::LineHeight::Normal => LineHeight::Normal,
                    specified::LineHeight::Number(number) => LineHeight::Number(*number),
                    specified::LineHeight::LengthPercentage(value) => {
                        LineHeight::Px(self.length_percentage(value).resolve(self.em()))
                    }
                }
            }
            (Id::TextAlign, V::TextAlign(value)) => {
                self.inherited.text_align = self.text_align(*value)
            }
            (Id::Direction, V::Direction(direction)) => {
                self.inherited.direction = match direction {
                    keywords::Direction::Ltr => Direction::Ltr,
                    keywords::Direction::Rtl => Direction::Rtl,
                }
            }
            (Id::WhiteSpaceCollapse, V::WhiteSpaceCollapse(value)) => {
                self.white_space(Some(*value), None)
            }
            (Id::TextWrapMode, V::TextWrapMode(value)) => {
                self.white_space(None, Some(matches!(value, keywords::TextWrapMode::Wrap)))
            }
            (Id::Visibility, V::Visibility(value)) => {
                self.inherited.visible = matches!(value, keywords::Visibility::Visible)
            }
            (Id::ListStyleType, V::ListStyleType(value)) => {
                self.inherited.list_style_type = match value {
                    specified::ListStyleType::None => ListStyleType::None,
                    specified::ListStyleType::String(_) => ListStyleType::Disc,
                    specified::ListStyleType::CounterStyle(name) => {
                        match name.to_ascii_lowercase().as_str() {
                            "circle" => ListStyleType::Circle,
                            "square" => ListStyleType::Square,
                            "decimal" | "decimal-leading-zero" => ListStyleType::Decimal,
                            "lower-alpha" | "lower-latin" => ListStyleType::LowerAlpha,
                            "upper-alpha" | "upper-latin" => ListStyleType::UpperAlpha,
                            "lower-roman" => ListStyleType::LowerRoman,
                            "upper-roman" => ListStyleType::UpperRoman,
                            _ => ListStyleType::Disc,
                        }
                    }
                }
            }
            (Id::VerticalAlign, V::VerticalAlign(value)) => {
                use specified::VerticalAlign as A;
                self.reset.vertical_align = match value {
                    A::Baseline => VerticalAlign::Baseline,
                    A::Sub => VerticalAlign::Sub,
                    A::Super => VerticalAlign::Super,
                    A::TextTop => VerticalAlign::TextTop,
                    A::TextBottom => VerticalAlign::TextBottom,
                    A::Middle => VerticalAlign::Middle,
                    A::Top => VerticalAlign::Top,
                    A::Bottom => VerticalAlign::Bottom,
                    A::LengthPercentage(value) => {
                        VerticalAlign::Length(self.length_percentage(value))
                    }
                }
            }
            (Id::TextDecorationLine, V::TextDecorationLine(lines)) => {
                let has = |bit: u8| lines.0 & bit != 0;
                let marker = Some(Color::TRANSPARENT);
                self.reset.text_decoration = TextDecorations {
                    underline: has(specified::TextDecorationLine::UNDERLINE)
                        .then_some(marker)
                        .flatten(),
                    overline: has(specified::TextDecorationLine::OVERLINE)
                        .then_some(marker)
                        .flatten(),
                    line_through: has(specified::TextDecorationLine::LINE_THROUGH)
                        .then_some(marker)
                        .flatten(),
                };
            }
            (Id::TextDecorationColor, V::Color(value)) => {
                self.decoration_color = Some(self.color(value))
            }
            (Id::MarginTop, V::LengthPercentageOrAuto(value)) => {
                self.reset.margin.top = self.auto(value)
            }
            (Id::MarginRight, V::LengthPercentageOrAuto(value)) => {
                self.reset.margin.right = self.auto(value)
            }
            (Id::MarginBottom, V::LengthPercentageOrAuto(value)) => {
                self.reset.margin.bottom = self.auto(value)
            }
            (Id::MarginLeft, V::LengthPercentageOrAuto(value)) => {
                self.reset.margin.left = self.auto(value)
            }
            (Id::PaddingTop, V::NonNegativeLengthPercentage(value)) => {
                self.reset.padding.top = self.length_percentage(&value.0)
            }
            (Id::PaddingRight, V::NonNegativeLengthPercentage(value)) => {
                self.reset.padding.right = self.length_percentage(&value.0)
            }
            (Id::PaddingBottom, V::NonNegativeLengthPercentage(value)) => {
                self.reset.padding.bottom = self.length_percentage(&value.0)
            }
            (Id::PaddingLeft, V::NonNegativeLengthPercentage(value)) => {
                self.reset.padding.left = self.length_percentage(&value.0)
            }
            (Id::BorderTopWidth, V::BorderWidth(value)) => {
                self.reset.border.top = self.length_percentage(&value.0).px
            }
            (Id::BorderRightWidth, V::BorderWidth(value)) => {
                self.reset.border.right = self.length_percentage(&value.0).px
            }
            (Id::BorderBottomWidth, V::BorderWidth(value)) => {
                self.reset.border.bottom = self.length_percentage(&value.0).px
            }
            (Id::BorderLeftWidth, V::BorderWidth(value)) => {
                self.reset.border.left = self.length_percentage(&value.0).px
            }
            (Id::BorderTopStyle, V::LineStyle(style)) => self.border_style(0, *style),
            (Id::BorderRightStyle, V::LineStyle(style)) => self.border_style(1, *style),
            (Id::BorderBottomStyle, V::LineStyle(style)) => self.border_style(2, *style),
            (Id::BorderLeftStyle, V::LineStyle(style)) => self.border_style(3, *style),
            (Id::BorderTopColor, V::Color(value)) => {
                self.reset.border_color.top = self.color(value)
            }
            (Id::BorderRightColor, V::Color(value)) => {
                self.reset.border_color.right = self.color(value)
            }
            (Id::BorderBottomColor, V::Color(value)) => {
                self.reset.border_color.bottom = self.color(value)
            }
            (Id::BorderLeftColor, V::Color(value)) => {
                self.reset.border_color.left = self.color(value)
            }
            (Id::Width, V::Size(value)) => self.reset.width = self.size(value),
            (Id::Height, V::Size(value)) => self.reset.height = self.size(value),
            (Id::MinWidth, V::Size(value)) => {
                self.reset.min_width = self.size(value).unwrap_or(LengthPercentage::ZERO)
            }
            (Id::MaxWidth, V::MaxSize(value)) => {
                self.reset.max_width = match value {
                    MaxSize::LengthPercentage(value) => Some(self.length_percentage(value)),
                    _ => None,
                }
            }
            _ => {}
        }
    }

    fn border_style(&mut self, side: u8, style: keywords::LineStyle) {
        let none = matches!(
            style,
            keywords::LineStyle::None | keywords::LineStyle::Hidden
        );
        let styles = &mut self.border_styles;
        if let Some(slot) = styles.get_mut(usize::from(side)) {
            *slot = !none;
        }
    }

    fn keyword(&mut self, id: LonghandId, keyword: CssWideKeyword) {
        let inherit = match keyword {
            CssWideKeyword::Inherit => true,
            CssWideKeyword::Initial => false,
            _ => id.inherited(),
        };
        let initial_inherited = InheritedStyle::initial();
        let initial_reset = ResetStyle::initial();
        let (from_inherited, from_reset) = if inherit {
            (&*self.parent.inherited, &*self.parent.reset)
        } else {
            (&initial_inherited, &initial_reset)
        };
        use LonghandId as Id;
        let (inherited, reset) = (&mut self.inherited, &mut self.reset);
        match id {
            Id::Display => reset.display = from_reset.display,
            Id::Color => inherited.color = from_inherited.color,
            Id::BackgroundColor => reset.background_color = from_reset.background_color,
            Id::FontSize => {
                inherited.font_size = from_inherited.font_size;
                inherited.font_size_keyword = from_inherited.font_size_keyword;
            }
            Id::FontFamily => inherited.font_family = from_inherited.font_family,
            Id::FontWeight => inherited.font_weight = from_inherited.font_weight,
            Id::FontStyle => inherited.italic = from_inherited.italic,
            Id::LineHeight => inherited.line_height = from_inherited.line_height,
            Id::TextAlign => inherited.text_align = from_inherited.text_align,
            Id::Direction => inherited.direction = from_inherited.direction,
            Id::WhiteSpaceCollapse | Id::TextWrapMode => {
                inherited.white_space = from_inherited.white_space
            }
            Id::Visibility => inherited.visible = from_inherited.visible,
            Id::ListStyleType => inherited.list_style_type = from_inherited.list_style_type,
            Id::VerticalAlign => reset.vertical_align = from_reset.vertical_align,
            Id::TextDecorationLine => reset.text_decoration = from_reset.text_decoration,
            Id::MarginTop => reset.margin.top = from_reset.margin.top,
            Id::MarginRight => reset.margin.right = from_reset.margin.right,
            Id::MarginBottom => reset.margin.bottom = from_reset.margin.bottom,
            Id::MarginLeft => reset.margin.left = from_reset.margin.left,
            Id::PaddingTop => reset.padding.top = from_reset.padding.top,
            Id::PaddingRight => reset.padding.right = from_reset.padding.right,
            Id::PaddingBottom => reset.padding.bottom = from_reset.padding.bottom,
            Id::PaddingLeft => reset.padding.left = from_reset.padding.left,
            Id::BorderTopWidth => reset.border.top = from_reset.border.top,
            Id::BorderRightWidth => reset.border.right = from_reset.border.right,
            Id::BorderBottomWidth => reset.border.bottom = from_reset.border.bottom,
            Id::BorderLeftWidth => reset.border.left = from_reset.border.left,
            Id::BorderTopColor => reset.border_color.top = from_reset.border_color.top,
            Id::BorderRightColor => reset.border_color.right = from_reset.border_color.right,
            Id::BorderBottomColor => reset.border_color.bottom = from_reset.border_color.bottom,
            Id::BorderLeftColor => reset.border_color.left = from_reset.border_color.left,
            Id::Width => reset.width = from_reset.width,
            Id::Height => reset.height = from_reset.height,
            Id::MinWidth => reset.min_width = from_reset.min_width,
            Id::MaxWidth => reset.max_width = from_reset.max_width,
            _ => {}
        }
    }

    fn finish(&mut self) {
        if self.context.root
            && matches!(
                self.reset.display,
                Display::Inline | Display::InlineBlock | Display::Contents
            )
        {
            self.reset.display = Display::Block;
        }
        for (width, visible) in [
            (&mut self.reset.border.top, self.border_styles[0]),
            (&mut self.reset.border.right, self.border_styles[1]),
            (&mut self.reset.border.bottom, self.border_styles[2]),
            (&mut self.reset.border.left, self.border_styles[3]),
        ] {
            if !visible {
                *width = 0.0;
            }
        }
        let own = self.reset.text_decoration;
        let decoration = self.decoration_color.unwrap_or(self.inherited.color);
        let decorations = &mut self.inherited.decorations;
        for (line, propagated) in [
            (own.underline, &mut decorations.underline),
            (own.overline, &mut decorations.overline),
            (own.line_through, &mut decorations.line_through),
        ] {
            if line.is_some() {
                *propagated = Some(decoration);
            }
        }
        let text_decoration = &mut self.reset.text_decoration;
        for line in [
            &mut text_decoration.underline,
            &mut text_decoration.overline,
            &mut text_decoration.line_through,
        ] {
            if line.is_some() {
                *line = Some(decoration);
            }
        }
    }
}
