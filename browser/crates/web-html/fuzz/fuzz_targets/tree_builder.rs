#![no_main]

use libfuzzer_sys::fuzz_target;
use web_dom::Document;
use web_html::{Atom, Namespace, Options, Parser, QualName};

const CONTEXTS: [(Namespace, Atom); 8] = [
    (Namespace::Html, Atom::BODY),
    (Namespace::Html, Atom::TABLE),
    (Namespace::Html, Atom::TEMPLATE),
    (Namespace::Html, Atom::SELECT),
    (Namespace::Html, Atom::TEXTAREA),
    (Namespace::Html, Atom::TR),
    (Namespace::Svg, Atom::SVG),
    (Namespace::MathMl, Atom::ANNOTATION_XML),
];

fuzz_target!(|data: &[u8]| {
    let Some((&selector, rest)) = data.split_first() else {
        return;
    };
    let options = Options {
        scripting: selector & 1 == 1,
        transport_encoding: None,
    };
    let chunk = usize::from(selector >> 4) + 1;
    if selector & 2 == 0 {
        let mut parser = Parser::new(Document::new(), options);
        for piece in rest.chunks(chunk) {
            parser.feed(piece);
        }
        parser.finish();
    } else {
        let (ns, local) = CONTEXTS[usize::from(selector >> 2) % CONTEXTS.len()];
        let text = String::from_utf8_lossy(rest);
        let mut parser = Parser::new_fragment(Document::new(), QualName { ns, local }, options);
        for piece in text.as_bytes().chunks(chunk) {
            parser.feed_str(&String::from_utf8_lossy(piece));
        }
        parser.finish();
    }
});
