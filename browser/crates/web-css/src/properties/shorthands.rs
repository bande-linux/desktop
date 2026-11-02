use super::*;
use crate::values::{Range, parse_number};

fn value(id: LonghandId, input: &mut Parser<'_, '_>) -> ParseResult<PropertyValue> {
    input.try_parse(|p| parse_longhand(id, p))
}

pub fn four_sides(
    input: &mut Parser<'_, '_>,
    ids: &[LonghandId],
    out: &mut Expanded,
) -> ParseResult<()> {
    let first = ids.first().copied().ok_or(ParseError::Invalid)?;
    let mut values = vec![value(first, input)?];
    while values.len() < 4 {
        match value(first, input) {
            Ok(v) => values.push(v),
            Err(_) => break,
        }
    }
    let pick = |indices: [usize; 4]| indices.map(|i| values.get(i).cloned());
    let sides = match values.len() {
        1 => pick([0, 0, 0, 0]),
        2 => pick([0, 1, 0, 1]),
        3 => pick([0, 1, 2, 1]),
        _ => pick([0, 1, 2, 3]),
    };
    for (id, side) in ids.iter().zip(sides) {
        out.push((*id, Some(side.ok_or(ParseError::Invalid)?)));
    }
    Ok(())
}

pub fn pair(input: &mut Parser<'_, '_>, ids: &[LonghandId], out: &mut Expanded) -> ParseResult<()> {
    let [first_id, second_id] = ids else {
        return Err(ParseError::Invalid);
    };
    let first = value(*first_id, input)?;
    let second = value(*second_id, input).unwrap_or_else(|_| first.clone());
    out.push((*first_id, Some(first)));
    out.push((*second_id, Some(second)));
    Ok(())
}

pub fn any_order(
    input: &mut Parser<'_, '_>,
    ids: &[LonghandId],
    out: &mut Expanded,
) -> ParseResult<()> {
    let mut values: Vec<Option<PropertyValue>> = vec![None; ids.len()];
    loop {
        let parsed = ids
            .iter()
            .zip(values.iter_mut())
            .find_map(|(id, slot)| match slot {
                None => value(*id, input).ok().map(|v| *slot = Some(v)),
                Some(_) => None,
            });
        if parsed.is_none() {
            break;
        }
    }
    if values.iter().all(Option::is_none) {
        return Err(ParseError::Invalid);
    }
    out.extend(ids.iter().copied().zip(values));
    Ok(())
}

pub fn border(input: &mut Parser<'_, '_>, out: &mut Expanded) -> ParseResult<()> {
    let ids = [
        LonghandId::BorderTopWidth,
        LonghandId::BorderTopStyle,
        LonghandId::BorderTopColor,
    ];
    let mut side = Expanded::new();
    any_order(input, &ids, &mut side)?;
    let groups = [
        ShorthandId::BorderWidth,
        ShorthandId::BorderStyle,
        ShorthandId::BorderColor,
    ];
    for ((_, value), group) in side.into_iter().zip(groups) {
        for id in group.longhands() {
            out.push((*id, value.clone()));
        }
    }
    Ok(())
}

pub fn border_radius(input: &mut Parser<'_, '_>, out: &mut Expanded) -> ParseResult<()> {
    let corners = |p: &mut Parser<'_, '_>| -> ParseResult<[LengthPercentage; 4]> {
        let mut values = vec![LengthPercentage::parse(p, Range::NonNegative)?];
        while values.len() < 4 {
            match p.try_parse(|p| LengthPercentage::parse(p, Range::NonNegative)) {
                Ok(v) => values.push(v),
                Err(_) => break,
            }
        }
        let get = |i: usize| values.get(i).cloned().unwrap_or(LengthPercentage::ZERO);
        Ok(match values.len() {
            1 => [get(0), get(0), get(0), get(0)],
            2 => [get(0), get(1), get(0), get(1)],
            3 => [get(0), get(1), get(2), get(1)],
            _ => [get(0), get(1), get(2), get(3)],
        })
    };
    let horizontal = corners(input)?;
    let vertical = if input.try_delim('/') {
        corners(input)?
    } else {
        horizontal.clone()
    };
    for ((id, h), v) in ShorthandId::BorderRadius
        .longhands()
        .iter()
        .zip(horizontal)
        .zip(vertical)
    {
        out.push((*id, Some(PropertyValue::BorderRadius(BorderRadius(h, v)))));
    }
    Ok(())
}

