use web_content::{Fonts, render_html};
use web_paint::Item;

const ARTICLE: &str = "<!doctype html><title> An  article </title>\
    <h1>Heading</h1>\
    <p>Some text with <a href=\"/next\">a link</a> inside a paragraph long enough to wrap \
    when the viewport is narrow, and then some more words.</p>";

#[test]
fn an_article_lays_out_and_its_link_is_found_under_its_text() -> Result<(), String> {
    let mut fonts = Fonts::new();
    let mut page = render_html(ARTICLE, 800.0, &mut fonts);
    assert_eq!(page.title().as_deref(), Some("An article"));
    let text_rect = |page: &web_content::Page, wanted: &str| {
        page.display_list.items.iter().find_map(|item| match item {
            Item::Glyphs {
                rect,
                node: Some(node),
                ..
            } if page.document.text(*node) == Some(wanted) => Some(*rect),
            _ => None,
        })
    };
    let heading = text_rect(&page, "Heading").ok_or("no heading text")?;
    let link = text_rect(&page, "a link").ok_or("no link text")?;
    assert!(heading.y + heading.height < link.y);
    assert!(heading.height > link.height);
    let center = (link.x + link.width / 2.0, link.y + link.height / 2.0);
    assert_eq!(page.link_at(center.0, center.1), Some("/next"));
    assert_eq!(page.link_at(center.0, heading.y + 1.0), None);
    let wide = page.display_list.height;
    page.resize(200.0, &mut fonts);
    assert!(page.display_list.height > wide);
    Ok(())
}

const HOVER: &str = "<!doctype html><style>\
    p { color: rgb(0, 0, 0) } a:hover { color: rgb(255, 0, 0) } .note b { color: rgb(0, 128, 0) }\
    </style>\
    <p>First <a href=\"/one\">one</a> and <a href=\"/two\">two</a>.</p>\
    <p class=\"note\">A <b>bold</b> note.</p>";

fn glyph_color(
    page: &web_content::Page,
    wanted: &str,
) -> Option<(web_paint::Rect, web_paint::Color)> {
    page.display_list.items.iter().find_map(|item| match item {
        Item::Glyphs {
            rect,
            node: Some(node),
            color,
            ..
        } if page.document.text(*node) == Some(wanted) => Some((*rect, *color)),
        _ => None,
    })
}

#[test]
fn author_styles_apply_and_hovering_a_link_restyles_only_that_link() -> Result<(), String> {
    let mut fonts = Fonts::new();
    let mut page = render_html(HOVER, 800.0, &mut fonts);
    let (_, bold) = glyph_color(&page, "bold").ok_or("no bold text")?;
    assert_eq!((bold.r, bold.g, bold.b), (0, 128, 0));
    let (one, before) = glyph_color(&page, "one").ok_or("no link text")?;
    assert_eq!((before.r, before.g, before.b), (0, 0, 238));
    assert!(page.hover_at(
        one.x + one.width / 2.0,
        one.y + one.height / 2.0,
        &mut fonts
    ));
    assert_eq!(page.restyled(), 1);
    let (_, hovered) = glyph_color(&page, "one").ok_or("no link text")?;
    assert_eq!((hovered.r, hovered.g, hovered.b), (255, 0, 0));
    let (_, other) = glyph_color(&page, "two").ok_or("no second link")?;
    assert_eq!((other.r, other.g, other.b), (0, 0, 238));
    assert!(page.hover_at(1.0, 1.0, &mut fonts));
    let (_, after) = glyph_color(&page, "one").ok_or("no link text")?;
    assert_eq!((after.r, after.g, after.b), (0, 0, 238));
    Ok(())
}
