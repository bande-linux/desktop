use web_dom::{Document, NodeId};
use web_html::{Atom, Namespace, QualName, parse_xml};

fn elements(document: &Document) -> Vec<(NodeId, Namespace, String)> {
    let mut found = Vec::new();
    let mut pending = vec![NodeId::DOCUMENT];
    while let Some(node) = pending.pop() {
        if let Some((ns, local)) = document.element_name(node) {
            found.push((node, ns, document.atoms().name(local).to_owned()));
        }
        let start = pending.len();
        pending.extend(document.children(node));
        pending[start..].reverse();
    }
    found
}

fn text(document: &Document, node: NodeId) -> String {
    document
        .children(node)
        .filter_map(|child| document.text(child))
        .collect()
}

#[test]
fn cdata_style_entities_and_namespace_scopes() {
    let source = r##"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.1//EN" "http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd">
<html xmlns="http://www.w3.org/1999/xhtml"><head><style type="text/css"><![CDATA[
p > b { color: green }
]]></style></head>
<body><p>a&nbsp;b&#x41;&lt;</p>
<s:svg xmlns:s="http://www.w3.org/2000/svg" xmlns:l="http://www.w3.org/1999/xlink"><s:a l:href="#x"><s:foreignObject><div/></s:foreignObject></s:a></s:svg>
<e xmlns="urn:other"><f xmlns=""/></e><div/></body></html>"##;
    let document = parse_xml(Document::new(), source.as_bytes(), None);
    let found = elements(&document);
    let names: Vec<(Namespace, &str)> = found
        .iter()
        .map(|(_, ns, name)| (*ns, name.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            (Namespace::Html, "html"),
            (Namespace::Html, "head"),
            (Namespace::Html, "style"),
            (Namespace::Html, "body"),
            (Namespace::Html, "p"),
            (Namespace::Svg, "svg"),
            (Namespace::Svg, "a"),
            (Namespace::Svg, "foreignObject"),
            (Namespace::Html, "div"),
            (Namespace::Other, "e"),
            (Namespace::None, "f"),
            (Namespace::Html, "div"),
        ]
    );
    assert_eq!(text(&document, found[2].0), "\np > b { color: green }\n");
    assert_eq!(text(&document, found[4].0), "a\u{a0}bA<");
    let href = QualName {
        ns: Namespace::XLink,
        local: Atom::HREF,
    };
    assert_eq!(document.qualified_attribute(found[6].0, href), Some("#x"));
}

#[test]
fn malformed_documents_become_an_error_document() {
    for source in [
        "<a><b></a>",
        "<a>&nbsp;</a>",
        "<p:a/>",
        "<a x='1' x='2'/>",
        "<a/><b/>",
        "<a><!-- -- --></a>",
        "",
    ] {
        let document = parse_xml(Document::new(), source.as_bytes(), None);
        let names: Vec<String> = elements(&document)
            .into_iter()
            .map(|(_, _, name)| name)
            .collect();
        assert_eq!(names, ["parsererror"], "{source}");
    }
}