pub fn flex(input: &mut Parser<'_, '_>, out: &mut Expanded) -> ParseResult<()> {
    let basis_auto = || PropertyValue::FlexBasis(FlexBasis::Size(Size::Auto));
    let (grow, shrink, basis) = if input.try_ident_matching("none") {
        (0.0, 0.0, basis_auto())
    } else if input.try_ident_matching("auto") {
        (1.0, 1.0, basis_auto())
    } else {
        let mut factors = None;
        let mut basis = None;
        for _ in 0..2 {
            if factors.is_none()
                && let Ok(grow) = input.try_parse(|p| parse_number(p, Range::NonNegative))
            {
                let shrink = input
                    .try_parse(|p| parse_number(p, Range::NonNegative))
                    .unwrap_or(1.0);
                factors = Some((grow, shrink));
                continue;
            }
            if basis.is_none()
                && let Ok(b) = value(LonghandId::FlexBasis, input)
            {
                basis = Some(b);
                continue;
            }
            break;
        }
        if factors.is_none() && basis.is_none() {
            return Err(ParseError::Invalid);
        }
        let (grow, shrink) = factors.unwrap_or((1.0, 1.0));
        let basis = basis.unwrap_or(PropertyValue::FlexBasis(FlexBasis::Size(
            Size::LengthPercentage(LengthPercentage::Percentage(0.0)),
        )));
        (grow, shrink, basis)
    };
    out.push((
        LonghandId::FlexGrow,
        Some(PropertyValue::NonNegativeNumber(NonNegativeNumber(grow))),
    ));
    out.push((
        LonghandId::FlexShrink,
        Some(PropertyValue::NonNegativeNumber(NonNegativeNumber(shrink))),
    ));
    out.push((LonghandId::FlexBasis, Some(basis)));
    Ok(())
}

pub fn list_style(input: &mut Parser<'_, '_>, out: &mut Expanded) -> ParseResult<()> {
    let mut position = None;
    let mut image = None;
    let mut kind = None;
    let mut nones = 0;
    loop {
        if input.try_ident_matching("none") {
            nones += 1;
            continue;
        }
        if position.is_none()
            && let Ok(v) = value(LonghandId::ListStylePosition, input)
        {
            position = Some(v);
            continue;
        }
        if image.is_none()
            && let Ok(v) = value(LonghandId::ListStyleImage, input)
        {
            image = Some(v);
            continue;
        }
        if kind.is_none()
            && let Ok(v) = value(LonghandId::ListStyleType, input)
        {
            kind = Some(v);
            continue;
        }
        break;
    }
    let none_image = || Some(PropertyValue::Image(Image::None));
    let none_type = || Some(PropertyValue::ListStyleType(ListStyleType::None));
    match (nones, image.is_some(), kind.is_some()) {
        (0, _, _) => {}
        (1, false, true) => image = none_image(),
        (1, true, false) => kind = none_type(),
        (1, false, false) | (2, false, false) => {
            image = none_image();
            kind = none_type();
        }
        _ => return Err(ParseError::Invalid),
    }
    if nones == 0 && position.is_none() && image.is_none() && kind.is_none() {
        return Err(ParseError::Invalid);
    }
    out.push((LonghandId::ListStylePosition, position));
    out.push((LonghandId::ListStyleImage, image));
    out.push((LonghandId::ListStyleType, kind));
    Ok(())
}

