use web_atoms::{Atom, Namespace};
use web_css::properties::{DeclaredValue, LonghandId, PropertyId, parse_value_str};
use web_dom::{Document, NodeId};

fn dimension(value: &str) -> Option<String> {
    let value = value.trim_start_matches([' ', '\t', '\n', '\x0c', '\r']);
    let digits = value
        .bytes()
        .take_while(|byte| byte.is_ascii_digit() || *byte == b'.')
        .count();
    let number: f32 = value.get(..digits)?.trim_end_matches('.').parse().ok()?;
    if value.as_bytes().get(digits) == Some(&b'%') {
        Some(format!("{number}%"))
    } else {
        Some(format!("{number}px"))
    }
}

fn nonzero_dimension(value: &str) -> Option<String> {
    dimension(value).filter(|dimension| !dimension.starts_with('0') || dimension.starts_with("0."))
}

fn integer(value: &str) -> Option<u32> {
    let value = value.trim();
    let digits = value.bytes().take_while(u8::is_ascii_digit).count();
    value.get(..digits)?.parse().ok()
}

fn legacy_color(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("transparent") {
        return None;
    }
    let hex = value.strip_prefix('#').unwrap_or(value);
    if (hex.len() == 6 || hex.len() == 3) && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Some(format!("#{hex}"));
    }
    Some(value.to_owned())
}

fn table_of(document: &Document, cell: NodeId) -> Option<NodeId> {
    document
        .ancestors(cell)
        .skip(1)
        .find(|&node| document.html_element_name(node) == Some(Atom::TABLE))
}

pub(crate) fn hints(document: &Document, element: NodeId) -> Vec<(LonghandId, DeclaredValue)> {
    let Some((Namespace::Html, name)) = document.element_name(element) else {
        return Vec::new();
    };
    let attribute = |atom: Atom| document.attribute(element, atom);
    let mut css: Vec<(&str, String)> = Vec::new();
    let size = |css: &mut Vec<(&str, String)>| {
        if let Some(width) = attribute(Atom::WIDTH).and_then(nonzero_dimension) {
            css.push(("width", width));
        }
        if let Some(height) = attribute(Atom::HEIGHT).and_then(nonzero_dimension) {
            css.push(("height", height));
        }
    };
    if let Some(color) = attribute(Atom::BGCOLOR).and_then(legacy_color)
        && matches!(
            name,
            Atom::BODY
                | Atom::TABLE
                | Atom::TR
                | Atom::TD
                | Atom::TH
                | Atom::THEAD
                | Atom::TBODY
                | Atom::TFOOT
        )
    {
        css.push(("background-color", color));
    }
    match name {
        Atom::TABLE => {
            size(&mut css);
            if let Some(spacing) = attribute(Atom::CELLSPACING).and_then(dimension) {
                css.push(("border-spacing", spacing));
            }
            if let Some(border) = attribute(Atom::BORDER).map(|border| integer(border).unwrap_or(1))
                && border > 0
            {
                css.push(("border", format!("{border}px outset gray")));
            }
            match attribute(Atom::ALIGN)
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("center") => css.push(("margin-inline", "auto".to_owned())),
                Some("left") => css.push(("float", "left".to_owned())),
                Some("right") => css.push(("float", "right".to_owned())),
                _ => {}
            }
        }
        Atom::TD | Atom::TH => {
            size(&mut css);
            if let Some(table) = table_of(document, element) {
                if let Some(padding) = document
                    .attribute(table, Atom::CELLPADDING)
                    .and_then(dimension)
                {
                    css.push(("padding", padding));
                }
                if document
                    .attribute(table, Atom::BORDER)
                    .is_some_and(|border| integer(border).unwrap_or(1) > 0)
                {
                    css.push(("border", "1px inset gray".to_owned()));
                }
            }
            if attribute(Atom::NOWRAP).is_some() {
                css.push(("white-space", "nowrap".to_owned()));
            }
        }
        Atom::IMG | Atom::IFRAME | Atom::VIDEO | Atom::CANVAS | Atom::EMBED | Atom::OBJECT => {
            size(&mut css)
        }
        Atom::TR => {
            if let Some(height) = attribute(Atom::HEIGHT).and_then(nonzero_dimension) {
                css.push(("height", height));
            }
        }
        Atom::HR => {
            if let Some(width) = attribute(Atom::WIDTH).and_then(nonzero_dimension) {
                css.push(("width", width));
            }
            if attribute(Atom::NOSHADE).is_some() || attribute(Atom::COLOR).is_some() {
                css.push(("border-style", "solid".to_owned()));
            }
            if let Some(color) = attribute(Atom::COLOR).and_then(legacy_color) {
                css.push(("border-color", color.clone()));
                css.push(("background-color", color));
            }
        }
        Atom::FONT => {
            if let Some(color) = attribute(Atom::COLOR).and_then(legacy_color) {
                css.push(("color", color));
            }
            if let Some(face) = attribute(Atom::FACE) {
                css.push(("font-family", face.to_owned()));
            }
            if let Some(size) = attribute(Atom::SIZE).and_then(font_size) {
                css.push(("font-size", size.to_owned()));
            }
        }
        Atom::BODY => {
            if let Some(color) = document
                .attribute(element, Atom::TEXT)
                .and_then(legacy_color)
            {
                css.push(("color", color));
            }
        }
        _ => {}
    }
    if matches!(
        name,
        Atom::TD | Atom::TH | Atom::TR | Atom::THEAD | Atom::TBODY | Atom::TFOOT
    ) && let Some(align) = attribute(Atom::VALIGN)
    {
        css.push(("vertical-align", align.to_ascii_lowercase()));
    }
    if matches!(
        name,
        Atom::DIV
            | Atom::P
            | Atom::H1
            | Atom::H2
            | Atom::H3
            | Atom::H4
            | Atom::H5
            | Atom::H6
            | Atom::TD
            | Atom::TH
            | Atom::TR
            | Atom::THEAD
            | Atom::TBODY
            | Atom::TFOOT
            | Atom::CAPTION
    ) && let Some(align) = attribute(Atom::ALIGN)
    {
        let align = align.to_ascii_lowercase();
        let value = match align.as_str() {
            "left" | "right" | "center" | "justify" => Some(align.as_str()),
            "middle" => Some("center"),
            _ => None,
        };
        if let Some(value) = value {
            css.push(("text-align", value.to_owned()));
        }
    }
    css.into_iter()
        .filter_map(|(property, value)| {
            let id = PropertyId::from_name(property)?;
            parse_value_str(id, &value).ok()
        })
        .flatten()
        .collect()
}

fn font_size(value: &str) -> Option<&'static str> {
    let value = value.trim();
    let (relative, digits) = match value.as_bytes().first() {
        Some(b'+') => (1, value.get(1..)?),
        Some(b'-') => (-1, value.get(1..)?),
        _ => (0, value),
    };
    let number = i32::try_from(integer(digits)?).ok()?;
    let size = match relative {
        1 => 3 + number,
        -1 => 3 - number,
        _ => number,
    }
    .clamp(1, 7);
    Some(match size {
        1 => "x-small",
        2 => "small",
        3 => "medium",
        4 => "large",
        5 => "x-large",
        6 => "xx-large",
        _ => "xxx-large",
    })
}
