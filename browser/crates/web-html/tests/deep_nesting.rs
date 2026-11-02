use web_dom::{Document, NodeId, NodeKind};
use web_html::{Options, Parser};

#[test]
fn deep_nesting_is_capped_and_keeps_content() {
    let mut parser = Parser::new(Document::new(), Options::default());
    parser.feed_str(&"<div>".repeat(100_000));
    parser.feed_str("x");
    let document = parser.finish();
    let mut pending = vec![NodeId::DOCUMENT];
    let mut text = None;
    while let Some(node) = pending.pop() {
        if document.kind(node) == Some(NodeKind::Text) {
            text = Some(node);
        }
        pending.extend(document.children(node));
    }
    let text = text.filter(|&node| document.text(node) == Some("x"));
    let depth = text.map(|node| document.ancestors(node).count());
    assert!(depth.is_some_and(|depth| depth <= 514), "depth {depth:?}");
}