pub fn white_space(input: &mut Parser<'_, '_>, out: &mut Expanded) -> ParseResult<()> {
    let ident = input.expect_ident()?.to_ascii_lowercase();
    let (collapse, wrap) = match ident.as_str() {
        "normal" => (WhiteSpaceCollapse::Collapse, TextWrapMode::Wrap),
        "pre" => (WhiteSpaceCollapse::Preserve, TextWrapMode::Nowrap),
        "nowrap" => (WhiteSpaceCollapse::Collapse, TextWrapMode::Nowrap),
        "pre-wrap" => (WhiteSpaceCollapse::Preserve, TextWrapMode::Wrap),
        "pre-line" => (WhiteSpaceCollapse::PreserveBreaks, TextWrapMode::Wrap),
        "break-spaces" => (WhiteSpaceCollapse::BreakSpaces, TextWrapMode::Wrap),
        _ => return Err(ParseError::Invalid),
    };
    out.push((
        LonghandId::WhiteSpaceCollapse,
        Some(PropertyValue::WhiteSpaceCollapse(collapse)),
    ));
    out.push((
        LonghandId::TextWrapMode,
        Some(PropertyValue::TextWrapMode(wrap)),
    ));
    Ok(())
}

pub fn background_position(input: &mut Parser<'_, '_>, out: &mut Expanded) -> ParseResult<()> {
    let positions = input.parse_comma_separated(Position::parse)?;
    let (x, y): (Vec<_>, Vec<_>) = positions.into_iter().map(|p| (p.x, p.y)).unzip();
    out.push((
        LonghandId::BackgroundPositionX,
        Some(PropertyValue::HorizontalPositionList(
            HorizontalPositionList(x.into_boxed_slice()),
        )),
    ));
    out.push((
        LonghandId::BackgroundPositionY,
        Some(PropertyValue::VerticalPositionList(VerticalPositionList(
            y.into_boxed_slice(),
        ))),
    ));
    Ok(())
}

#[derive(Default)]
struct Layer {
    image: Option<Image>,
    position: Option<Position>,
    size: Option<BackgroundSize>,
    repeat: Option<BackgroundRepeat>,
    attachment: Option<BackgroundAttachment>,
    origin: Option<BackgroundBox>,
    clip: Option<BackgroundBox>,
}

pub fn background(input: &mut Parser<'_, '_>, out: &mut Expanded) -> ParseResult<()> {
    let mut color = None;
    let layers = input.parse_comma_separated(|p| {
        let mut layer = Layer::default();
        let mut any = false;
        loop {
            if layer.image.is_none()
                && let Ok(image) = p.try_parse(<Image as Parse>::parse)
            {
                layer.image = Some(image);
            } else if layer.position.is_none()
                && let Ok(position) = p.try_parse(Position::parse)
            {
                layer.position = Some(position);
                if p.try_delim('/') {
                    layer.size = Some(BackgroundSize::parse_one(p)?);
                }
            } else if layer.repeat.is_none()
                && let Ok(repeat) = p.try_parse(BackgroundRepeat::parse_one)
            {
                layer.repeat = Some(repeat);
            } else if layer.attachment.is_none()
                && let Ok(attachment) = p.try_parse(<BackgroundAttachment as Parse>::parse)
            {
                layer.attachment = Some(attachment);
            } else if layer.clip.is_none()
                && let Ok(b) = p.try_parse(<BackgroundBox as Parse>::parse)
            {
                if layer.origin.is_none() {
                    layer.origin = Some(b);
                } else {
                    layer.clip = Some(b);
                }
            } else if color.is_none()
                && let Ok(c) = p.try_parse(Color::parse)
            {
                color = Some(c);
            } else {
                break;
            }
            any = true;
        }
        if !any && color.is_none() {
            return Err(ParseError::Invalid);
        }
        if layer.clip.is_none() {
            layer.clip = layer.origin;
        }
        Ok(layer)
    })?;
    let mut images = vec![];
    let mut xs = vec![];
    let mut ys = vec![];
    let mut sizes = vec![];
    let mut repeats = vec![];
    let mut attachments = vec![];
    let mut origins = vec![];
    let mut clips = vec![];
    for layer in layers {
        images.push(layer.image.unwrap_or(Image::None));
        let position = layer.position.unwrap_or(Position {
            x: PositionComponent::Length(LengthPercentage::Percentage(0.0)),
            y: PositionComponent::Length(LengthPercentage::Percentage(0.0)),
        });
        xs.push(position.x);
        ys.push(position.y);
        sizes.push(layer.size.unwrap_or(BackgroundSize::Explicit(
            LengthPercentageOrAuto::Auto,
            LengthPercentageOrAuto::Auto,
        )));
        repeats.push(
            layer
                .repeat
                .unwrap_or(BackgroundRepeat(RepeatStyle::Repeat, RepeatStyle::Repeat)),
        );
        attachments.push(layer.attachment.unwrap_or(BackgroundAttachment::Scroll));
        origins.push(layer.origin.unwrap_or(BackgroundBox::PaddingBox));
        clips.push(layer.clip.unwrap_or(BackgroundBox::BorderBox));
    }
    out.push((
        LonghandId::BackgroundColor,
        Some(PropertyValue::Color(color.unwrap_or(Color::TRANSPARENT))),
    ));
    out.push((
        LonghandId::BackgroundImage,
        Some(PropertyValue::ImageList(ImageList(images.into()))),
    ));
    out.push((
        LonghandId::BackgroundRepeat,
        Some(PropertyValue::BackgroundRepeatList(BackgroundRepeatList(
            repeats.into(),
        ))),
    ));
    out.push((
        LonghandId::BackgroundPositionX,
        Some(PropertyValue::HorizontalPositionList(
            HorizontalPositionList(xs.into()),
        )),
    ));
    out.push((
        LonghandId::BackgroundPositionY,
        Some(PropertyValue::VerticalPositionList(VerticalPositionList(
            ys.into(),
        ))),
    ));
    out.push((
        LonghandId::BackgroundSize,
        Some(PropertyValue::BackgroundSizeList(BackgroundSizeList(
            sizes.into(),
        ))),
    ));
    out.push((
        LonghandId::BackgroundAttachment,
        Some(PropertyValue::BackgroundAttachmentList(
            BackgroundAttachmentList(attachments.into()),
        )),
    ));
    out.push((
        LonghandId::BackgroundOrigin,
        Some(PropertyValue::BackgroundBoxList(BackgroundBoxList(
            origins.into(),
        ))),
    ));
    out.push((
        LonghandId::BackgroundClip,
        Some(PropertyValue::BackgroundBoxList(BackgroundBoxList(
            clips.into(),
        ))),
    ));
    Ok(())
}

