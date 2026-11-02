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
