use web_atoms::Atom;
use web_dom::{Document, Namespace, NodeId};

use crate::{
    Color, Display, FontFamily, InheritedStyle, ListStyleType, ResetStyle, TextAlign,
    VerticalAlign, WhiteSpace, is_html,
};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Declaration {
    Display(Display),
    FontSizeScale(f32),
    MarginBlockEm(f32),
    MarginInlinePx(f32),
    MarginLeftPx(f32),
    MarginPx(f32),
    PaddingLeftPx(f32),
    BorderBlockPx(f32, Color),
    Bold,
    Bolder,
    Italic,
    Family(FontFamily),
    WhiteSpace(WhiteSpace),
    Color(Color),
    Background(Color),
    Underline,
    LineThrough,
    ListStyle(ListStyleType),
    VerticalAlign(VerticalAlign),
    TextAlign(TextAlign),
}

const MONOSPACE_SCALE: f32 = 13.0 / 16.0;

impl Declaration {
    pub fn apply(
        self,
        inherited: &mut InheritedStyle,
        reset: &mut ResetStyle,
        parent: &InheritedStyle,
    ) {
        let em = inherited.font_size;
        match self {
            Self::Display(display) => reset.display = display,
            Self::FontSizeScale(scale) => inherited.font_size = parent.font_size * scale,
            Self::MarginBlockEm(value) => {
                reset.margin.top = value * em;
                reset.margin.bottom = value * em;
            }
            Self::MarginInlinePx(value) => {
                reset.margin.left = value;
                reset.margin.right = value;
            }
            Self::MarginLeftPx(value) => reset.margin.left = value,
            Self::MarginPx(value) => {
                reset.margin.top = value;
                reset.margin.right = value;
                reset.margin.bottom = value;
                reset.margin.left = value;
            }
            Self::PaddingLeftPx(value) => reset.padding.left = value,
            Self::BorderBlockPx(width, color) => {
                reset.border.top = width;
                reset.border.bottom = width;
                reset.border_color = color;
            }
            Self::Bold => inherited.font_weight = 700,
            Self::Bolder => {
                inherited.font_weight = match parent.font_weight {
                    0..350 => 400,
                    350..550 => 700,
                    _ => 900,
                }
            }
            Self::Italic => inherited.italic = true,
            Self::Family(family) => {
                if family == FontFamily::Monospace && parent.font_family != FontFamily::Monospace {
                    inherited.font_size *= MONOSPACE_SCALE;
                }
                inherited.font_family = family;
            }
            Self::WhiteSpace(white_space) => inherited.white_space = white_space,
            Self::Color(color) => inherited.color = color,
            Self::Background(color) => reset.background_color = color,
            Self::Underline => inherited.decorations.underline = Some(inherited.color),
            Self::LineThrough => inherited.decorations.line_through = Some(inherited.color),
            Self::ListStyle(list_style) => inherited.list_style_type = list_style,
            Self::VerticalAlign(align) => reset.vertical_align = align,
            Self::TextAlign(align) => inherited.text_align = align,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum RuleKey {
    Hidden,
    Html(Atom),
    Link,
    List(Atom, u8),
}

pub(crate) fn rule_key(document: &Document, node: NodeId) -> RuleKey {
    let Some((namespace, name)) = document.element_name(node) else {
        return RuleKey::Hidden;
    };
    if namespace == Namespace::Svg || document.attribute(node, Atom::HIDDEN).is_some() {
        return RuleKey::Hidden;
    }
    if namespace == Namespace::MathMl {
        return RuleKey::Html(Atom::MATH);
    }
    match name {
        Atom::A if document.attribute(node, Atom::HREF).is_some() => RuleKey::Link,
        Atom::UL | Atom::OL | Atom::MENU | Atom::DIR => {
            let depth = document
                .ancestors(node)
                .skip(1)
                .filter(|&ancestor| {
                    [Atom::UL, Atom::OL, Atom::MENU, Atom::DIR]
                        .iter()
                        .any(|&list| is_html(document, ancestor, list))
                })
                .take(2)
                .count();
            RuleKey::List(name, depth as u8)
        }
        Atom::INPUT => RuleKey::Hidden,
        _ => RuleKey::Html(name),
    }
}

use Declaration::*;

const BLOCK: &[Declaration] = &[Display(Display::Block)];
const NONE: &[Declaration] = &[Display(Display::None)];
const PARAGRAPH: &[Declaration] = &[Display(Display::Block), MarginBlockEm(1.0)];
const INDENTED: &[Declaration] = &[
    Display(Display::Block),
    MarginBlockEm(1.0),
    MarginInlinePx(40.0),
];
const PREFORMATTED: &[Declaration] = &[
    Display(Display::Block),
    Family(FontFamily::Monospace),
    WhiteSpace(WhiteSpace::Pre),
    MarginBlockEm(1.0),
];
const LINK_BLUE: Color = Color::rgb(0, 0, 238);
const RULE_GRAY: Color = Color::rgb(128, 128, 128);
const MARK_YELLOW: Color = Color::rgb(255, 255, 0);

const fn heading(size: f32, margin: f32) -> [Declaration; 4] {
    [
        Display(Display::Block),
        FontSizeScale(size),
        MarginBlockEm(margin),
        Bold,
    ]
}

const H1: [Declaration; 4] = heading(2.0, 0.67);
const H2: [Declaration; 4] = heading(1.5, 0.83);
const H3: [Declaration; 4] = heading(1.17, 1.0);
const H4: [Declaration; 4] = heading(1.0, 1.33);
const H5: [Declaration; 4] = heading(0.83, 1.67);
const H6: [Declaration; 4] = heading(0.67, 2.33);

pub(crate) fn declarations(key: RuleKey) -> &'static [Declaration] {
    match key {
        RuleKey::Hidden => NONE,
        RuleKey::Link => &[Color(LINK_BLUE), Underline],
        RuleKey::List(Atom::OL, 0) => &[
            Display(Display::Block),
            MarginBlockEm(1.0),
            PaddingLeftPx(40.0),
            ListStyle(ListStyleType::Decimal),
        ],
        RuleKey::List(Atom::OL, _) => &[
            Display(Display::Block),
            PaddingLeftPx(40.0),
            ListStyle(ListStyleType::Decimal),
        ],
        RuleKey::List(_, 0) => &[
            Display(Display::Block),
            MarginBlockEm(1.0),
            PaddingLeftPx(40.0),
            ListStyle(ListStyleType::Disc),
        ],
        RuleKey::List(_, 1) => &[
            Display(Display::Block),
            PaddingLeftPx(40.0),
            ListStyle(ListStyleType::Circle),
        ],
        RuleKey::List(..) => &[
            Display(Display::Block),
            PaddingLeftPx(40.0),
            ListStyle(ListStyleType::Square),
        ],
        RuleKey::Html(name) => element(name),
    }
}

fn element(name: Atom) -> &'static [Declaration] {
    match name {
        Atom::HEAD
        | Atom::SCRIPT
        | Atom::STYLE
        | Atom::TITLE
        | Atom::META
        | Atom::LINK
        | Atom::BASE
        | Atom::TEMPLATE
        | Atom::AREA
        | Atom::DATALIST
        | Atom::PARAM
        | Atom::RP
        | Atom::NOEMBED
        | Atom::NOFRAMES
        | Atom::SELECT
        | Atom::DIALOG
        | Atom::IFRAME
        | Atom::VIDEO
        | Atom::AUDIO
        | Atom::CANVAS
        | Atom::EMBED
        | Atom::TRACK
        | Atom::SOURCE => NONE,
        Atom::HTML
        | Atom::ADDRESS
        | Atom::ARTICLE
        | Atom::ASIDE
        | Atom::DIV
        | Atom::DT
        | Atom::FIGCAPTION
        | Atom::FOOTER
        | Atom::FORM
        | Atom::HEADER
        | Atom::HGROUP
        | Atom::LEGEND
        | Atom::MAIN
        | Atom::NAV
        | Atom::SEARCH
        | Atom::SECTION
        | Atom::DETAILS
        | Atom::SUMMARY
        | Atom::FIELDSET
        | Atom::TABLE
        | Atom::THEAD
        | Atom::TBODY
        | Atom::TFOOT
        | Atom::TR
        | Atom::TD
        | Atom::COLGROUP
        | Atom::FRAMESET
        | Atom::FRAME => BLOCK,
        Atom::BODY => &[Display(Display::Block), MarginPx(8.0)],
        Atom::P | Atom::DL => PARAGRAPH,
        Atom::BLOCKQUOTE | Atom::FIGURE => INDENTED,
        Atom::DD => &[Display(Display::Block), MarginLeftPx(40.0)],
        Atom::PRE | Atom::LISTING | Atom::XMP | Atom::PLAINTEXT => PREFORMATTED,
        Atom::LI => &[Display(Display::ListItem)],
        Atom::CENTER => &[Display(Display::Block), TextAlign(TextAlign::Center)],
        Atom::CAPTION => &[Display(Display::Block), TextAlign(TextAlign::Center)],
        Atom::TH => &[Display(Display::Block), Bold, TextAlign(TextAlign::Center)],
        Atom::H1 => &H1,
        Atom::H2 => &H2,
        Atom::H3 => &H3,
        Atom::H4 => &H4,
        Atom::H5 => &H5,
        Atom::H6 => &H6,
        Atom::HR => &[
            Display(Display::Block),
            MarginBlockEm(0.5),
            Color(RULE_GRAY),
            BorderBlockPx(1.0, RULE_GRAY),
        ],
        Atom::B | Atom::STRONG => &[Bolder],
        Atom::I | Atom::EM | Atom::CITE | Atom::VAR | Atom::DFN => &[Italic],
        Atom::CODE | Atom::KBD | Atom::SAMP | Atom::TT => &[Family(FontFamily::Monospace)],
        Atom::U | Atom::INS => &[Underline],
        Atom::S | Atom::STRIKE | Atom::DEL => &[LineThrough],
        Atom::SMALL => &[FontSizeScale(1.0 / 1.2)],
        Atom::BIG => &[FontSizeScale(1.2)],
        Atom::SUB => &[
            FontSizeScale(1.0 / 1.2),
            VerticalAlign(VerticalAlign::Sub),
        ],
        Atom::SUP => &[
            FontSizeScale(1.0 / 1.2),
            VerticalAlign(VerticalAlign::Super),
        ],
        Atom::MARK => &[Background(MARK_YELLOW), Color(Color::BLACK)],
        Atom::NOBR => &[WhiteSpace(WhiteSpace::Nowrap)],
        Atom::TEXTAREA => &[WhiteSpace(WhiteSpace::PreWrap)],
        _ => &[],
    }
}