const SYSTEM_FONTS: &[&str] = &[
    "caption",
    "icon",
    "menu",
    "message-box",
    "small-caption",
    "status-bar",
];

pub fn font(input: &mut Parser<'_, '_>, out: &mut Expanded) -> ParseResult<()> {
    if let Ok(ident) = input.try_parse(|p| {
        let ident = p.expect_ident()?;
        p.expect_exhausted()?;
        Ok(ident)
    }) {
        if !SYSTEM_FONTS.iter().any(|s| ident.eq_ignore_ascii_case(s)) {
            return Err(ParseError::Invalid);
        }
        for id in ShorthandId::Font.longhands() {
            let value = (*id == LonghandId::FontFamily).then(|| {
                PropertyValue::FontFamily(FontFamily(Box::new([FamilyName::Generic(
                    GenericFamily::SystemUi,
                )])))
            });
            out.push((*id, value));
        }
        return Ok(());
    }
    let mut style = None;
    let mut caps = None;
    let mut weight = None;
    let mut stretch = None;
    for _ in 0..4 {
        if input.try_ident_matching("normal") {
            continue;
        }
        if style.is_none()
            && let Ok(v) = value(LonghandId::FontStyle, input)
        {
            style = Some(v);
            continue;
        }
        if caps.is_none() && input.try_ident_matching("small-caps") {
            caps = Some(PropertyValue::FontVariantCaps(FontVariantCaps::SmallCaps));
            continue;
        }
        if weight.is_none()
            && let Ok(v) = value(LonghandId::FontWeight, input)
        {
            weight = Some(v);
            continue;
        }
        if stretch.is_none()
            && let Ok(v) = input.try_parse(|p| {
                let ident = p.expect_ident()?;
                let mut tokenized = Tokenizer::new(&ident);
                let mut inner = Parser::new(&mut tokenized);
                <FontStretch as Parse>::parse(&mut inner).map(PropertyValue::FontStretch)
            })
        {
            stretch = Some(v);
            continue;
        }
        break;
    }
    let size = value(LonghandId::FontSize, input)?;
    let line_height = if input.try_delim('/') {
        Some(value(LonghandId::LineHeight, input)?)
    } else {
        None
    };
    let family = value(LonghandId::FontFamily, input)?;
    for (id, v) in [
        (LonghandId::FontStyle, style),
        (LonghandId::FontVariantCaps, caps),
        (LonghandId::FontWeight, weight),
        (LonghandId::FontStretch, stretch),
        (LonghandId::FontSize, Some(size)),
        (LonghandId::LineHeight, line_height),
        (LonghandId::FontFamily, Some(family)),
    ] {
        out.push((id, v));
    }
    Ok(())
}

fn line_pair(
    input: &mut Parser<'_, '_>,
    ids: [LonghandId; 2],
    out: &mut Expanded,
) -> ParseResult<()> {
    let start = <GridLine as Parse>::parse(input)?;
    let end = if input.try_delim('/') {
        <GridLine as Parse>::parse(input)?
    } else if let GridLine::Name(name) = &start {
        GridLine::Name(name.clone())
    } else {
        GridLine::Auto
    };
    out.push((ids[0], Some(PropertyValue::GridLine(start))));
    out.push((ids[1], Some(PropertyValue::GridLine(end))));
    Ok(())
}

pub fn grid_column(input: &mut Parser<'_, '_>, out: &mut Expanded) -> ParseResult<()> {
    line_pair(
        input,
        [LonghandId::GridColumnStart, LonghandId::GridColumnEnd],
        out,
    )
}

pub fn grid_row(input: &mut Parser<'_, '_>, out: &mut Expanded) -> ParseResult<()> {
    line_pair(
        input,
        [LonghandId::GridRowStart, LonghandId::GridRowEnd],
        out,
    )
}

pub fn grid_area(input: &mut Parser<'_, '_>, out: &mut Expanded) -> ParseResult<()> {
    let mut lines = vec![<GridLine as Parse>::parse(input)?];
    while lines.len() < 4 && input.try_delim('/') {
        lines.push(<GridLine as Parse>::parse(input)?);
    }
    let copy = |line: &GridLine| match line {
        GridLine::Name(name) => GridLine::Name(name.clone()),
        _ => GridLine::Auto,
    };
    let row_start = lines.first().cloned().unwrap_or(GridLine::Auto);
    let column_start = lines.get(1).cloned().unwrap_or_else(|| copy(&row_start));
    let row_end = lines.get(2).cloned().unwrap_or_else(|| copy(&row_start));
    let column_end = lines.get(3).cloned().unwrap_or_else(|| copy(&column_start));
    for (id, line) in [
        (LonghandId::GridRowStart, row_start),
        (LonghandId::GridColumnStart, column_start),
        (LonghandId::GridRowEnd, row_end),
        (LonghandId::GridColumnEnd, column_end),
    ] {
        out.push((id, Some(PropertyValue::GridLine(line))));
    }
    Ok(())
}

pub fn grid_template(input: &mut Parser<'_, '_>, out: &mut Expanded) -> ParseResult<()> {
    if input.try_ident_matching("none") {
        out.push((
            LonghandId::GridTemplateRows,
            Some(PropertyValue::GridTemplate(GridTemplate::None)),
        ));
        out.push((
            LonghandId::GridTemplateColumns,
            Some(PropertyValue::GridTemplate(GridTemplate::None)),
        ));
        out.push((
            LonghandId::GridTemplateAreas,
            Some(PropertyValue::GridAreas(GridAreas::None)),
        ));
        return Ok(());
    }
    let rows = <GridTemplate as Parse>::parse(input)?;
    if !input.try_delim('/') {
        return Err(ParseError::Invalid);
    }
    let columns = <GridTemplate as Parse>::parse(input)?;
    out.push((
        LonghandId::GridTemplateRows,
        Some(PropertyValue::GridTemplate(rows)),
    ));
    out.push((
        LonghandId::GridTemplateColumns,
        Some(PropertyValue::GridTemplate(columns)),
    ));
    out.push((
        LonghandId::GridTemplateAreas,
        Some(PropertyValue::GridAreas(GridAreas::None)),
    ));
    Ok(())
}
